#![cfg(unix)]

use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use mcp_studio::{
    config::McpServerConfig,
    error::StudioError,
    registry::Registry,
    supervisor::{ProcessState, Supervisor},
};

fn supervisor_with(id: &str, command: &str, args: &[&str]) -> Supervisor {
    let server = McpServerConfig {
        name: id.to_owned(),
        command: PathBuf::from(command),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        working_dir: std::env::current_dir().unwrap(),
        env: BTreeMap::new(),
    };
    Supervisor::new(
        Registry::new(BTreeMap::from([(id.to_owned(), server)])),
        64,
        Duration::from_secs(2),
        std::env::current_dir().unwrap(),
    )
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
    let supervisor = supervisor_with("fixture", "/bin/sleep", &["30"]);

    let started = supervisor.start("fixture").await.unwrap();
    assert_eq!(started.state, ProcessState::Running);
    assert!(started.pid.is_some());

    let stopped = supervisor.stop("fixture").await.unwrap();
    assert_eq!(stopped.state, ProcessState::Stopped);
    assert!(stopped.pid.is_none());
}

#[tokio::test]
async fn restart_replaces_process_and_increments_counter() {
    let supervisor = supervisor_with("fixture", "/bin/sleep", &["30"]);

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
    let supervisor = supervisor_with("fixture", "/bin/sh", &["-c", "exit 7"]);

    supervisor.start("fixture").await.unwrap();
    wait_for_state(&supervisor, "fixture", ProcessState::Failed).await;

    let status = supervisor.status("fixture").await.unwrap();
    assert_eq!(status.last_exit_code, Some(7));
    assert_eq!(status.crash_count, 1);
}

#[tokio::test]
async fn rejects_duplicate_start() {
    let supervisor = supervisor_with("fixture", "/bin/sleep", &["30"]);

    supervisor.start("fixture").await.unwrap();
    let error = supervisor.start("fixture").await.unwrap_err();
    assert!(matches!(error, StudioError::AlreadyRunning(_)));

    supervisor.stop("fixture").await.unwrap();
}

#[tokio::test]
async fn rejects_stop_when_already_stopped() {
    let supervisor = supervisor_with("fixture", "/bin/sleep", &["30"]);

    let error = supervisor.stop("fixture").await.unwrap_err();
    assert!(matches!(error, StudioError::NotRunning(_)));
}

#[tokio::test]
async fn invalid_executable_transitions_to_failed() {
    let supervisor = supervisor_with("fixture", "/definitely/not/an/executable", &[]);

    let error = supervisor.start("fixture").await.unwrap_err();
    assert!(matches!(error, StudioError::Process(_)));

    let status = supervisor.status("fixture").await.unwrap();
    assert_eq!(status.state, ProcessState::Failed);
    assert!(status.last_error.is_some());
}

#[tokio::test]
async fn shutdown_all_stops_owned_processes() {
    let supervisor = supervisor_with("fixture", "/bin/sleep", &["30"]);

    supervisor.start("fixture").await.unwrap();
    supervisor.shutdown_all().await;

    assert_eq!(
        supervisor.status("fixture").await.unwrap().state,
        ProcessState::Stopped
    );
}
