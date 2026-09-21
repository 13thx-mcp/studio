#![cfg(unix)]

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use mcp_studio::{
    error::StudioError,
    realtime::{EventHub, StudioEvent},
    storage::HistoryHandle,
    tunnel::{SecretReference, TunnelConfig, TunnelState, TunnelStatus, TunnelSupervisor},
};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(1);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(script: &str) -> Self {
        let unique = format!(
            "mcp-studio-tunnel-test-{}-{}",
            std::process::id(),
            NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed)
        );
        let root = std::env::temp_dir().join(unique);
        fs::create_dir(&root).unwrap();
        fs::write(root.join("config.yaml"), "config_version: 1\n").unwrap();
        let runtime = root.join("runtime.sh");
        fs::write(&runtime, script).unwrap();
        let mut permissions = fs::metadata(&runtime).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&runtime, permissions).unwrap();
        Self { root }
    }

    fn supervisor(&self, events: EventHub) -> TunnelSupervisor {
        TunnelSupervisor::new(
            TunnelConfig {
                name: "Test tunnel".into(),
                runtime: self.root.join("runtime.sh"),
                working_dir: self.root.clone(),
                config_file: self.root.join("config.yaml"),
                env: BTreeMap::new(),
            },
            64,
            Duration::from_secs(1),
            PathBuf::from("/"),
            events,
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const LONG_RUNNING: &str = r#"#!/bin/sh
trap 'exit 0' TERM INT
printf 'tunnel ready\n'
while :; do sleep 1; done
"#;

#[tokio::test]
async fn start_stop_and_duplicate_start_are_safe() {
    let fixture = Fixture::new(LONG_RUNNING);
    let supervisor = fixture.supervisor(EventHub::default());

    let running = supervisor.start().await.unwrap();
    assert_eq!(running.state, TunnelState::Running);
    assert!(running.pid.is_some());

    assert!(matches!(
        supervisor.start().await.unwrap_err(),
        StudioError::AlreadyRunning(_)
    ));

    let stopped = supervisor.stop().await.unwrap();
    assert_eq!(stopped.state, TunnelState::Stopped);
    assert!(stopped.pid.is_none());
    assert_eq!(stopped.crash_count, 0);

    assert!(matches!(
        supervisor.stop().await.unwrap_err(),
        StudioError::NotRunning(_)
    ));
}

#[tokio::test]
async fn restart_replaces_pid_and_increments_restart_count() {
    let fixture = Fixture::new(LONG_RUNNING);
    let supervisor = fixture.supervisor(EventHub::default());

    let first = supervisor.start().await.unwrap();
    let first_pid = first.pid.unwrap();
    let restarted = supervisor.restart().await.unwrap();

    assert_eq!(restarted.state, TunnelState::Running);
    assert_ne!(restarted.pid, Some(first_pid));
    assert_eq!(restarted.restart_count, 1);
    assert_eq!(restarted.crash_count, 0);

    supervisor.shutdown().await;
    assert_eq!(supervisor.status().await.state, TunnelState::Stopped);
}

#[tokio::test]
async fn unexpected_nonzero_exit_transitions_to_failed_and_counts_crash() {
    let fixture = Fixture::new("#!/bin/sh\nprintf 'about to fail\\n'\nexit 7\n");
    let supervisor = fixture.supervisor(EventHub::default());

    let _ = supervisor.start().await.unwrap();
    let terminal = wait_for_state(&supervisor, TunnelState::Failed).await;

    assert_eq!(terminal.crash_count, 1);
    assert_eq!(terminal.last_exit_code, Some(7));
}

#[tokio::test]
async fn unexpected_zero_exit_is_still_a_crash() {
    let fixture = Fixture::new("#!/bin/sh\nprintf 'completed unexpectedly\\n'\nexit 0\n");
    let supervisor = fixture.supervisor(EventHub::default());

    let _ = supervisor.start().await.unwrap();
    let terminal = wait_for_state(&supervisor, TunnelState::Failed).await;

    assert_eq!(terminal.crash_count, 1);
    assert_eq!(terminal.last_exit_code, Some(0));
    assert!(
        terminal
            .last_error
            .as_deref()
            .unwrap_or_default()
            .contains("unexpectedly")
    );
}

#[tokio::test]
async fn tunnel_clean_zero_exit_is_still_crash_in_history() {
    let fixture = Fixture::new(
        "#!/bin/sh
exit 0
",
    );
    let history = HistoryHandle::initialize(&fixture.root.join("history-runtime"));
    history
        .start_run(uuid::Uuid::new_v4(), "tunnel-history-test", None)
        .unwrap();
    let supervisor = TunnelSupervisor::new_with_history(
        TunnelConfig {
            name: "Test tunnel".into(),
            runtime: fixture.root.join("runtime.sh"),
            working_dir: fixture.root.clone(),
            config_file: fixture.root.join("config.yaml"),
            env: BTreeMap::new(),
        },
        64,
        Duration::from_secs(1),
        PathBuf::from("/"),
        EventHub::default(),
        history.clone(),
    );

    supervisor.start().await.unwrap();
    wait_for_state(&supervisor, TunnelState::Failed).await;
    history.close_run(true).unwrap();
    history.shutdown();

    let connection = rusqlite::Connection::open(
        fixture
            .root
            .join("history-runtime/studio/data/history/studio.sqlite3"),
    )
    .unwrap();
    let (end_kind, exit_code, is_crash): (String, Option<i32>, Option<i64>) = connection
        .query_row(
            "SELECT end_kind, exit_code, is_crash FROM runtime_sessions LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(end_kind, "unexpected_exit");
    assert_eq!(exit_code, Some(0));
    assert_eq!(is_crash, Some(1));
}

#[tokio::test]
async fn tunnel_shutdown_completes_with_stalled_history() {
    let fixture = Fixture::new(LONG_RUNNING);
    let history = HistoryHandle::initialize(&fixture.root.join("history-runtime"));
    history
        .start_run(uuid::Uuid::new_v4(), "stalled-tunnel-history", None)
        .unwrap();
    let supervisor = TunnelSupervisor::new_with_history(
        TunnelConfig {
            name: "Test tunnel".into(),
            runtime: fixture.root.join("runtime.sh"),
            working_dir: fixture.root.clone(),
            config_file: fixture.root.join("config.yaml"),
            env: BTreeMap::new(),
        },
        64,
        Duration::from_secs(1),
        PathBuf::from("/"),
        EventHub::default(),
        history.clone(),
    );

    assert_eq!(
        supervisor.start().await.unwrap().state,
        TunnelState::Running
    );
    history.shutdown();

    supervisor.shutdown().await;
    assert_eq!(supervisor.status().await.state, TunnelState::Stopped);
}

#[tokio::test]
async fn invalid_runtime_is_rejected_and_recorded_as_failed() {
    let fixture = Fixture::new(LONG_RUNNING);
    let supervisor = TunnelSupervisor::new(
        TunnelConfig {
            name: "Broken tunnel".into(),
            runtime: fixture.root.join("missing-runtime"),
            working_dir: fixture.root.clone(),
            config_file: fixture.root.join("config.yaml"),
            env: BTreeMap::new(),
        },
        64,
        Duration::from_secs(1),
        PathBuf::from("/"),
        EventHub::default(),
    );

    assert!(matches!(
        supervisor.start().await,
        Err(StudioError::Config(_))
    ));
    let status = supervisor.status().await;
    assert_eq!(status.state, TunnelState::Failed);
    assert!(!status.runtime_available);
    assert!(status.last_error.is_some());
}

#[tokio::test]
async fn publishes_realtime_status_and_log_events() {
    let fixture = Fixture::new(LONG_RUNNING);
    let events = EventHub::default();
    let mut receiver = events.subscribe();
    let supervisor = fixture.supervisor(events);

    supervisor.start().await.unwrap();

    let mut saw_status = false;
    let mut saw_log = false;
    for _ in 0..12 {
        let Ok(Ok(event)) = tokio::time::timeout(Duration::from_millis(250), receiver.recv()).await
        else {
            break;
        };
        match event {
            StudioEvent::TunnelStatus { status } if status.state == TunnelState::Running => {
                saw_status = true;
            }
            StudioEvent::TunnelLog { entry }
                if entry.message.contains("started PID")
                    || entry.message.contains("tunnel ready") =>
            {
                saw_log = true;
            }
            _ => {}
        }
        if saw_status && saw_log {
            break;
        }
    }

    supervisor.shutdown().await;
    assert!(saw_status, "expected running tunnel status event");
    assert!(saw_log, "expected tunnel log event");
}

#[tokio::test]
async fn secret_value_is_redacted_before_logs_are_exposed() {
    let fixture = Fixture::new(
        "#!/bin/sh\nprintf 'token=%s\\n' \"$TEST_SECRET\"\ntrap 'exit 0' TERM INT\nwhile :; do sleep 1; done\n",
    );
    let secret_file = fixture.root.join("secret.txt");
    fs::write(&secret_file, "super-secret-value\n").unwrap();
    let mut env = BTreeMap::new();
    env.insert(
        "TEST_SECRET".into(),
        SecretReference::FromFile {
            from_file: secret_file,
        },
    );
    let supervisor = TunnelSupervisor::new(
        TunnelConfig {
            name: "Secret tunnel".into(),
            runtime: fixture.root.join("runtime.sh"),
            working_dir: fixture.root.clone(),
            config_file: fixture.root.join("config.yaml"),
            env,
        },
        64,
        Duration::from_secs(1),
        PathBuf::from("/"),
        EventHub::default(),
    );

    supervisor.start().await.unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    loop {
        let logs = supervisor.logs().await;
        let serialized = serde_json::to_string(&logs).unwrap();
        assert!(
            !serialized.contains("super-secret-value"),
            "secret value became observable before redaction"
        );
        if serialized.contains("[REDACTED]") {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for redacted tunnel log; last logs: {serialized}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    supervisor.shutdown().await;
}

async fn wait_for_state(supervisor: &TunnelSupervisor, expected: TunnelState) -> TunnelStatus {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let status = supervisor.status().await;
        if status.state == expected {
            return status;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for tunnel state {expected:?}; last status: {status:?}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}
