use std::{
    collections::{BTreeMap, VecDeque},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::{Mutex, RwLock, broadcast},
    time::{sleep, timeout},
};

use crate::{
    error::{StudioError, StudioResult},
    realtime::{EventHub, StudioEvent},
    registry::{RegisteredMcp, Registry},
    reliability::{RestartEpisode, RestartPolicy},
    storage::{HistoryHandle, LifecycleEndKind, LifecycleOwnerKind, LifecycleSessionContext},
    update::RuntimeOperationCoordinator,
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
    pub desired_running: bool,
    pub restart_state: &'static str,
    pub consecutive_restart_failures: u32,
    pub retry_at_ms: Option<u64>,
    pub last_exit_code: Option<i32>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    pub sequence: u64,
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
    next_log_sequence: u64,
    generation: u64,
    history_session: Option<LifecycleSessionContext>,
    desired_running: bool,
    restart_episode: RestartEpisode,
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
            next_log_sequence: 1,
            generation: 0,
            history_session: None,
            desired_running: false,
            restart_episode: RestartEpisode::default(),
        }
    }
}

impl RuntimeState {
    fn status(&self, id: &str, name: &str) -> ProcessStatus {
        ProcessStatus {
            id: id.to_owned(),
            name: name.to_owned(),
            state: self.state,
            pid: self.pid,
            uptime_ms: self
                .started_at
                .map(|started| u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
            restart_count: self.restart_count,
            crash_count: self.crash_count,
            desired_running: self.desired_running,
            restart_state: self.restart_episode.phase.as_str(),
            consecutive_restart_failures: self.restart_episode.consecutive_failures,
            retry_at_ms: self.restart_episode.retry_at_ms,
            last_exit_code: self.last_exit_code,
            last_error: self.last_error.clone(),
        }
    }

    fn push_log(&mut self, capacity: usize, stream: LogStream, message: String) -> LogEntry {
        while self.logs.len() >= capacity {
            self.logs.pop_front();
        }
        let entry = LogEntry {
            sequence: self.next_log_sequence,
            timestamp_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
            stream,
            message,
        };
        self.next_log_sequence = self.next_log_sequence.saturating_add(1);
        self.logs.push_back(entry.clone());
        entry
    }
}

#[derive(Clone)]
pub struct Supervisor {
    registry: Registry,
    runtimes: Arc<RwLock<BTreeMap<String, Arc<Mutex<RuntimeState>>>>>,
    events: EventHub,
    log_capacity: usize,
    stop_timeout: Duration,
    history: Option<HistoryHandle>,
    restart_policy: RestartPolicy,
    runtime_operations: Option<Arc<RuntimeOperationCoordinator>>,
}

impl Supervisor {
    pub fn new(registry: Registry, log_capacity: usize, stop_timeout: Duration) -> Self {
        Self::new_internal(
            registry,
            log_capacity,
            stop_timeout,
            None,
            RestartPolicy::disabled(),
            None,
        )
    }

    pub fn new_with_history(
        registry: Registry,
        log_capacity: usize,
        stop_timeout: Duration,
        history: HistoryHandle,
    ) -> Self {
        Self::new_internal(
            registry,
            log_capacity,
            stop_timeout,
            Some(history),
            RestartPolicy::disabled(),
            None,
        )
    }

    pub fn new_with_history_and_restart(
        registry: Registry,
        log_capacity: usize,
        stop_timeout: Duration,
        history: HistoryHandle,
        restart_policy: RestartPolicy,
        runtime_operations: Arc<RuntimeOperationCoordinator>,
    ) -> Self {
        Self::new_internal(
            registry,
            log_capacity,
            stop_timeout,
            Some(history),
            restart_policy,
            Some(runtime_operations),
        )
    }

