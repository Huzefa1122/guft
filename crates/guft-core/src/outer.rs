//! Outer encryption layer.
//!
//! Keys here come only from a secret handed over with the invite plus the
//! one-time code, both symmetric and independent of every public-key
//! operation. A flaw in libsignal or a break of X25519/ML-KEM therefore does
//! not expose messages protected by this layer. Each direction is a hash chain:
//! one fresh key per message, and old keys are erased as the chain advances.

use std::collections::BTreeMap;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::error::{Error, Result};
use crate::vault;

/// Most message keys we will skip ahead (and remember) for lost or reordered frames.
const MAX_SKIP: u64 = 256;
const TAG: usize = 16;

type Key32 = Zeroizing<[u8; 32]>;

#[derive(Clone, Serialize, Deserialize)]
struct Chain {
    key: Key32,
    index: u64,
    skipped: BTreeMap<u64, Zeroizing<Vec<u8>>>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OuterState {
    send: Chain,
    recv: Chain,
}

/// Normalise a typed code: lowercase, drop separators and spaces.
pub fn normalize_code(code: &str) -> String {
    code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect()
}

/// Combine the invite secret with the stretched one-time code.
pub fn derive_root(secret: &[u8; 32], code: &str) -> Result<Key32> {
    let code = normalize_code(code);
    if code.len() < 12 {
        return Err(Error::Invalid("code too short"));
    }
    let stretched = vault::stretch(code.as_bytes(), secret, 32 * 1024, 3)?;
    let mut root = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(Some(secret), &*stretched)
        .expand(b"guft outer root v1", &mut *root)
        .map_err(|_| Error::Vault)?;
    Ok(root)
}

fn chain_start(root: &Key32, label: &[u8]) -> Chain {
    let mut key = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::from_prk(&**root)
        .expect("32-byte PRK")
        .expand(label, &mut *key)
        .expect("32 bytes");
    Chain { key, index: 0, skipped: BTreeMap::new() }
}

/// Returns the message material (32-byte key || 24-byte nonce) and advances.
fn step(chain: &mut Chain) -> Zeroizing<Vec<u8>> {
    let hk = Hkdf::<Sha256>::from_prk(&*chain.key).expect("32-byte PRK");
    let mut material = Zeroizing::new(vec![0u8; 56]);
    hk.expand(b"msg", &mut material).expect("56 bytes");
    let mut next = Zeroizing::new([0u8; 32]);
    hk.expand(b"chain", &mut *next).expect("32 bytes");
    chain.key = next;
    chain.index += 1;
    material
}

fn cipher(material: &[u8]) -> (XChaCha20Poly1305, XNonce) {
    let c = XChaCha20Poly1305::new_from_slice(&material[..32]).expect("32-byte key");
    let nonce: [u8; 24] = material[32..56].try_into().expect("24-byte nonce");
    (c, XNonce::from(nonce))
}

impl OuterState {
    /// `initiator` is the side that imported the invite.
    pub fn new(root: &Key32, initiator: bool) -> Self {
        let (i2r, r2i) = (chain_start(root, b"i2r"), chain_start(root, b"r2i"));
        if initiator {
            Self { send: i2r, recv: r2i }
        } else {
            Self { send: r2i, recv: i2r }
        }
    }

    /// Output: `index (8 bytes BE) | ciphertext`. `aad` binds the frame header.
    pub fn seal(&mut self, aad: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
        let index = self.send.index;
        let material = step(&mut self.send);
        let (c, nonce) = cipher(&material);
        let mut full_aad = aad.to_vec();
        full_aad.extend_from_slice(&index.to_be_bytes());
        let ct = c.encrypt(&nonce, Payload { msg: plaintext, aad: &full_aad }).map_err(|_| Error::Vault)?;
        let mut out = index.to_be_bytes().to_vec();
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// State changes only if the frame authenticates.
    pub fn open(&mut self, aad: &[u8], frame: &[u8]) -> Result<Vec<u8>> {
        if frame.len() < 8 + TAG {
            return Err(Error::Invalid("short frame"));
        }
        let index = u64::from_be_bytes(frame[..8].try_into().unwrap());
        let mut next = self.recv.clone();
        let material = if index < next.index {
            next.skipped.remove(&index).ok_or(Error::Invalid("replayed or too old"))?
        } else {
            if index - next.index > MAX_SKIP {
                return Err(Error::Invalid("too far ahead"));
            }
            while next.index < index {
                let m = step(&mut next);
                let at = next.index - 1;
                next.skipped.insert(at, m);
            }
            step(&mut next)
        };
        let (c, nonce) = cipher(&material);
        let mut full_aad = aad.to_vec();
        full_aad.extend_from_slice(&index.to_be_bytes());
        let pt = c.decrypt(&nonce, Payload { msg: &frame[8..], aad: &full_aad }).map_err(|_| Error::Vault)?;
        while next.skipped.len() as u64 > MAX_SKIP {
            let first = *next.skipped.keys().next().unwrap();
            next.skipped.remove(&first);
        }
        self.recv = next;
        Ok(pt)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (OuterState, OuterState) {
        let root = Zeroizing::new([5u8; 32]);
        (OuterState::new(&root, true), OuterState::new(&root, false))
    }

    #[test]
    fn roundtrip_both_directions() {
        let (mut a, mut b) = pair();
        let f = a.seal(b"h", b"hi").unwrap();
        assert_eq!(b.open(b"h", &f).unwrap(), b"hi");
        let g = b.seal(b"h", b"yo").unwrap();
        assert_eq!(a.open(b"h", &g).unwrap(), b"yo");
    }

    #[test]
    fn replay_tamper_and_wrong_aad_rejected() {
        let (mut a, mut b) = pair();
        let f = a.seal(b"h", b"one").unwrap();
        assert!(b.open(b"x", &f).is_err());
        let mut bad = f.clone();
        *bad.last_mut().unwrap() ^= 1;
        assert!(b.open(b"h", &bad).is_err());
        assert_eq!(b.open(b"h", &f).unwrap(), b"one");
        assert!(b.open(b"h", &f).is_err(), "replay");
    }

    #[test]
    fn out_of_order_within_window() {
        let (mut a, mut b) = pair();
        let f: Vec<_> = (0..4).map(|i| a.seal(b"h", &[i]).unwrap()).collect();
        assert_eq!(b.open(b"h", &f[2]).unwrap(), [2]);
        assert_eq!(b.open(b"h", &f[0]).unwrap(), [0]);
        assert_eq!(b.open(b"h", &f[3]).unwrap(), [3]);
        assert_eq!(b.open(b"h", &f[1]).unwrap(), [1]);
    }

    #[test]
    fn far_ahead_rejected_and_state_kept() {
        let (mut a, mut b) = pair();
        let mut f = a.seal(b"h", b"ok").unwrap();
        let ok = f.clone();
        f[..8].copy_from_slice(&(MAX_SKIP + 5).to_be_bytes());
        assert!(b.open(b"h", &f).is_err());
        assert_eq!(b.open(b"h", &ok).unwrap(), b"ok");
    }

    #[test]
    fn wrong_code_gives_different_root() {
        let s = [3u8; 32];
        let a = derive_root(&s, "abcd-efgh-ijkl").unwrap();
        let b = derive_root(&s, "ABCD EFGH IJKL").unwrap();
        let c = derive_root(&s, "abcd-efgh-ijkm").unwrap();
        assert_eq!(*a, *b);
        assert_ne!(*a, *c);
        assert!(derive_root(&s, "short").is_err());
    }
}
