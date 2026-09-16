use thiserror::Error;

#[derive(Debug, Error)]
pub enum StudioError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("MCP server not found: {0}")]
    NotFound(String),

    #[error("MCP server is already running: {0}")]
    AlreadyRunning(String),

    #[error("MCP server is not running: {0}")]
    NotRunning(String),

    #[error("process error: {0}")]
    Process(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML decode error: {0}")]
    Toml(#[from] toml::de::Error),
}

pub type StudioResult<T> = Result<T, StudioError>;
