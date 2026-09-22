#![cfg(unix)]

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use mcp_studio::{
    config::{McpServerConfig, RegistryConfig},
    error::StudioError,
    registry::Registry,
    reliability::RestartPolicy,
    storage::HistoryHandle,
    supervisor::{ProcessState, Supervisor},
    update::RuntimeOperationCoordinator,
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    supervisor: Supervisor,
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn temp_root(name: &str) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "mcp-studio-supervisor-{name}-{}-{}-{sequence}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn local_script(project: &Path, body: &str, name: &str) -> PathBuf {
    let bin = project.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let destination = bin.join(name);
    fs::write(&destination, format!("#!/bin/sh\n{body}\n")).unwrap();
    let mut permissions = fs::metadata(&destination).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&destination, permissions).unwrap();
    destination
}

fn restart_policy() -> RestartPolicy {
    RestartPolicy {
        enabled: true,
        max_attempts: 3,
        stability_window_ms: 100,
        initial_backoff_ms: 10,
        max_backoff_ms: 40,
        cooldown_ms: 100,
    }
}

fn supervisor_with_restart(
    id: &str,
    script_body: &str,
    args: &[&str],
) -> (Fixture, Arc<RuntimeOperationCoordinator>) {
    let root = temp_root(id);
    let project = root.join(id);
    fs::create_dir_all(&project).unwrap();
    let command = local_script(&project, script_body, "fixture-bin");
    let server = McpServerConfig {
        name: id.to_owned(),
        command,
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        working_dir: project,
        env: BTreeMap::new(),
    };
    let registry = Registry::open(
        Path::new("."),
        &RegistryConfig {
            path: root.join("data/registry.toml"),
            mcp_root: root.clone(),
        },
        &BTreeMap::from([(id.to_owned(), server)]),
    )
    .unwrap();
    let history = HistoryHandle::initialize(&root.join("history-runtime"));
    history
        .start_run(uuid::Uuid::new_v4(), "m8-restart-test", None)
        .unwrap();
    history.mark_run_ready().unwrap();
    let coordinator = Arc::new(RuntimeOperationCoordinator::default());
    (
        Fixture {
            supervisor: Supervisor::new_with_history_and_restart(
                registry,
                64,
                Duration::from_secs(2),
                history,
                restart_policy(),
                coordinator.clone(),
            ),
            root,
        },
        coordinator,
    )
}

fn supervisor_with(id: &str, script_body: &str, args: &[&str]) -> Fixture {
    let root = temp_root(id);
    let project = root.join(id);
    fs::create_dir_all(&project).unwrap();
    let command = local_script(&project, script_body, "fixture-bin");
    let server = McpServerConfig {
        name: id.to_owned(),
        command,
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        working_dir: project,
        env: BTreeMap::new(),
    };
    let registry = Registry::open(
        Path::new("."),
        &RegistryConfig {
            path: root.join("data/registry.toml"),
            mcp_root: root.clone(),
        },
        &BTreeMap::from([(id.to_owned(), server)]),
    )
    .unwrap();
    Fixture {
        supervisor: Supervisor::new(registry, 64, Duration::from_secs(2)),
        root,
    }
}

async fn wait_for_state(supervisor: &Supervisor, id: &str, expected: ProcessState) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if supervisor.status(id).await.unwrap().state == expected {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("state transition timed out");
}

#[tokio::test]
async fn m8_crash_schedules_and_processes_bounded_restart() {
    let (fixture, _coordinator) = supervisor_with_restart("auto-restart", "exit 42", &[]);
    let supervisor = &fixture.supervisor;

    supervisor.start("auto-restart").await.unwrap();
    wait_for_state(supervisor, "auto-restart", ProcessState::Failed).await;
    let failed = supervisor.status("auto-restart").await.unwrap();
    assert!(failed.desired_running);
    assert_eq!(failed.consecutive_restart_failures, 1);
    let due = failed.retry_at_ms.expect("crash must schedule retry");

    supervisor.process_due_restarts(due).await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    let after = supervisor.status("auto-restart").await.unwrap();
    assert!(after.restart_count >= 1);
    assert!(after.consecutive_restart_failures >= 1);
}

