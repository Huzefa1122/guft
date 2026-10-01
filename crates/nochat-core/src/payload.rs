//! The plaintext carried inside each encrypted message.

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::limits::*;
use crate::padding;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Payload {
    /// Sent first, so the recipient learns where to reply.
    Hello { onion: String, name: String },
    Text(String),
    File { name: String, data: Vec<u8> },
}

impl Payload {
    pub fn validate(&self) -> Result<()> {
        match self {
            Payload::Hello { onion, name } => {
                check_onion(onion)?;
                check_display_name(name)
            }
            Payload::Text(t) => {
                if t.len() > MAX_TEXT_BYTES {
                    return Err(Error::Limit("text too long"));
                }
                Ok(())
            }
            Payload::File { name, data } => {
                if data.len() > MAX_FILE_BYTES {
                    return Err(Error::Limit("file too large"));
                }
                check_file_name(name)
            }
        }
    }

    /// Serialize and pad to a bucket size, ready for encryption.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        padding::pad(&postcard::to_allocvec(self)?)
    }

    /// Inverse of [`encode`]; validates everything the sender claims.
    pub fn decode(padded: Vec<u8>) -> Result<Self> {
        let raw = padding::unpad(padded)?;
        let p: Payload = postcard::from_bytes(&raw)?;
        p.validate()?;
        Ok(p)
    }
}

pub fn check_display_name(name: &str) -> Result<()> {
    if name.len() > MAX_DISPLAY_NAME_BYTES || name.chars().any(|c| c.is_control()) {
        return Err(Error::Invalid("bad display name"));
    }
    Ok(())
}

/// A v3 onion address: 56 base32 chars + ".onion".
pub fn check_onion(onion: &str) -> Result<()> {
    let host = onion.strip_suffix(".onion").ok_or(Error::Invalid("bad onion address"))?;
    let ok = host.len() == 56 && host.bytes().all(|b| matches!(b, b'a'..=b'z' | b'2'..=b'7'));
    if ok { Ok(()) } else { Err(Error::Invalid("bad onion address")) }
}

/// File names come from the network: no paths, no control chars, no hidden or
/// reserved names, so a hostile name can never escape the download directory.
pub fn check_file_name(name: &str) -> Result<()> {
    let bad = name.is_empty()
        || name.len() > MAX_FILE_NAME_BYTES
        || name.starts_with('.')
        || name.ends_with('.')
        || name.ends_with(' ')
        || name.chars().any(|c| c.is_control() || matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0'));
    if bad { Err(Error::Invalid("bad file name")) } else { Ok(()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_rules() {
        for ok in ["a.txt", "photo 1.png"] {
            assert!(check_file_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", ".bashrc", "../x", "a/b", "a\\b", "x\0y", "con:", "trail.", "new\nline"] {
            assert!(check_file_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn roundtrip_and_limits() {
        let p = Payload::Text("hello".into());
        assert_eq!(Payload::decode(p.encode().unwrap()).unwrap(), p);
        let big = Payload::File { name: "a".into(), data: vec![0; MAX_FILE_BYTES + 1] };
        assert!(big.encode().is_err());
        let max = Payload::File { name: "a".into(), data: vec![9; MAX_FILE_BYTES] };
        assert_eq!(Payload::decode(max.encode().unwrap()).unwrap(), max);
    }

    #[test]
    fn onion_rules() {
        let good = format!("{}.onion", "a".repeat(56));
        assert!(check_onion(&good).is_ok());
        assert!(check_onion("short.onion").is_err());
        assert!(check_onion(&format!("{}.com", "a".repeat(56))).is_err());
        assert!(check_onion(&format!("{}.onion", "A".repeat(56))).is_err());
    }
}
