use std::{
    collections::{BTreeMap, VecDeque},
    fs,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::Command,
    sync::{broadcast, Mutex},
    time::{sleep, timeout},
};

use crate::{
    error::{StudioError, StudioResult},
    realtime::{EventHub, StudioEvent},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TunnelConfig {
    #[serde(default = "default_name")]
    pub name: String,
    pub runtime: PathBuf,
    pub working_dir: PathBuf,
    pub config_file: PathBuf,
    #[serde(default)]
    pub env: BTreeMap<String, SecretReference>,
}

impl Default for TunnelConfig {
    fn default() -> Self {
        Self {
            name: default_name(),
            runtime: PathBuf::from("../tunnel-client/tunnel-client-runtime-cloudflared"),
            working_dir: PathBuf::from("../tunnel-client"),
            config_file: PathBuf::from("../tunnel-client/config.yaml"),
            env: BTreeMap::new(),
        }
    }
}

fn default_name() -> String {
    "Secure tunnel".to_owned()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SecretReference {
    FromEnv { from_env: String },
    FromFile { from_file: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelState {
    Stopped,
    Starting,
    Running,
    Stopping,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
pub struct TunnelStatus {
    pub name: String,
    pub state: TunnelState,
    pub runtime_available: bool,
    pub pid: Option<u32>,
    pub uptime_ms: Option<u64>,
    pub restart_count: u64,
    pub crash_count: u64,
    pub last_exit_code: Option<i32>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TunnelLogEntry {
    pub sequence: u64,
    pub timestamp_ms: u128,
    pub stream: TunnelLogStream,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TunnelLogStream {
    Stdout,
    Stderr,
    Studio,
}

#[derive(Debug)]
struct RuntimeState {
    state: TunnelState,
    pid: Option<u32>,
    started_at: Option<Instant>,
    restart_count: u64,
    crash_count: u64,
    last_exit_code: Option<i32>,
    last_error: Option<String>,
    logs: VecDeque<TunnelLogEntry>,
    next_log_sequence: u64,
    generation: u64,
}

impl Default for RuntimeState {
    fn default() -> Self {
        Self {
            state: TunnelState::Stopped,
            pid: None,
            started_at: None,
            restart_count: 0,
            crash_count: 0,
            last_exit_code: None,
            last_error: None,
            logs: VecDeque::new(),
            next_log_sequence: 1,
            generation: 0,
        }
    }
}

impl RuntimeState {
    fn status(&self, name: &str, runtime_available: bool) -> TunnelStatus {
        TunnelStatus {
            name: name.to_owned(),
            state: self.state,
            runtime_available,
            pid: self.pid,
            uptime_ms: self
                .started_at
                .map(|started| u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)),
            restart_count: self.restart_count,
            crash_count: self.crash_count,
            last_exit_code: self.last_exit_code,
            last_error: self.last_error.clone(),
        }
    }

    fn push_log(
        &mut self,
        capacity: usize,
        stream: TunnelLogStream,
        message: String,
    ) -> TunnelLogEntry {
        while self.logs.len() >= capacity {
            self.logs.pop_front();
        }
        let entry = TunnelLogEntry {
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
pub struct TunnelSupervisor {
    config: Arc<TunnelConfig>,
    runtime: Arc<Mutex<RuntimeState>>,
    events: EventHub,
    log_capacity: usize,
    stop_timeout: Duration,
    base_dir: Arc<PathBuf>,
}

impl TunnelSupervisor {
    pub fn new(
        config: TunnelConfig,
        log_capacity: usize,
        stop_timeout: Duration,
        base_dir: PathBuf,
        events: EventHub,
    ) -> Self {
        Self {
            config: Arc::new(config),
            runtime: Arc::new(Mutex::new(RuntimeState::default())),
            events,
            log_capacity,
            stop_timeout,
            base_dir: Arc::new(base_dir),
        }
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<StudioEvent> {
        self.events.subscribe()
    }

    pub async fn status(&self) -> TunnelStatus {
        let available = self.validated_paths().is_ok();
        self.runtime
            .lock()
            .await
            .status(&self.config.name, available)
    }

    pub async fn logs(&self) -> Vec<TunnelLogEntry> {
        self.runtime.lock().await.logs.iter().cloned().collect()
    }

    pub async fn start(&self) -> StudioResult<TunnelStatus> {
        let generation = {
            let mut state = self.runtime.lock().await;
            match state.state {
                TunnelState::Stopped | TunnelState::Failed => {}
                _ => return Err(StudioError::AlreadyRunning("tunnel".into())),
            }
            state.state = TunnelState::Starting;
            state.last_error = None;
            state.generation = state.generation.wrapping_add(1);
            state.generation
        };
        self.publish_status().await;

        let (working_dir, runtime_path, config_path) = match self.validated_paths() {
            Ok(paths) => paths,
            Err(error) => {
                self.mark_start_failed(generation, &error).await;
                return Err(error);
            }
        };
        let (env, secrets) = match self.resolve_secret_env() {
            Ok(env) => env,
            Err(error) => {
                self.mark_start_failed(generation, &error).await;
                return Err(error);
            }
        };

        let mut command = Command::new(&runtime_path);
        command
            .arg("run")
            .arg("--config")
            .arg(&config_path)
            .current_dir(&working_dir)
            .envs(&env)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                let studio_error = StudioError::Process(format!(
                    "tunnel: failed to spawn {}: {error}",
                    runtime_path.display()
                ));
                self.mark_start_failed(generation, &studio_error).await;
                return Err(studio_error);
            }
        };
        let Some(pid) = child.id() else {
            let error = StudioError::Process("tunnel: spawned process did not report a PID".into());
            self.mark_start_failed(generation, &error).await;
            return Err(error);
        };
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        {
            let mut state = self.runtime.lock().await;
            if state.generation != generation {
                return Err(StudioError::Process(
                    "tunnel: process generation changed during startup".into(),
                ));
            }
            state.state = TunnelState::Running;
            state.pid = Some(pid);
            state.started_at = Some(Instant::now());
            state.last_exit_code = None;
            state.last_error = None;
            let entry = state.push_log(
                self.log_capacity,
                TunnelLogStream::Studio,
                format!("started PID {pid}"),
            );
            self.events.publish(StudioEvent::TunnelLog { entry });
        }
        self.publish_status().await;

        if let Some(stdout) = stdout {
            spawn_log_reader(
                self.runtime.clone(),
                stdout,
                TunnelLogStream::Stdout,
                self.log_capacity,
                self.events.clone(),
                secrets.clone(),
            );
        }
        if let Some(stderr) = stderr {
            spawn_log_reader(
                self.runtime.clone(),
                stderr,
                TunnelLogStream::Stderr,
                self.log_capacity,
                self.events.clone(),
                secrets,
            );
        }

        let runtime = self.runtime.clone();
        let events = self.events.clone();
        let capacity = self.log_capacity;
        let name = self.config.name.clone();
        tokio::spawn(async move {
            let result = child.wait().await;
            let (status, entry) = {
                let mut state = runtime.lock().await;
                if state.generation != generation {
                    return;
                }
                let was_stopping = state.state == TunnelState::Stopping;
                state.pid = None;
                state.started_at = None;
                let message = match result {
                    Ok(exit) => {
                        state.last_exit_code = exit.code();
                        if was_stopping {
                            state.state = TunnelState::Stopped;
                            state.last_error = None;
                        } else {
                            state.state = TunnelState::Failed;
                            state.crash_count = state.crash_count.saturating_add(1);
                            state.last_error = Some(format!("process exited unexpectedly: {exit}"));
                        }
                        format!("process exited: {exit}")
                    }
                    Err(error) => {
                        state.state = TunnelState::Failed;
                        state.crash_count = state.crash_count.saturating_add(1);
                        state.last_error = Some(format!("failed waiting for process: {error}"));
                        format!("wait failed: {error}")
                    }
                };
                let entry = state.push_log(capacity, TunnelLogStream::Studio, message);
                (state.status(&name, true), entry)
            };
            events.publish(StudioEvent::TunnelLog { entry });
            events.publish(StudioEvent::TunnelStatus { status });
        });

        Ok(self.status().await)
    }

    pub async fn stop(&self) -> StudioResult<TunnelStatus> {
        let pid = {
            let mut state = self.runtime.lock().await;
            match state.state {
                TunnelState::Running | TunnelState::Starting => {
                    let Some(pid) = state.pid else {
                        return Err(StudioError::Process(
                            "tunnel: running state has no PID".into(),
                        ));
                    };
                    state.state = TunnelState::Stopping;
                    let entry = state.push_log(
                        self.log_capacity,
                        TunnelLogStream::Studio,
                        "stop requested".to_owned(),
                    );
                    self.events.publish(StudioEvent::TunnelLog { entry });
                    pid
                }
                TunnelState::Stopping => {
                    return Err(StudioError::Process(
                        "tunnel: stop already in progress".into(),
                    ));
                }
                TunnelState::Stopped | TunnelState::Failed => {
                    return Err(StudioError::NotRunning("tunnel".into()));
                }
            }
        };
        self.publish_status().await;
        send_terminate(pid)?;

        if timeout(self.stop_timeout, self.wait_for_terminal_state())
            .await
            .is_err()
        {
            send_kill(pid)?;
            timeout(self.stop_timeout, self.wait_for_terminal_state())
                .await
                .map_err(|_| {
                    StudioError::Process("tunnel: process did not exit after kill".into())
                })??;
        }

        Ok(self.status().await)
    }

    pub async fn restart(&self) -> StudioResult<TunnelStatus> {
        let status = self.status().await;
        if matches!(status.state, TunnelState::Running | TunnelState::Starting) {
            self.stop().await?;
        } else if status.state == TunnelState::Stopping {
            self.wait_for_terminal_state().await?;
        }
        {
            let mut state = self.runtime.lock().await;
            state.restart_count = state.restart_count.saturating_add(1);
        }
        self.start().await
    }

    pub async fn shutdown(&self) {
        let status = self.status().await;
        if matches!(
            status.state,
            TunnelState::Running | TunnelState::Starting | TunnelState::Stopping
        ) && let Err(error) = self.stop().await
        {
            tracing::error!(%error, "failed to stop tunnel during Studio shutdown");
        }
    }

    async fn mark_start_failed(&self, generation: u64, error: &StudioError) {
        {
            let mut state = self.runtime.lock().await;
            if state.generation != generation {
                return;
            }
            state.state = TunnelState::Failed;
            state.pid = None;
            state.started_at = None;
            state.last_error = Some(error.to_string());
            let entry = state.push_log(
                self.log_capacity,
                TunnelLogStream::Studio,
                format!("failed to start: {error}"),
            );
            self.events.publish(StudioEvent::TunnelLog { entry });
        }
        self.publish_status().await;
    }

    async fn publish_status(&self) {
        self.events.publish(StudioEvent::TunnelStatus {
            status: self.status().await,
        });
    }

    async fn wait_for_terminal_state(&self) -> StudioResult<()> {
        loop {
            let status = self.status().await;
            if matches!(status.state, TunnelState::Stopped | TunnelState::Failed) {
                return Ok(());
            }
            sleep(Duration::from_millis(25)).await;
        }
    }

    fn validated_paths(&self) -> StudioResult<(PathBuf, PathBuf, PathBuf)> {
        let working_dir = canonicalize(
            &self.base_dir,
            &self.config.working_dir,
            "working directory",
        )?;
        if !working_dir.is_dir() {
            return Err(StudioError::Config(format!(
                "tunnel working directory is not a directory: {}",
                working_dir.display()
            )));
        }

        let runtime = canonicalize(&self.base_dir, &self.config.runtime, "runtime")?;
        let config_file = canonicalize(&self.base_dir, &self.config.config_file, "config file")?;

        if !runtime.starts_with(&working_dir) || !config_file.starts_with(&working_dir) {
            return Err(StudioError::Config(
                "tunnel runtime and config file must remain inside tunnel working_dir".into(),
            ));
        }
        if !runtime.is_file() {
            return Err(StudioError::Config(format!(
                "tunnel runtime is not a file: {}",
                runtime.display()
            )));
        }
        if !is_executable(&runtime)? {
            return Err(StudioError::Config(format!(
                "tunnel runtime is not executable: {}",
                runtime.display()
            )));
        }
        if !config_file.is_file() {
            return Err(StudioError::Config(format!(
                "tunnel config is not a file: {}",
                config_file.display()
            )));
        }
        Ok((working_dir, runtime, config_file))
    }

    fn resolve_secret_env(&self) -> StudioResult<(BTreeMap<String, String>, Vec<String>)> {
        let mut env = BTreeMap::new();
        let mut secrets = Vec::new();
        for (key, reference) in &self.config.env {
            if key.trim().is_empty() {
                return Err(StudioError::Config(
                    "tunnel environment variable name must not be empty".into(),
                ));
            }
            let value = match reference {
                SecretReference::FromEnv { from_env } => std::env::var(from_env).map_err(|_| {
                    StudioError::Config(format!(
                        "tunnel secret environment reference is unavailable: {from_env}"
                    ))
                })?,
                SecretReference::FromFile { from_file } => {
                    let path = canonicalize(&self.base_dir, from_file, "secret file")?;
                    fs::read_to_string(path)?.trim_end().to_owned()
                }
            };
            if value.is_empty() {
                return Err(StudioError::Config(format!(
                    "tunnel secret reference for {key} resolved to an empty value"
                )));
            }
            secrets.push(value.clone());
            env.insert(key.clone(), value);
        }
        Ok((env, secrets))
    }
}

fn canonicalize(base_dir: &Path, path: &Path, label: &str) -> StudioResult<PathBuf> {
    let resolved = if path.is_absolute() {
        path.to_owned()
    } else {
        base_dir.join(path)
    };
    resolved.canonicalize().map_err(|error| {
        StudioError::Config(format!(
            "tunnel {label} does not resolve: {}: {error}",
            resolved.display()
        ))
    })
}

#[cfg(unix)]
fn is_executable(path: &Path) -> StudioResult<bool> {
    use std::os::unix::fs::PermissionsExt;

    Ok(fs::metadata(path)?.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> StudioResult<bool> {
    Ok(path.is_file())
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

fn redact_message(input: &str, secrets: &[String]) -> String {
    let mut value = strip_ansi(input);
    for secret in secrets {
        if !secret.is_empty() {
            value = value.replace(secret, "[REDACTED]");
        }
    }

    for key in [
        "token",
        "secret",
        "password",
        "credential",
        "authorization",
    ] {
        value = redact_named_field(value, key);
    }
    value
}

fn redact_named_field(mut value: String, key: &str) -> String {
    let mut search_from = 0;
    loop {
        let lowered = value.to_ascii_lowercase();
        if search_from >= lowered.len() {
            break;
        }
        let Some(relative_index) = lowered[search_from..].find(key) else {
            break;
        };
        let key_start = search_from + relative_index;
        let mut cursor = key_start + key.len();
        let bytes = value.as_bytes();

        while cursor < bytes.len()
            && matches!(bytes[cursor], b' ' | b'\t' | b'\'' | b'"')
        {
            cursor += 1;
        }
        if cursor >= bytes.len() || !matches!(bytes[cursor], b'=' | b':') {
            search_from = key_start + key.len();
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t') {
            cursor += 1;
        }

        let value_start = cursor;
        let quote = bytes.get(value_start).copied().filter(|byte| matches!(byte, b'\'' | b'"'));
        let content_start = value_start + usize::from(quote.is_some());
        let content_end = if let Some(quote) = quote {
            value[content_start..]
                .find(char::from(quote))
                .map(|offset| content_start + offset)
                .unwrap_or(value.len())
        } else {
            value[content_start..]
                .find([',', ' ', '\t'])
                .map(|offset| content_start + offset)
                .unwrap_or(value.len())
        };

        if content_end > content_start {
            value.replace_range(content_start..content_end, "[REDACTED]");
            search_from = content_start + "[REDACTED]".len();
        } else {
            search_from = content_start;
        }
    }
    value
}

fn spawn_log_reader<R>(
    runtime: Arc<Mutex<RuntimeState>>,
    reader: R,
    stream: TunnelLogStream,
    capacity: usize,
    events: EventHub,
    secrets: Vec<String>,
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
                        state.push_log(capacity, stream, redact_message(&line, &secrets))
                    };
                    events.publish(StudioEvent::TunnelLog { entry });
                }
                Ok(None) => break,
                Err(error) => {
                    let entry = {
                        let mut state = runtime.lock().await;
                        state.push_log(
                            capacity,
                            TunnelLogStream::Studio,
                            format!("log reader error: {error}"),
                        )
                    };
                    events.publish(StudioEvent::TunnelLog { entry });
                    break;
                }
            }
        }
    });
}

#[cfg(unix)]
fn send_terminate(pid: u32) -> StudioResult<()> {
    use nix::{
        sys::signal::{kill, Signal},
        unistd::Pid,
    };

    kill(Pid::from_raw(pid as i32), Signal::SIGTERM).map_err(|error| {
        StudioError::Process(format!("failed to terminate tunnel PID {pid}: {error}"))
    })
}

#[cfg(unix)]
fn send_kill(pid: u32) -> StudioResult<()> {
    use nix::{
        sys::signal::{kill, Signal},
        unistd::Pid,
    };

    kill(Pid::from_raw(pid as i32), Signal::SIGKILL).map_err(|error| {
        StudioError::Process(format!("failed to kill tunnel PID {pid}: {error}"))
    })
}

#[cfg(not(unix))]
fn send_terminate(_pid: u32) -> StudioResult<()> {
    Err(StudioError::Process(
        "tunnel process control currently requires a Unix platform".into(),
    ))
}

#[cfg(not(unix))]
fn send_kill(_pid: u32) -> StudioResult<()> {
    Err(StudioError::Process(
        "tunnel process control currently requires a Unix platform".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_exact_secret_values() {
        assert_eq!(
            redact_message("token=abc123 host=example", &["abc123".into()]),
            "token=[REDACTED] host=example"
        );
    }

    #[test]
    fn redacts_common_secret_fields_without_known_value() {
        assert_eq!(
            redact_message(
                "token=abc authorization: Bearer password='hello world' host=example",
                &[]
            ),
            "token=[REDACTED] authorization: [REDACTED] password='[REDACTED]' host=example"
        );
    }

    #[test]
    fn tunnel_status_serialization_contains_no_config_or_secret_fields() {
        let status = TunnelStatus {
            name: "Secure tunnel".into(),
            state: TunnelState::Stopped,
            runtime_available: true,
            pid: None,
            uptime_ms: None,
            restart_count: 0,
            crash_count: 0,
            last_exit_code: None,
            last_error: None,
        };
        let json = serde_json::to_string(&status).unwrap();
        assert!(!json.contains("runtime_path"));
        assert!(!json.contains("config_file"));
        assert!(!json.contains("secret"));
    }
}
