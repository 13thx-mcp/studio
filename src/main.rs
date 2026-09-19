use std::{
    net::SocketAddr,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::Result;
use clap::Parser;
use mcp_studio::{
    api::{self, AppState},
    config::{LoadedConfigIdentity, StudioConfig},
    discovery::DiscoveryService,
    logging,
    realtime::EventHub,
    registry::Registry,
    supervisor::Supervisor,
    tunnel::TunnelSupervisor,
    update::{
        ComponentCatalog, ComponentId, FleetUpdateManager, GatewayUpdateManager, HostPlatform,
        HostRuntimeRoots, InventoryService, McpUpdateManager, RuntimeReconciler, SelfUpdateManager,
        StudioProcessConfigIdentity, TunnelUpdateManager, Version,
        write_activation_ready_proof_from_env,
    },
};
use tokio::sync::Notify;

#[derive(Debug, Parser)]
#[command(version, about = "MCP Studio local control plane")]
struct Cli {
    /// Optional configuration file. If omitted, built-in safe defaults are used.
    #[arg(long, env = "MCP_STUDIO_CONFIG")]
    config: Option<std::path::PathBuf>,
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

    let base_dir = std::env::current_dir()?;
    let process_instance = format!(
        "{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let activation_config_identity = loaded_config_identity.clone();
    let studio_config_identity =
        StudioProcessConfigIdentity::from_loaded(loaded_config_identity, process_instance);
    let registry = Registry::open(&base_dir, &config.registry, &config.mcp)?;
    let supervisor = Arc::new(Supervisor::new(
        registry.clone(),
        config.log_capacity,
        Duration::from_millis(config.stop_timeout_ms),
    ));
    let discovery = Arc::new(DiscoveryService::new(registry));

    let (source_root, bin_root, runtime_root) = config.updates.resolve_roots(&base_dir);
    let activation_runtime_root = runtime_root.clone();
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

    let registry_events = EventHub::default();
    let tunnel = Arc::new(TunnelSupervisor::new(
        config.tunnel.clone(),
        config.log_capacity,
        Duration::from_millis(config.stop_timeout_ms),
        base_dir,
        EventHub::default(),
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
    let tunnel_updates = Arc::new(TunnelUpdateManager::new(
        catalog.clone(),
        tunnel.clone(),
        inventory.clone(),
        registry_events.clone(),
    )?);
    tunnel_updates.recover_startup().await?;
    let finalized_self_updates = self_updates.finalize_startup().await?;
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

    let addr: SocketAddr = config.server.listen_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    if write_activation_ready_proof_from_env(&activation_runtime_root, &activation_config_identity)?
    {
        tracing::info!("published process-bound Studio activation proof");
    }
    tracing::info!(
        listen_addr = %addr,
        managed_mcp_count = supervisor.registry().ids().len(),
        mcp_root = %supervisor.registry().canonical_root().display(),
        "starting MCP Studio"
    );

    let shutdown_supervisor = supervisor.clone();
    let shutdown_tunnel = tunnel.clone();
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
            shutdown_tunnel.shutdown().await;
            shutdown_supervisor.shutdown_all().await;
        })
        .await?;
    Ok(())
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
