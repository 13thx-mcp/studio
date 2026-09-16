#![cfg(unix)]

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use mcp_studio::{
    config::{McpServerConfig, RegistryConfig},
    error::StudioError,
    registry::Registry,
    supervisor::{ProcessState, Supervisor},
};

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
    let path = std::env::temp_dir().join(format!(
        "mcp-studio-supervisor-{name}-{}-{}",
        std::process::id(),
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

fn local_executable(project: &Path, source: &str, name: &str) -> PathBuf {
    let bin = project.join("bin");
    fs::create_dir_all(&bin).unwrap();
    let destination = bin.join(name);
    fs::copy(source, &destination).unwrap();
    let mut permissions = fs::metadata(&destination).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&destination, permissions).unwrap();
    destination
}

fn supervisor_with(id: &str, source_command: &str, args: &[&str]) -> Fixture {
    let root = temp_root(id);
    let project = root.join(id);
    fs::create_dir_all(&project).unwrap();
    let command = local_executable(&project, source_command, "fixture-bin");
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
async fn starts_and_stops_process() {
    let fixture = supervisor_with("fixture", "/bin/sleep", &["30"]);
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
    let fixture = supervisor_with("fixture", "/bin/sleep", &["30"]);
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
async fn detects_unexpected_nonzero_exit() {
    let fixture = supervisor_with("fixture", "/bin/sh", &["-c", "exit 7"]);
    let supervisor = &fixture.supervisor;

    supervisor.start("fixture").await.unwrap();
    wait_for_state(supervisor, "fixture", ProcessState::Failed).await;

    let status = supervisor.status("fixture").await.unwrap();
    assert_eq!(status.last_exit_code, Some(7));
    assert_eq!(status.crash_count, 1);
}

#[tokio::test]
async fn rejects_duplicate_start() {
    let fixture = supervisor_with("fixture", "/bin/sleep", &["30"]);
    let supervisor = &fixture.supervisor;

    supervisor.start("fixture").await.unwrap();
    let error = supervisor.start("fixture").await.unwrap_err();
    assert!(matches!(error, StudioError::AlreadyRunning(_)));

    supervisor.stop("fixture").await.unwrap();
}

#[tokio::test]
async fn rejects_stop_when_already_stopped() {
    let fixture = supervisor_with("fixture", "/bin/sleep", &["30"]);
    let error = fixture.supervisor.stop("fixture").await.unwrap_err();
    assert!(matches!(error, StudioError::NotRunning(_)));
}

#[tokio::test]
async fn disabled_mcp_cannot_start() {
    let fixture = supervisor_with("fixture", "/bin/sleep", &["30"]);
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
    let fixture = supervisor_with("fixture", "/bin/sleep", &["30"]);
    let supervisor = &fixture.supervisor;

    supervisor.start("fixture").await.unwrap();
    supervisor.shutdown_all().await;

    assert_eq!(
        supervisor.status("fixture").await.unwrap().state,
        ProcessState::Stopped
    );
}