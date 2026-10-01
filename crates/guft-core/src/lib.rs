//! guft-core: end-to-end encryption, encrypted storage and wire format.
//!
//! All message encryption is delegated to Signal's `libsignal-protocol`
//! (PQXDH with ML-KEM-1024 plus the SPQR post-quantum ratchet). This crate only
//! adds persistence, invites, padding and strict input limits around it.
#![forbid(unsafe_code)]

pub mod engine;
pub mod error;
pub mod invite;
pub mod limits;
pub mod outer;
pub mod padding;
pub mod payload;
pub mod rooms;
pub mod spool;
mod store;
pub mod vault;

pub use engine::{ContactId, Engine, Received};
pub use error::{Error, Result};
pub use payload::Payload;
pub use store::RoomKind;
