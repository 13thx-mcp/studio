use std::{
    collections::{BTreeMap, VecDeque},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::{Mutex, RwLock},
    time::{sleep, timeout},
};

use crate::{
    config::McpServerConfig,
    error::{StudioError, StudioResult},
    registry::Registry,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessStatus {
    pub id: String,
    pub name: String,
    pub state: ProcessState,
    pub pid: Option<u32>,
    pub uptime_ms: Option<u64>,
    pub restart_count: u64,
    pub crash_count: u64,
    pub last_exit_code: Option<i32>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub timestamp_ms: u128,
    pub stream: LogStream,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LogStream {
    Stdout,
    Stderr,
    Studio,
}

#[derive(Debug)]
struct RuntimeState {
    state: ProcessState,
    pid: Option<u32>,
    started_at: Option<Instant>,
    restart_count: u64,
    crash_count: u64,
    last_exit_code: Option<i32>,
    last_error: Option<String>,
    logs: VecDeque<LogEntry>,
    generation: u64,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            state: ProcessState::Stopped,
            pid: None,
            started_at: None,
            restart_count: 0,
            crash_count: 0,
            last_exit_code: None,
            last_error: None,
            logs: VecDeque::new(),
            generation: 0,
        }
    }
}

#[derive(Clone)]
pub struct Supervisor {
    registry: Arc<Registry>,
    runtimes: Arc<RwLock<BTreeMap<String, Arc<Mutex<RuntimeState>>>>>,
    log_capacity: usize,
    stop_timeout: Duration,
    base_dir: Arc<PathBuf>,
}

impl Supervisor {
    pub fn new(
        registry: Registry,
        log_capacity: usize,
        stop_timeout: Duration,
        base_dir: PathBuf,
    ) -> Self {
        let runtimes = registry
            .ids()
            .map(|id| (id.clone(), Arc::new(Mutex::new(RuntimeState::default()))))
            .collect();

        Self {
            registry: Arc::new(registry),
            runtimes: Arc::new(RwLock::new(runtimes)),
            log_capacity,
            stop_timeout,
            base_dir: Arc::new(base_dir),
        }
    }

    pub async fn list(&self) -> Vec<ProcessStatus> {
        let ids: Vec<String> = self.registry.ids().cloned().collect();
        let mut result = Vec::with_capacity(ids.len());
        for id in ids {
            if let Ok(status) = self.status(&id).await {
                result.push(status);
            }
        }
        result
    }

