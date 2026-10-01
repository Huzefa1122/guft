//! guft-net: moves opaque encrypted frames between guft apps over Tor.
//!
//! Frames are cut into fixed-size cells, spread across several isolated Tor
//! circuits, padded with cover traffic, and acknowledged end to end. This crate
//! never sees plaintext or keys: it only handles bytes the core already encrypted.
#![forbid(unsafe_code)]

pub mod access;
pub mod cell;
pub mod link;
#[cfg(feature = "mem")]
pub mod mem;
pub mod reassembly;
pub mod tor;

pub use link::{Connector, Inbound, Net, NetConfig, Receiver, Transport};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io: {0}")]
    Io(String),
    #[error("protocol violation: {0}")]
    Protocol(&'static str),
    #[error("not enough circuits ({have} of {need})")]
    NotEnoughCircuits { have: usize, need: usize },
    #[error("timed out waiting for acknowledgement")]
    AckTimeout,
    #[error("frame too large")]
    TooLarge,
    #[error("tor: {0}")]
    Tor(String),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io(e.kind().to_string())
    }
}

pub type Result<T> = std::result::Result<T, Error>;
