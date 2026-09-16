use thiserror::Error;

#[derive(Debug, Error)]
pub enum StudioError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("MCP server not found: {0}")]
    NotFound(String),

    #[error("duplicate MCP registration: {0}")]
    Duplicate(String),

    #[error("MCP server is disabled: {0}")]
    Disabled(String),

    #[error("MCP registry mutation conflicts with runtime state: {0}")]
    Conflict(String),

    #[error("unsupported MCP project: {0}")]
    Unsupported(String),

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

    #[error("TOML encode error: {0}")]
    TomlEncode(#[from] toml::ser::Error),

    #[error("JSON decode error: {0}")]
    Json(#[from] serde_json::Error),
}

pub type StudioResult<T> = Result<T, StudioError>;