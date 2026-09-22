use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use clap::{Parser, Subcommand};
use mcp_studio::{
    api::{self, AppState},
    automation::AutomationController,
    config::{LoadedConfigIdentity, StudioConfig},
    discovery::DiscoveryService,
    logging,
    operation::OperationService,
    realtime::EventHub,
    registry::Registry,
    storage::HistoryHandle,
    supervisor::Supervisor,
    tunnel::TunnelSupervisor,
    update::{
        ComponentCatalog, ComponentId, FleetUpdateManager, GatewayControlClient,
        GatewayUpdateManager, HostPlatform, HostRuntimeRoots, InventoryService, McpUpdateManager,
        RuntimeReconciler, SelfUpdateManager, StudioProcessConfigIdentity, TunnelUpdateManager,
        Version, write_activation_ready_proof_from_env,
    },
};
use tokio::sync::Notify;

#[derive(Debug, Parser)]
#[command(version, about = "MCP Studio local control plane")]
struct Cli {
    /// Optional configuration file. If omitted, built-in safe defaults are used.
    #[arg(long, env = "MCP_STUDIO_CONFIG")]
    config: Option<std::path::PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Offline/foreground history maintenance using the configured runtime root.
    History {
        #[command(subcommand)]
        command: HistoryCommand,
    },
}

#[derive(Debug, Subcommand)]
enum HistoryCommand {
    /// Create a consistent private SQLite backup.
    Backup,
    /// Verify history schema, integrity, and foreign keys.
    Verify,
}

