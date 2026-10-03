use thiserror::Error;

#[derive(Error, Debug)]
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

    /// Talking to (or starting) the daemon: connect refused, handshake
    /// mismatch, a socket that answers with nonsense.
    #[error("IPC error: {0}")]
    Ipc(String),
}

pub type AppResult<T> = anyhow::Result<T>;
