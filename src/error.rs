use thiserror::Error;

#[derive(Debug, Error)]
pub enum StudioError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("TOML decode error: {0}")]
    Toml(#[from] toml::de::Error),
}

pub type StudioResult<T> = Result<T, StudioError>;