#[tokio::main]
async fn main() -> Result<()> {
    logging::init()?;
    let cli = Cli::parse();

    let (config, loaded_config_identity) = match cli.config {
        Some(path) => StudioConfig::load_with_identity(&path)?,
        None => (StudioConfig::default(), LoadedConfigIdentity::builtin()),
    };
    config.validate()?;
    let automation_policy_fingerprint = loaded_config_identity
        .sha256
        .clone()
        .unwrap_or(config.automation.fingerprint()?);

    let base_dir = std::env::current_dir()?;
    let (source_root, bin_root, runtime_root) = config.updates.resolve_roots(&base_dir);
    let activation_runtime_root = runtime_root.clone();
    let history = HistoryHandle::initialize(&runtime_root);
    let operations = OperationService::new(history.clone());
    if let Some(Command::History { command }) = cli.command {
        match command {
            HistoryCommand::Backup => {
                let backup = history.backup()?;
                history.shutdown();
                println!("{}", backup.display());
            }
            HistoryCommand::Verify => {
                history.verify()?;
                history.shutdown();
                println!("history verification passed");
            }
        }
        return Ok(());
    }

    let history_run_id = uuid::Uuid::new_v4();
    let process_instance = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    if let Err(error) = history.start_run(
        history_run_id,
        &process_instance,
        loaded_config_identity.sha256.as_deref(),
    ) {
        tracing::warn!(history_error = %error, "could not persist Studio run start");
    } else {
        if let Err(error) =
            history.record_bootstrap(history_run_id, &format!("bootstrap/{history_run_id}"), 0)
        {
            tracing::warn!(history_error = %error, "could not persist M6 bootstrap observation");
        }
        if let Err(error) = history.housekeeping() {
            tracing::warn!(history_error = %error, "initial history housekeeping failed");
        }
    }

    let activation_config_identity = loaded_config_identity.clone();
    let studio_config_identity =
        StudioProcessConfigIdentity::from_loaded(loaded_config_identity, process_instance);
    let registry = Registry::open(&base_dir, &config.registry, &config.mcp)?;
    let supervisor = Arc::new(Supervisor::new_with_history(
        registry.clone(),
        config.log_capacity,
        Duration::from_millis(config.stop_timeout_ms),
        history.clone(),
    ));
    let discovery = Arc::new(DiscoveryService::new(registry));

    let catalog = ComponentCatalog::new(HostRuntimeRoots::new(bin_root, runtime_root)?);
    let runtime_operations = catalog.runtime_operations();
    let mut desired = std::collections::BTreeMap::new();
    for (component, version) in &config.updates.desired {
        desired.insert(component.parse::<ComponentId>()?, Version::parse(version)?);
    }
    let inventory = Arc::new(InventoryService::new(
        catalog.clone(),
        source_root,
        HostPlatform::detect()?.platform(),
        desired,
    )?);
    let gateway_history_client = GatewayControlClient::from_catalog(&catalog);

    let registry_events = EventHub::default();
    let tunnel = Arc::new(TunnelSupervisor::new_with_history(
        config.tunnel.clone(),
        config.log_capacity,
        Duration::from_millis(config.stop_timeout_ms),
        base_dir,
        EventHub::default(),
        history.clone(),
    ));
    let updates = Arc::new(McpUpdateManager::new(
        catalog.clone(),
        supervisor.clone(),
        inventory.clone(),
        registry_events.clone(),
    )?);
    let gateway_updates = Arc::new(GatewayUpdateManager::new(
        catalog.clone(),
        tunnel.clone(),
        inventory.clone(),
        registry_events.clone(),
    )?);
    let fleet_updates = Arc::new(FleetUpdateManager::new(
        catalog.clone(),
        inventory.clone(),
        registry_events.clone(),
    )?);
    let self_updates = Arc::new(SelfUpdateManager::new(
        catalog.clone(),
        inventory.clone(),
        registry_events.clone(),
    )?);
    let tunnel_updates = Arc::new(TunnelUpdateManager::new_with_history(
        catalog.clone(),
        tunnel.clone(),
        inventory.clone(),
        registry_events.clone(),
        history.clone(),
    )?);
    tunnel_updates.recover_startup().await?;
    let finalized_self_updates = self_updates.finalize_startup().await?;
    for view in &finalized_self_updates {
        history.observe_update_transaction(view, None, false);
    }
    if !finalized_self_updates.is_empty() {
        tracing::info!(
            transaction_count = finalized_self_updates.len(),
            "finalized persisted Studio self-update transaction state"
        );
    }
    let reconciliation = Arc::new(RuntimeReconciler::new_with_config_identity(
        catalog,
        tunnel.clone(),
        inventory.clone(),
        registry_events.clone(),
        studio_config_identity,
    )?);
    let startup_reconciliation = reconciliation.check().await?;
    tracing::info!(
        reconciliation_state = ?startup_reconciliation.state,
        reconciliation_generation = startup_reconciliation.generation,
        safe_to_reconcile = startup_reconciliation.safe_to_reconcile,
        "runtime reconciliation startup check completed"
    );

    let automation = Arc::new(AutomationController::new(
        config.automation.clone(),
        activation_runtime_root.clone(),
        Some(automation_policy_fingerprint),
        runtime_operations.clone(),
    ));
    if let Some(blocker) = automation.blocker().await {
        tracing::warn!(
            ?blocker,
            "M8 automation foundation is observation-only and destructive automation is blocked"
        );
    }

    let addr: SocketAddr = config.server.listen_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    if write_activation_ready_proof_from_env(&activation_runtime_root, &activation_config_identity)?
    {
        tracing::info!("published process-bound Studio activation proof");
    }
    if let Err(error) = history.mark_run_ready() {
        tracing::warn!(history_error = %error, "could not persist Studio ready observation");
    }
    let automation_shutdown = Arc::new(Notify::new());
    let automation_task = tokio::spawn(automation.clone().run(automation_shutdown.clone()));
    tokio::spawn(ingest_gateway_history(
        history.clone(),
        gateway_history_client,
    ));
    tracing::info!(
        listen_addr = %addr,
        managed_mcp_count = supervisor.registry().ids().len(),
        mcp_root = %supervisor.registry().canonical_root().display(),
        "starting MCP Studio"
    );

    let periodic_history = history.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            let handle = periodic_history.clone();
            let result = tokio::task::spawn_blocking(move || handle.housekeeping()).await;
            match result {
                Ok(Ok(_)) => {}
                Ok(Err(error)) => {
                    tracing::warn!(history_error = %error, "periodic history housekeeping stopped");
                    break;
                }
                Err(error) => {
                    tracing::warn!(%error, "history housekeeping task failed");
                    break;
                }
            }
        }
    });

    let shutdown_supervisor = supervisor.clone();
    let shutdown_tunnel = tunnel.clone();
    let shutdown_history = history.clone();
    let shutdown_automation = automation_shutdown.clone();
    let self_update_shutdown = Arc::new(Notify::new());
    let shutdown_request = self_update_shutdown.clone();
    let app = api::router(AppState {
        supervisor,
        discovery,
        tunnel,
        inventory,
        updates,
        gateway_updates,
        fleet_updates,
        self_updates,
        tunnel_updates,
        reconciliation,
        runtime_operations,
        operations,
        history: history.clone(),
        shutdown: self_update_shutdown,
        registry_events,
    });
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            tokio::select! {
                _ = shutdown_signal() => {}
                _ = shutdown_request.notified() => {
                    tracing::info!("Studio self-update requested graceful shutdown");
                }
            }
            shutdown_automation.notify_waiters();
            match tokio::time::timeout(Duration::from_secs(1), automation_task).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%error, "automation controller task failed during shutdown");
                }
                Err(_) => {
                    tracing::warn!("automation controller did not stop within shutdown bound");
                }
            }
            shutdown_tunnel.shutdown().await;
            shutdown_supervisor.shutdown_all().await;
            let tunnel_terminal = !matches!(
                shutdown_tunnel.status().await.state,
                mcp_studio::tunnel::TunnelState::Starting
                    | mcp_studio::tunnel::TunnelState::Running
                    | mcp_studio::tunnel::TunnelState::Stopping
            );
            let mcp_terminal = shutdown_supervisor.list().await.iter().all(|status| {
                !matches!(
                    status.state,
                    mcp_studio::supervisor::ProcessState::Starting
                        | mcp_studio::supervisor::ProcessState::Running
                        | mcp_studio::supervisor::ProcessState::Stopping
                )
            });
            if let Err(error) = shutdown_history.close_run(tunnel_terminal && mcp_terminal) {
                tracing::warn!(history_error = %error, "could not persist Studio run closure");
            }
            shutdown_history.shutdown();
        })
        .await?;
    Ok(())
}

