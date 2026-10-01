//! guft-app: the application logic that ties the engine, encrypted history,
//! session lock and network together. The UI layer only calls [`App`] and
//! listens for [`Event`]s; it never sees keys.
#![forbid(unsafe_code)]

mod app;
mod backend;
mod temp;
mod tor_backend;

pub use app::{App, AppOptions, ChatView, ContactView, MemberView, RoomView};
pub use backend::{NetworkBackend, Running};
pub use tor_backend::TorBackend;
#[cfg(feature = "mem")]
pub use backend::MemBackend;
pub use guft_store::{Body, Status, StoredMessage};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error(transparent)]
    Core(#[from] guft_core::Error),
    #[error(transparent)]
    Store(guft_store::Error),
    #[error(transparent)]
    Net(#[from] guft_net::Error),
    #[error("locked")]
    Locked,
    #[error("io error: {0}")]
    Io(String),
    #[error("background task failed")]
    Task,
    #[error("network is not running")]
    Offline,
}

impl From<guft_store::Error> for AppError {
    fn from(e: guft_store::Error) -> Self {
        match e {
            guft_store::Error::Locked => AppError::Locked,
            other => AppError::Store(other),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        AppError::Io(e.kind().to_string())
    }
}

impl From<tokio::task::JoinError> for AppError {
    fn from(_: tokio::task::JoinError) -> Self {
        AppError::Task
    }
}

pub type Result<T> = std::result::Result<T, AppError>;

/// Things the UI should react to. Events carry ids, never message contents.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Unlocked,
    Locked,
    NetworkReady { onion: String },
    NetworkError(String),
    ContactAdded { id: String, name: String },
    Message { chat: String, id: i64 },
    Delivered { chat: String, msg_id: i64 },
    SendFailed { chat: String, msg_id: i64 },
    /// A room appeared, or its members or name changed.
    RoomChanged { room: String },
    RoomRemoved { room: String },
}