#[tokio::test]
async fn m8_explicit_stop_suppresses_restart_intent() {
    let (fixture, _coordinator) =
        supervisor_with_restart("stop-suppresses", "exec /bin/sleep 30", &[]);
    let supervisor = &fixture.supervisor;
    supervisor.start("stop-suppresses").await.unwrap();
    let stopped = supervisor.stop("stop-suppresses").await.unwrap();
    assert!(!stopped.desired_running);
    assert!(stopped.retry_at_ms.is_none());
    assert!(supervisor.next_restart_due_ms().await.is_none());
}

#[tokio::test]
async fn m8_runtime_operation_conflict_defers_without_failure_increment() {
    let (fixture, coordinator) = supervisor_with_restart("busy-restart", "exit 42", &[]);
    let supervisor = &fixture.supervisor;
    supervisor.start("busy-restart").await.unwrap();
    wait_for_state(supervisor, "busy-restart", ProcessState::Failed).await;
    let before = supervisor.status("busy-restart").await.unwrap();
    let due = before.retry_at_ms.unwrap();
    let lease = coordinator.acquire_control("m8_test_busy").unwrap();

    supervisor.process_due_restarts(due).await;
    let deferred = supervisor.status("busy-restart").await.unwrap();
    assert_eq!(
        deferred.consecutive_restart_failures,
        before.consecutive_restart_failures
    );
    assert_eq!(deferred.restart_count, before.restart_count);
    assert!(deferred.retry_at_ms.unwrap() > due);
    drop(lease);
}

#[tokio::test]
async fn starts_and_stops_process() {
    let fixture = supervisor_with("fixture", "exec /bin/sleep \"$@\"", &["30"]);
    let supervisor = &fixture.supervisor;

    let started = supervisor.start("fixture").await.unwrap();
    assert_eq!(started.state, ProcessState::Running);
    assert!(started.pid.is_some());

    let stopped = supervisor.stop("fixture").await.unwrap();
    assert_eq!(stopped.state, ProcessState::Stopped);
    assert!(stopped.pid.is_none());
}

#[tokio::test]
async fn restart_replaces_process_and_increments_counter() {
    let fixture = supervisor_with("fixture", "exec /bin/sleep \"$@\"", &["30"]);
    let supervisor = &fixture.supervisor;

    let first = supervisor.start("fixture").await.unwrap();
    let first_pid = first.pid.unwrap();

    let restarted = supervisor.restart("fixture").await.unwrap();
    assert_eq!(restarted.state, ProcessState::Running);
    assert_ne!(restarted.pid, Some(first_pid));
    assert_eq!(restarted.restart_count, 1);

    supervisor.stop("fixture").await.unwrap();
}