    pub async fn status(&self, id: &str) -> StudioResult<ProcessStatus> {
        let config = self
            .registry
            .get(id)
            .ok_or_else(|| StudioError::NotFound(id.to_owned()))?;
        let runtime = self.runtime(id).await?;
        let state = runtime.lock().await;
        Ok(ProcessStatus {
            id: id.to_owned(),
            name: config.name.clone(),
            state: state.state,
            pid: state.pid,
            uptime_ms: state
                .started_at
                .map(|started| u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
            restart_count: state.restart_count,
            crash_count: state.crash_count,
            last_exit_code: state.last_exit_code,
            last_error: state.last_error.clone(),
        })
    }

    pub async fn logs(&self, id: &str) -> StudioResult<Vec<LogEntry>> {
        self.registry
            .get(id)
            .ok_or_else(|| StudioError::NotFound(id.to_owned()))?;
        let runtime = self.runtime(id).await?;
        let state = runtime.lock().await;
        Ok(state.logs.iter().cloned().collect())
    }

    pub async fn start(&self, id: &str) -> StudioResult<ProcessStatus> {
        let config = self
            .registry
            .get(id)
            .cloned()
            .ok_or_else(|| StudioError::NotFound(id.to_owned()))?;
        let runtime = self.runtime(id).await?;

        let generation = {
            let mut state = runtime.lock().await;
            match state.state {
                ProcessState::Stopped | ProcessState::Failed => {}
                _ => return Err(StudioError::AlreadyRunning(id.to_owned())),
            }
            state.state = ProcessState::Starting;
            state.last_error = None;
            state.generation = state.generation.wrapping_add(1);
            state.generation
        };

        match self
            .spawn_process(id, &config, runtime.clone(), generation)
            .await
        {
            Ok(()) => self.status(id).await,
            Err(error) => {
                let mut state = runtime.lock().await;
                if state.generation == generation {
                    state.state = ProcessState::Failed;
                    state.pid = None;
                    state.started_at = None;
                    state.last_error = Some(error.to_string());
                    push_log(
                        &mut state.logs,
                        self.log_capacity,
                        LogStream::Studio,
                        format!("failed to start: {error}"),
                    );
                }
                Err(error)
            }
        }
    }

    pub async fn stop(&self, id: &str) -> StudioResult<ProcessStatus> {
        self.registry
            .get(id)
            .ok_or_else(|| StudioError::NotFound(id.to_owned()))?;
        let runtime = self.runtime(id).await?;

        let pid = {
            let mut state = runtime.lock().await;
            match state.state {
                ProcessState::Running | ProcessState::Starting => {
                    let Some(pid) = state.pid else {
                        return Err(StudioError::Process(format!(
                            "{id}: running state has no PID"
                        )));
                    };
                    state.state = ProcessState::Stopping;
                    push_log(
                        &mut state.logs,
                        self.log_capacity,
                        LogStream::Studio,
                        "stop requested".to_owned(),
                    );
                    pid
                }
                ProcessState::Stopping => {
                    return Err(StudioError::Process(format!(
                        "{id}: stop already in progress"
                    )));
                }
                ProcessState::Stopped | ProcessState::Failed => {
                    return Err(StudioError::NotRunning(id.to_owned()));
                }
            }
        };

        send_terminate(pid)?;

        if timeout(self.stop_timeout, self.wait_for_terminal_state(id))
            .await
            .is_err()
        {
            tracing::warn!(mcp_id = id, pid, "graceful stop timed out; sending kill");
            send_kill(pid)?;
            timeout(self.stop_timeout, self.wait_for_terminal_state(id))
                .await
                .map_err(|_| {
                    StudioError::Process(format!("{id}: process did not exit after kill"))
                })??;
        }

        self.status(id).await
    }

    pub async fn restart(&self, id: &str) -> StudioResult<ProcessStatus> {
        let status = self.status(id).await?;
        if matches!(status.state, ProcessState::Running | ProcessState::Starting) {
            self.stop(id).await?;
        } else if status.state == ProcessState::Stopping {
            self.wait_for_terminal_state(id).await?;
        }

        let runtime = self.runtime(id).await?;
        {
            let mut state = runtime.lock().await;
            state.restart_count = state.restart_count.saturating_add(1);
        }
        self.start(id).await
    }

    pub async fn shutdown_all(&self) {
        let ids: Vec<String> = self.registry.ids().cloned().collect();
        for id in ids {
            let Ok(status) = self.status(&id).await else {
                continue;
            };
            if matches!(
                status.state,
                ProcessState::Running | ProcessState::Starting | ProcessState::Stopping
            ) && let Err(error) = self.stop(&id).await
            {
                tracing::error!(mcp_id = id, %error, "failed to stop MCP during Studio shutdown");
            }
        }
    }

    async fn runtime(&self, id: &str) -> StudioResult<Arc<Mutex<RuntimeState>>> {
        self.runtimes
            .read()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| StudioError::NotFound(id.to_owned()))
    }