async fn ingest_gateway_history(history: HistoryHandle, control: GatewayControlClient) {
    let mut instance_id = None::<String>;
    let mut after_sequence = 0_u64;
    loop {
        match control.history_after(after_sequence).await {
            Ok((next_instance_id, _batch)) if instance_id.as_deref() != Some(&next_instance_id) => {
                instance_id = Some(next_instance_id);
                after_sequence = 0;
            }
            Ok((next_instance_id, batch)) => {
                if batch.events.is_empty() {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
                let ingest_history = history.clone();
                let events = batch.events;
                let result = tokio::task::spawn_blocking(move || {
                    ingest_history.observe_gateway_events(next_instance_id, events)
                })
                .await;
                match result {
                    Ok(Ok(sequence)) => after_sequence = sequence,
                    Ok(Err(error)) => {
                        tracing::warn!(history_error = %error, "Gateway history ingestion failed; live Gateway outcomes are unchanged");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Gateway history ingestion worker failed; live Gateway outcomes are unchanged");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
            Err(error) => {
                tracing::debug!(gateway_history_error = %error, "Gateway history source unavailable");
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        }
    }
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        if let Ok(mut sigterm) = signal(SignalKind::terminate()) {
            sigterm.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received; stopping managed tunnel and MCP processes");
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn history_subcommands_preserve_config_option() {
        let cli = Cli::try_parse_from([
            "mcp-studio",
            "--config",
            "runtime/studio/studio.toml",
            "history",
            "verify",
        ])
        .unwrap();
        assert_eq!(
            cli.config.as_deref(),
            Some(std::path::Path::new("runtime/studio/studio.toml"))
        );
        assert!(matches!(
            cli.command,
            Some(Command::History {
                command: HistoryCommand::Verify
            })
        ));
    }
}