#[tokio::test]
async fn mcp_clean_zero_exit_is_not_crash_in_history() {
    let root = temp_root("history-zero");
    let project = root.join("fixture");
    fs::create_dir_all(&project).unwrap();
    let command = local_script(&project, "exit 0", "fixture-bin");
    let server = McpServerConfig {
        name: "fixture".to_owned(),
        command,
        args: vec![],
        working_dir: project,
        env: BTreeMap::new(),
    };
    let registry = Registry::open(
        Path::new("."),
        &RegistryConfig {
            path: root.join("data/registry.toml"),
            mcp_root: root.clone(),
        },
        &BTreeMap::from([("fixture".to_owned(), server)]),
    )
    .unwrap();
    let history = HistoryHandle::initialize(&root.join("history-runtime"));
    history
        .start_run(uuid::Uuid::new_v4(), "supervisor-history-test", None)
        .unwrap();
    let supervisor =
        Supervisor::new_with_history(registry, 64, Duration::from_secs(2), history.clone());

    supervisor.start("fixture").await.unwrap();
    wait_for_state(&supervisor, "fixture", ProcessState::Stopped).await;
    history.close_run(true).unwrap();
    history.shutdown();

    let connection =
        rusqlite::Connection::open(root.join("history-runtime/studio/data/history/studio.sqlite3"))
            .unwrap();
    let (end_kind, exit_code, is_crash): (String, Option<i32>, Option<i64>) = connection
        .query_row(
            "SELECT end_kind, exit_code, is_crash FROM runtime_sessions LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(end_kind, "clean_exit");
    assert_eq!(exit_code, Some(0));
    assert_eq!(is_crash, Some(0));
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn shutdown_completes_with_stalled_history() {
    let root = temp_root("history-stalled-stop");
    let project = root.join("fixture");
    fs::create_dir_all(&project).unwrap();
    let command = local_script(&project, "exec /bin/sleep 30", "fixture-bin");
    let server = McpServerConfig {
        name: "fixture".to_owned(),
        command,
        args: vec![],
        working_dir: project,
        env: BTreeMap::new(),
    };
    let registry = Registry::open(
        Path::new("."),
        &RegistryConfig {
            path: root.join("data/registry.toml"),
            mcp_root: root.clone(),
        },
        &BTreeMap::from([("fixture".to_owned(), server)]),
    )
    .unwrap();
    let history = HistoryHandle::initialize(&root.join("history-runtime"));
    history
        .start_run(uuid::Uuid::new_v4(), "stalled-history-stop", None)
        .unwrap();
    let supervisor =
        Supervisor::new_with_history(registry, 64, Duration::from_secs(2), history.clone());

    assert_eq!(
        supervisor.start("fixture").await.unwrap().state,
        ProcessState::Running
    );
    history.shutdown();

    let stopped = supervisor.stop("fixture").await.unwrap();
    assert_eq!(stopped.state, ProcessState::Stopped);
    assert!(stopped.pid.is_none());
    let _ = fs::remove_dir_all(root);
}

#[tokio::test]
async fn detects_unexpected_nonzero_exit() {
    let fixture = supervisor_with("fixture", "exit 7", &[]);
    let supervisor = &fixture.supervisor;

    supervisor.start("fixture").await.unwrap();
    wait_for_state(supervisor, "fixture", ProcessState::Failed).await;

    let status = supervisor.status("fixture").await.unwrap();
    assert_eq!(status.last_exit_code, Some(7));
    assert_eq!(status.crash_count, 1);
}

#[tokio::test]
async fn rejects_duplicate_start() {
    let fixture = supervisor_with("fixture", "exec /bin/sleep \"$@\"", &["30"]);
    let supervisor = &fixture.supervisor;

    supervisor.start("fixture").await.unwrap();
    let error = supervisor.start("fixture").await.unwrap_err();
    assert!(matches!(error, StudioError::AlreadyRunning(_)));

    supervisor.stop("fixture").await.unwrap();
}

#[tokio::test]
async fn rejects_stop_when_already_stopped() {
    let fixture = supervisor_with("fixture", "exec /bin/sleep \"$@\"", &["30"]);
    let error = fixture.supervisor.stop("fixture").await.unwrap_err();
    assert!(matches!(error, StudioError::NotRunning(_)));
}

#[tokio::test]
async fn disabled_mcp_cannot_start() {
    let fixture = supervisor_with("fixture", "exec /bin/sleep \"$@\"", &["30"]);
    fixture
        .supervisor
        .registry()
        .set_enabled("fixture", false)
        .unwrap();
    let error = fixture.supervisor.start("fixture").await.unwrap_err();
    assert!(matches!(error, StudioError::Disabled(_)));
}

#[tokio::test]
async fn shutdown_all_stops_owned_processes() {
    let fixture = supervisor_with("fixture", "exec /bin/sleep \"$@\"", &["30"]);
    let supervisor = &fixture.supervisor;

    supervisor.start("fixture").await.unwrap();
    supervisor.shutdown_all().await;

    assert_eq!(
        supervisor.status("fixture").await.unwrap().state,
        ProcessState::Stopped
    );
}