    fn new_internal(
        registry: Registry,
        log_capacity: usize,
        stop_timeout: Duration,
        history: Option<HistoryHandle>,
        restart_policy: RestartPolicy,
        runtime_operations: Option<Arc<RuntimeOperationCoordinator>>,
    ) -> Self {
        Self {
            registry,
            runtimes: Arc::new(RwLock::new(BTreeMap::new())),
            events: EventHub::default(),
            log_capacity,
            stop_timeout,
            history,
            restart_policy,
            runtime_operations,
        }
    }

    pub fn registry(&self) -> &Registry {
        &self.registry
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<StudioEvent> {
        self.events.subscribe()
    }

    pub async fn list(&self) -> Vec<ProcessStatus> {
        let ids = self.registry.ids();
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
        let runtime = self.runtime_or_create(id).await;
        let state = runtime.lock().await;
        Ok(state.status(id, &config.name))
    }

    pub async fn is_active(&self, id: &str) -> StudioResult<bool> {
        let status = self.status(id).await?;
        Ok(matches!(
            status.state,
            ProcessState::Starting | ProcessState::Running | ProcessState::Stopping
        ))
    }

    pub async fn remove_inactive_runtime(&self, id: &str) -> StudioResult<()> {
        let runtime = self.runtimes.read().await.get(id).cloned();
        if let Some(runtime) = runtime {
            let state = runtime.lock().await;
            if matches!(
                state.state,
                ProcessState::Starting | ProcessState::Running | ProcessState::Stopping
            ) {
                return Err(StudioError::Conflict(format!(
                    "{id} must be stopped before unregistering"
                )));
            }
            drop(state);
            self.runtimes.write().await.remove(id);
        }
        Ok(())
    }

    pub async fn logs(&self, id: &str) -> StudioResult<Vec<LogEntry>> {
        self.registry
            .get(id)
            .ok_or_else(|| StudioError::NotFound(id.to_owned()))?;
        let runtime = self.runtime_or_create(id).await;
        let state = runtime.lock().await;
        Ok(state.logs.iter().cloned().collect())
    }

    pub async fn start(&self, id: &str) -> StudioResult<ProcessStatus> {
        let (config, working_dir, command_path) = self.registry.validate_for_spawn(id)?;
        let runtime = self.runtime_or_create(id).await;

        let (generation, starting_status) = {
            let mut state = runtime.lock().await;
            match state.state {
                ProcessState::Stopped | ProcessState::Failed => {}
                _ => return Err(StudioError::AlreadyRunning(id.to_owned())),
            }
            state.desired_running = true;
            state.state = ProcessState::Starting;
            state.last_error = None;
            state.generation = state.generation.wrapping_add(1);
            (state.generation, state.status(id, &config.name))
        };
        self.publish_status(id, starting_status);

        match self
            .spawn_process(
                id,
                &config,
                &working_dir,
                &command_path,
                runtime.clone(),
                generation,
            )
            .await
        {
            Ok(()) => self.status(id).await,
            Err(error) => {
                let (failed_status, log_entry) = {
                    let mut state = runtime.lock().await;
                    if state.generation == generation {
                        state.state = ProcessState::Failed;
                        state.pid = None;
                        state.started_at = None;
                        state.last_error = Some(error.to_string());
                    }
                    let entry = state.push_log(
                        self.log_capacity,
                        LogStream::Studio,
                        format!("failed to start: {error}"),
                    );
                    (state.status(id, &config.name), entry)
                };
                self.publish_log(id, log_entry);
                self.publish_status(id, failed_status);
                if let Some(history) = &self.history {
                    history.observe_start_failed(LifecycleOwnerKind::Mcp, generation);
                }
                Err(error)
            }
        }
    }

    pub async fn stop(&self, id: &str) -> StudioResult<ProcessStatus> {
        let config = self
            .registry
            .get(id)
            .ok_or_else(|| StudioError::NotFound(id.to_owned()))?;
        let runtime = self.runtime_or_create(id).await;

        let (pid, stopping_status, log_entry) = {
            let mut state = runtime.lock().await;
            state.desired_running = false;
            state.restart_episode.clear();
            match state.state {
                ProcessState::Running | ProcessState::Starting => {
                    let Some(pid) = state.pid else {
                        return Err(StudioError::Process(format!(
                            "{id}: running state has no PID"
                        )));
                    };
                    state.state = ProcessState::Stopping;
                    let entry = state.push_log(
                        self.log_capacity,
                        LogStream::Studio,
                        "stop requested".to_owned(),
                    );
                    (pid, state.status(id, &config.name), entry)
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
        self.publish_log(id, log_entry);
        self.publish_status(id, stopping_status);

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
        if self.registry.get(id).is_some_and(|config| !config.enabled) {
            return Err(StudioError::Disabled(id.to_owned()));
        }
        let status = self.status(id).await?;
        if matches!(status.state, ProcessState::Running | ProcessState::Starting) {
            self.stop(id).await?;
        } else if status.state == ProcessState::Stopping {
            self.wait_for_terminal_state(id).await?;
        }

        let runtime = self.runtime_or_create(id).await;
        {
            let mut state = runtime.lock().await;
            state.restart_count = state.restart_count.saturating_add(1);
        }
        self.start(id).await
    }

    pub async fn shutdown_all(&self) {
        let ids = self.registry.ids();
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

    fn publish_status(&self, id: &str, status: ProcessStatus) {
        self.events.publish(StudioEvent::ProcessStatus {
            mcp_id: id.to_owned(),
            status,
        });
    }

    fn publish_log(&self, id: &str, entry: LogEntry) {
        self.events.publish(StudioEvent::Log {
            mcp_id: id.to_owned(),
            entry,
        });
    }

    async fn runtime_or_create(&self, id: &str) -> Arc<Mutex<RuntimeState>> {
        if let Some(runtime) = self.runtimes.read().await.get(id).cloned() {
            return runtime;
        }
        let mut runtimes = self.runtimes.write().await;
        runtimes
            .entry(id.to_owned())
            .or_insert_with(|| Arc::new(Mutex::new(RuntimeState::default())))
            .clone()
    }

    async fn spawn_process(
        &self,
        id: &str,
        config: &RegisteredMcp,
        working_dir: &std::path::Path,
        command_path: &std::path::Path,
        runtime: Arc<Mutex<RuntimeState>>,
        generation: u64,
    ) -> StudioResult<()> {
        let mut command = Command::new(command_path);
        command
            .args(&config.args)
            .current_dir(working_dir)
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
        let history_session = self.history.as_ref().and_then(|history| {
            history.observe_session_started(LifecycleOwnerKind::Mcp, generation, pid)
        });

        let (running_status, started_log) = {
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
            state.restart_episode.on_started();
            state.history_session = history_session.clone();
            let entry = state.push_log(
                self.log_capacity,
                LogStream::Studio,
                format!("started PID {pid}"),
            );
            (state.status(id, &config.name), entry)
        };
        self.publish_log(id, started_log);
        self.publish_status(id, running_status);

        if let (Some(history), Some(context)) = (self.history.clone(), history_session) {
            let heartbeat_runtime = runtime.clone();
            tokio::spawn(async move {
                loop {
                    sleep(Duration::from_secs(15)).await;
                    let active = {
                        let state = heartbeat_runtime.lock().await;
                        state.generation == generation
                            && matches!(state.state, ProcessState::Running | ProcessState::Stopping)
                    };
                    if !active {
                        break;
                    }
                    history.observe_session_heartbeat(&context);
                }
            });
        }

        if let Some(stdout) = stdout {
            spawn_log_reader(
                id.to_owned(),
                runtime.clone(),
                stdout,
                LogStream::Stdout,
                self.log_capacity,
                self.events.clone(),
            );
        }
        if let Some(stderr) = stderr {
            spawn_log_reader(
                id.to_owned(),
                runtime.clone(),
                stderr,
                LogStream::Stderr,
                self.log_capacity,
                self.events.clone(),
            );
        }

        let id_owned = id.to_owned();
        let name_owned = config.name.clone();
        let capacity = self.log_capacity;
        let events = self.events.clone();
        let history = self.history.clone();
        let supervisor = self.clone();
        tokio::spawn(async move {
            let _stdin_guard = stdin;
            let result = child.wait().await;
            let (status, log_entry, terminal, retry_at_ms) = {
                let mut state = runtime.lock().await;
                if state.generation != generation {
                    return;
                }

                let was_stopping = state.state == ProcessState::Stopping;
                let exact_duration = state.started_at.map(|started| started.elapsed());
                let history_session = state.history_session.take();
                state.pid = None;
                state.started_at = None;

                let (message, end_kind, exit_code, is_crash) = match result {
                    Ok(exit) => {
                        let exit_code = exit.code();
                        state.last_exit_code = exit_code;
                        if was_stopping {
                            state.state = ProcessState::Stopped;
                            state.last_error = None;
                            (
                                format!("process exited: {exit}"),
                                LifecycleEndKind::RequestedStop,
                                exit_code,
                                false,
                            )
                        } else if exit.success() {
                            state.state = ProcessState::Stopped;
                            state.desired_running = false;
                            state.restart_episode.clear();
                            state.last_error = None;
                            (
                                format!("process exited: {exit}"),
                                LifecycleEndKind::CleanExit,
                                exit_code,
                                false,
                            )
                        } else {
                            state.state = ProcessState::Failed;
                            state.crash_count = state.crash_count.saturating_add(1);
                            state.last_error = Some(format!("process exited with status {exit}"));
                            (
                                format!("process exited: {exit}"),
                                LifecycleEndKind::UnexpectedExit,
                                exit_code,
                                true,
                            )
                        }
                    }
                    Err(error) => {
                        state.state = ProcessState::Failed;
                        state.crash_count = state.crash_count.saturating_add(1);
                        state.last_error = Some(format!("failed waiting for process: {error}"));
                        (
                            format!("wait failed: {error}"),
                            LifecycleEndKind::WaitError,
                            None,
                            true,
                        )
                    }
                };
                let retry_at_ms = if is_crash && state.desired_running {
                    state.restart_episode.record_failure(
                        now_ms(),
                        exact_duration.map(|duration| {
                            u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
                        }),
                        supervisor.restart_policy,
                    )
                } else {
                    None
                };
                let entry = state.push_log(capacity, LogStream::Studio, message);
                (
                    state.status(&id_owned, &name_owned),
                    entry,
                    history_session
                        .map(|context| (context, end_kind, exit_code, is_crash, exact_duration)),
                    retry_at_ms,
                )
            };
            if let (Some(history), Some((context, end_kind, exit_code, is_crash, duration))) =
                (history.as_ref(), terminal)
            {
                history.observe_session_terminal(context, end_kind, exit_code, is_crash, duration);
            }
            events.publish(StudioEvent::Log {
                mcp_id: id_owned.clone(),
                entry: log_entry,
            });
            events.publish(StudioEvent::ProcessStatus {
                mcp_id: id_owned.clone(),
                status: status.clone(),
            });
            tracing::info!(mcp_id = id_owned, state = ?status.state, "MCP process exited");
            let _ = retry_at_ms;
        });

        Ok(())
    }

    pub async fn next_restart_due_ms(&self) -> Option<u64> {
        let runtimes = self.runtimes.read().await;
        let values = runtimes.values().cloned().collect::<Vec<_>>();
        drop(runtimes);
        let mut next = None;
        for runtime in values {
            let state = runtime.lock().await;
            if state.desired_running
                && state.state == ProcessState::Failed
                && let Some(retry) = state.restart_episode.retry_at_ms
            {
                next = Some(next.map_or(retry, |current: u64| current.min(retry)));
            }
        }
        next
    }

    pub async fn process_due_restarts(&self, now_ms: u64) {
        if !self.restart_policy.enabled {
            return;
        }
        let busy = self.runtime_operations.as_ref().is_some_and(|coordinator| {
            coordinator
                .snapshot()
                .map(|snapshot| !snapshot.is_idle())
                .unwrap_or(true)
        });

        for id in self.registry.ids() {
            let runtime = self.runtime_or_create(&id).await;
            let due = {
                let mut state = runtime.lock().await;
                if !state.desired_running
                    || state.state != ProcessState::Failed
                    || !state.restart_episode.ready(now_ms)
                {
                    false
                } else if busy {
                    state
                        .restart_episode
                        .defer_without_failure(now_ms, self.restart_policy);
                    false
                } else {
                    state.restart_count = state.restart_count.saturating_add(1);
                    true
                }
            };
            if !due {
                continue;
            }

            if let Err(error) = self.start(&id).await {
                tracing::warn!(mcp_id = %id, %error, "automatic MCP restart attempt failed");
                let runtime = self.runtime_or_create(&id).await;
                let mut state = runtime.lock().await;
                if state.desired_running {
                    state
                        .restart_episode
                        .record_failure(now_ms, Some(0), self.restart_policy);
                }
            }
        }
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

fn now_ms() -> u64 {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    u64::try_from(millis).unwrap_or(u64::MAX)
}

fn strip_ansi(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = String::with_capacity(input.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == 0x1b && index + 1 < bytes.len() {
            match bytes[index + 1] {
                b'[' => {
                    index += 2;
                    while index < bytes.len() {
                        let byte = bytes[index];
                        index += 1;
                        if (0x40..=0x7e).contains(&byte) {
                            break;
                        }
                    }
                    continue;
                }
                b']' => {
                    index += 2;
                    while index < bytes.len() {
                        if bytes[index] == 0x07 {
                            index += 1;
                            break;
                        }
                        if bytes[index] == 0x1b
                            && index + 1 < bytes.len()
                            && bytes[index + 1] == b'\\'
                        {
                            index += 2;
                            break;
                        }
                        index += 1;
                    }
                    continue;
                }
                _ => {
                    index += 2;
                    continue;
                }
            }
        }

        let start = index;
        while index < bytes.len() && bytes[index] != 0x1b {
            index += 1;
        }
        output.push_str(&input[start..index]);
    }

    output
}

fn spawn_log_reader<R>(
    id: String,
    runtime: Arc<Mutex<RuntimeState>>,
    reader: R,
    stream: LogStream,
    capacity: usize,
    events: EventHub,
) where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    tokio::spawn(async move {
        let mut lines = BufReader::new(reader).lines();
        loop {
            match lines.next_line().await {
                Ok(Some(line)) => {
                    let entry = {
                        let mut state = runtime.lock().await;
                        state.push_log(capacity, stream, strip_ansi(&line))
                    };
                    events.publish(StudioEvent::Log {
                        mcp_id: id.clone(),
                        entry,
                    });
                }
                Ok(None) => break,
                Err(error) => {
                    let entry = {
                        let mut state = runtime.lock().await;
                        state.push_log(
                            capacity,
                            LogStream::Studio,
                            format!("log reader error: {error}"),
                        )
                    };
                    events.publish(StudioEvent::Log {
                        mcp_id: id.clone(),
                        entry,
                    });
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

#[cfg(test)]
mod tests {
    use super::strip_ansi;

    #[test]
    fn strips_ansi_color_and_style_sequences() {
        let input = "\u{1b}[2m2026-09-16T11:22:29Z\u{1b}[0m \u{1b}[32m INFO\u{1b}[0m \u{1b}[2mrust_mcp_filesystem\u{1b}[0m: starting";
        assert_eq!(
            strip_ansi(input),
            "2026-09-16T11:22:29Z  INFO rust_mcp_filesystem: starting"
        );
    }

    #[test]
    fn preserves_plain_text() {
        assert_eq!(strip_ansi("plain text"), "plain text");
    }
}
