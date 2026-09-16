use std::net::SocketAddr;

use anyhow::Result;
use clap::Parser;
use mcp_studio::{api, config::StudioConfig, logging};

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

    let addr: SocketAddr = config.server.listen_addr.parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!(listen_addr = %addr, "starting MCP Studio foundation service");

    axum::serve(listener, api::router())
        .with_graceful_shutdown(shutdown_signal())
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

    tracing::info!("shutdown signal received");
}