    async fn spawn_process(
        &self,
        id: &str,
        config: &McpServerConfig,
        runtime: Arc<Mutex<RuntimeState>>,
        generation: u64,
    ) -> StudioResult<()> {
        let working_dir = resolve_path(&self.base_dir, &config.working_dir);
        let command_path = resolve_path(&self.base_dir, &config.command);

        if !working_dir.is_dir() {
            return Err(StudioError::Process(format!(
                "{id}: working directory does not exist: {}",
                working_dir.display()
            )));
        }
        if !command_path.is_file() {
            return Err(StudioError::Process(format!(
                "{id}: executable does not exist: {}",
                command_path.display()
            )));
        }

        let mut command = Command::new(&command_path);
        command
            .args(&config.args)
            .current_dir(&working_dir)
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = command.spawn().map_err(|error| {
            StudioError::Process(format!(
                "{id}: failed to spawn {}: {error}",
                command_path.display()
            ))
        })?;
        let pid = child.id().ok_or_else(|| {
            StudioError::Process(format!("{id}: spawned process did not report a PID"))
        })?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        {
            let mut state = runtime.lock().await;
            if state.generation != generation {
                return Err(StudioError::Process(format!(
                    "{id}: process generation changed during startup"
                )));
            }
            state.state = ProcessState::Running;
            state.pid = Some(pid);
            state.started_at = Some(Instant::now());
            state.last_exit_code = None;
            state.last_error = None;
            push_log(
                &mut state.logs,
                self.log_capacity,
                LogStream::Studio,
                format!("started PID {pid}"),
            );
        }

        if let Some(stdout) = stdout {
            spawn_log_reader(
                runtime.clone(),
                stdout,
                LogStream::Stdout,
                self.log_capacity,
            );
        }
        if let Some(stderr) = stderr {
            spawn_log_reader(
                runtime.clone(),
                stderr,
                LogStream::Stderr,
                self.log_capacity,
            );
        }

        let id_owned = id.to_owned();
        let capacity = self.log_capacity;
        tokio::spawn(async move {
            // Keep the child's stdin pipe alive while waiting for the process.
            // Tokio's Child::wait closes an owned stdin handle before waiting;
            // taking it out and retaining this guard prevents stdio MCP servers
            // from observing an immediate EOF before an MCP client connects.
            let _stdin_guard = stdin;
            let result = child.wait().await;
            let mut state = runtime.lock().await;
            if state.generation != generation {
                return;
            }

            let was_stopping = state.state == ProcessState::Stopping;
            state.pid = None;
            state.started_at = None;

            match result {
                Ok(exit) => {
                    state.last_exit_code = exit.code();
                    if was_stopping || exit.success() {
                        state.state = ProcessState::Stopped;
                        state.last_error = None;
                    } else {
                        state.state = ProcessState::Failed;
                        state.crash_count = state.crash_count.saturating_add(1);
                        state.last_error = Some(format!("process exited with status {exit}"));
                    }
                    push_log(
                        &mut state.logs,
                        capacity,
                        LogStream::Studio,
                        format!("process exited: {exit}"),
                    );
                }
                Err(error) => {
                    state.state = ProcessState::Failed;
                    state.crash_count = state.crash_count.saturating_add(1);
                    state.last_error = Some(format!("failed waiting for process: {error}"));
                    push_log(
                        &mut state.logs,
                        capacity,
                        LogStream::Studio,
                        format!("wait failed: {error}"),
                    );
                }
            }
            tracing::info!(mcp_id = id_owned, state = ?state.state, "MCP process exited");
        });

        Ok(())
    }

    async fn wait_for_terminal_state(&self, id: &str) -> StudioResult<()> {
        loop {
            let status = self.status(id).await?;
            if matches!(status.state, ProcessState::Stopped | ProcessState::Failed) {
                return Ok(());
            }
            sleep(Duration::from_millis(25)).await;
        }
    }
}

fn resolve_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        base_dir.join(path)
    }
}

fn push_log(logs: &mut VecDeque<LogEntry>, capacity: usize, stream: LogStream, message: String) {
    while logs.len() >= capacity {
        logs.pop_front();
    }
    logs.push_back(LogEntry {
        timestamp_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
        stream,
        message,
    });
}

fn spawn_log_reader<R>(
    runtime: Arc<Mutex<RuntimeState>>,
    reader: R,
    stream: LogStream,
    capacity: usize,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let mut state = runtime.lock().await;
                    push_log(&mut state.logs, capacity, stream, line);
                }
                Ok(None) => break,
                Err(error) => {
                    let mut state = runtime.lock().await;
                    push_log(
                        &mut state.logs,
                        capacity,
                        LogStream::Studio,
                        format!("log reader error: {error}"),
                    );
                    break;
                }
            }
        }
    });
}

#[cfg(unix)]
fn send_terminate(pid: u32) -> StudioResult<()> {
    use nix::{
        sys::signal::{Signal, kill},
        unistd::Pid,
    };
    kill(Pid::from_raw(pid as i32), Signal::SIGTERM)
        .map_err(|error| StudioError::Process(format!("failed to terminate PID {pid}: {error}")))
}

#[cfg(unix)]
fn send_kill(pid: u32) -> StudioResult<()> {
    use nix::{
        sys::signal::{Signal, kill},
        unistd::Pid,
    };
    kill(Pid::from_raw(pid as i32), Signal::SIGKILL)
        .map_err(|error| StudioError::Process(format!("failed to kill PID {pid}: {error}")))
}

#[cfg(not(unix))]
fn send_terminate(_pid: u32) -> StudioResult<()> {
    Err(StudioError::Process(
        "Milestone 1 graceful process control currently requires a Unix platform".into(),
    ))
}

#[cfg(not(unix))]
fn send_kill(_pid: u32) -> StudioResult<()> {
    Err(StudioError::Process(
        "Milestone 1 force-kill currently requires a Unix platform".into(),
    ))
}
