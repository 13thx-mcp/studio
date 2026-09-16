use anyhow::{Result, anyhow};
use tracing_subscriber::{EnvFilter, fmt};

pub fn init() -> Result<()> {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("mcp_studio=info,tower_http=info"));

    fmt()
        .with_env_filter(filter)
        .json()
        .with_current_span(true)
        .with_span_list(true)
        .try_init()
        .map_err(|error| anyhow!("failed to initialize tracing subscriber: {error}"))?;

    Ok(())
}
