use std::{net::SocketAddr, sync::Arc, time::Duration};

use anyhow::Result;
use clap::Parser;
use mcp_studio::{
    api::{self, AppState},
    config::StudioConfig,
    logging,
    registry::Registry,
    supervisor::Supervisor,
};

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

    let config = match cli.config {
        Some(path) => StudioConfig::load(&path)?,
        None => StudioConfig::default(),
    };
    config.validate()?;

    let base_dir = std::env::current_dir()?;
    let supervisor = Arc::new(Supervisor::new(
        Registry::new(config.mcp.clone()),
        config.log_capacity,
        Duration::from_millis(config.stop_timeout_ms),
        base_dir,
    ));

    let addr: SocketAddr = config.server.listen_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(
        listen_addr = %addr,
        managed_mcp_count = config.mcp.len(),
        "starting MCP Studio core supervisor"
    );

    let shutdown_supervisor = supervisor.clone();
    let app = api::router(AppState { supervisor });
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            shutdown_signal().await;
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

    tracing::info!("shutdown signal received; stopping managed MCP processes");
}
