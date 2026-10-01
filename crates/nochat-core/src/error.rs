use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

/// Errors never carry secrets or plaintext.
#[derive(Debug, Error)]
pub enum Error {
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("invalid input: {0}")]
    Invalid(&'static str),
    #[error("limit exceeded: {0}")]
    Limit(&'static str),
    #[error("wrong passphrase or corrupted data")]
    Vault,
    #[error("unknown contact")]
    UnknownContact,
    #[error("contact identity key changed; verify the safety number before continuing")]
    IdentityChanged,
}

impl From<libsignal_protocol::SignalProtocolError> for Error {
    fn from(e: libsignal_protocol::SignalProtocolError) -> Self {
        match e {
            libsignal_protocol::SignalProtocolError::UntrustedIdentity(_) => Error::IdentityChanged,
            other => Error::Protocol(other.to_string()),
        }
    }
}

impl From<postcard::Error> for Error {
    fn from(_: postcard::Error) -> Self {
        Error::Invalid("malformed encoding")
    }
}
