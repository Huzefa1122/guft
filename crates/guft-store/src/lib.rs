//! guft-store: encrypted chat history (SQLCipher) and the profile lifecycle
//! (create / unlock / save / lock).
#![forbid(unsafe_code)]

pub mod history;
pub mod locker;
pub mod profile;

pub use history::{Body, ChatSummary, History, OutboxItem, Status, StoredMessage};
pub use locker::Locker;
pub use profile::Profile;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Core(#[from] guft_core::Error),
    #[error("database error")]
    Db,
    #[error("io error: {0}")]
    Io(String),
    #[error("a profile already exists here")]
    Exists,
    #[error("no profile found")]
    Missing,
    #[error("locked")]
    Locked,
    #[error("too many attempts; wait {0} seconds")]
    Backoff(u64),
}

impl From<rusqlite::Error> for Error {
    fn from(_: rusqlite::Error) -> Self {
        // Never leak SQL text or row contents in errors.
        Error::Db
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.kind().to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
