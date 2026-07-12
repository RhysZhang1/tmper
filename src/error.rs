use thiserror::Error;

#[derive(Error, Debug)]
#[allow(dead_code)]
pub enum AppError {
    #[error("Audio error: {0}")]
    Audio(String),

    #[error("Metadata error: {0}")]
    Metadata(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Config error: {0}")]
    Config(String),

    #[error("Lyrics error: {0}")]
    Lyrics(String),
}

#[allow(dead_code)]
pub type AppResult<T> = anyhow::Result<T>;
