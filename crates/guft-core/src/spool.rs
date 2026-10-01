//! A sealed box for frames received while the vault is locked.
//!
//! The public half stays in memory after locking; only the vault can open
//! entries again. Hybrid construction, like the handshake: ML-KEM-1024 +
//! X25519 feed HKDF, and the result keys XChaCha20-Poly1305. Nothing readable
//! (not even the sender hint in the frame header) is stored while locked.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use libsignal_protocol::{kem, KeyPair, PrivateKey, PublicKey};
use rand::rngs::OsRng;
use rand::{RngCore, TryRngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::error::{Error, Result};
use crate::limits::MAX_WIRE_BYTES;

const VERSION: u8 = 1;
const X_PUB: usize = 33;
const KEM_CT: usize = 1569;
const NONCE: usize = 24;
const TAG: usize = 16;
const HEAD: usize = 1 + X_PUB + KEM_CT + NONCE;

/// Secret half; lives in the vault state.
#[derive(Clone, Serialize, Deserialize)]
pub struct SpoolSecret {
    kem: Zeroizing<Vec<u8>>,
    x: Zeroizing<Vec<u8>>,
    public: SpoolPublic,
}

/// Public half; safe to keep in memory while locked.
#[derive(Clone, Serialize, Deserialize)]
pub struct SpoolPublic {
    kem: Vec<u8>,
    x: Vec<u8>,
}

impl SpoolSecret {
    pub fn generate() -> Self {
        let mut r = OsRng.unwrap_err();
        let kem = kem::KeyPair::generate(kem::KeyType::Kyber1024, &mut r);
        let x = KeyPair::generate(&mut r);
        Self {
            kem: Zeroizing::new(kem.secret_key.serialize().to_vec()),
            x: Zeroizing::new(x.private_key.serialize()),
            public: SpoolPublic { kem: kem.public_key.serialize().to_vec(), x: x.public_key.serialize().to_vec() },
        }
    }

    pub fn public(&self) -> SpoolPublic {
        self.public.clone()
    }

    pub fn open(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        let bad = || Error::Invalid("bad spool entry");
        if blob.len() < HEAD + TAG + 1 || blob.len() > HEAD + MAX_WIRE_BYTES + TAG || blob[0] != VERSION {
            return Err(bad());
        }
        let (head, ct) = blob.split_at(HEAD);
        let eph_pub = &head[1..1 + X_PUB];
        let kem_ct: Box<[u8]> = head[1 + X_PUB..1 + X_PUB + KEM_CT].into();
        let nonce: [u8; NONCE] = head[1 + X_PUB + KEM_CT..].try_into().map_err(|_| bad())?;

        let ss_kem = kem::SecretKey::try_from(&self.kem[..]).map_err(|_| bad())?.decapsulate(&kem_ct).map_err(|_| bad())?;
        let x_secret = PrivateKey::deserialize(&self.x).map_err(|_| bad())?;
        let ss_x = x_secret.calculate_agreement(&PublicKey::deserialize(eph_pub).map_err(|_| bad())?).map_err(|_| bad())?;
        let key = derive(&ss_kem, &ss_x, &head[1..1 + X_PUB + KEM_CT]);
        let cipher = XChaCha20Poly1305::new_from_slice(&*key).map_err(|_| bad())?;
        let pt = cipher
            .decrypt(&XNonce::from(nonce), Payload { msg: ct, aad: &head[..1 + X_PUB + KEM_CT] })
            .map_err(|_| bad())?;
        Ok(Zeroizing::new(pt))
    }
}

fn derive(ss_kem: &[u8], ss_x: &[u8], context: &[u8]) -> Zeroizing<[u8; 32]> {
    let mut ikm = Zeroizing::new(Vec::with_capacity(ss_kem.len() + ss_x.len()));
    ikm.extend_from_slice(ss_kem);
    ikm.extend_from_slice(ss_x);
    let mut key = Zeroizing::new([0u8; 32]);
    Hkdf::<Sha256>::new(Some(context), &ikm).expand(b"guft spool v1", &mut *key).expect("32 bytes");
    key
}

impl SpoolPublic {
    pub fn seal(&self, frame: &[u8]) -> Result<Vec<u8>> {
        if frame.is_empty() || frame.len() > MAX_WIRE_BYTES {
            return Err(Error::Limit("frame size"));
        }
        let bad = || Error::Invalid("bad spool key");
        let mut r = OsRng.unwrap_err();
        let kem_pub = kem::PublicKey::deserialize(&self.kem).map_err(|_| bad())?;
        let x_pub = PublicKey::deserialize(&self.x).map_err(|_| bad())?;
        let (ss_kem, kem_ct) = kem_pub.encapsulate(&mut r).map_err(|_| bad())?;
        let eph = KeyPair::generate(&mut r);
        let ss_x = eph.private_key.calculate_agreement(&x_pub).map_err(|_| bad())?;

        let mut out = Vec::with_capacity(HEAD + frame.len() + TAG);
        out.push(VERSION);
        out.extend_from_slice(&eph.public_key.serialize());
        out.extend_from_slice(&kem_ct);
        let key = derive(&ss_kem, &ss_x, &out[1..]);
        let mut nonce = [0u8; NONCE];
        r.fill_bytes(&mut nonce);
        let cipher = XChaCha20Poly1305::new_from_slice(&*key).map_err(|_| bad())?;
        let aad = out.clone();
        let ct = cipher.encrypt(&XNonce::from(nonce), Payload { msg: frame, aad: &aad }).map_err(|_| bad())?;
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_wrong_key_tamper_and_size() {
        let (a, b) = (SpoolSecret::generate(), SpoolSecret::generate());
        let frame = vec![9u8; 5000];
        let sealed = a.public().seal(&frame).unwrap();
        assert_eq!(&*a.open(&sealed).unwrap(), &frame[..]);
        assert!(b.open(&sealed).is_err(), "another key must not open it");
        assert_ne!(sealed, a.public().seal(&frame).unwrap(), "fresh randomness each time");
        assert!(!sealed.windows(64).any(|w| w == &frame[..64]), "plaintext must not appear");
        for i in [0, 1, 40, HEAD - 1, HEAD, sealed.len() - 1] {
            let mut bad = sealed.clone();
            bad[i] ^= 1;
            assert!(a.open(&bad).is_err(), "byte {i}");
        }
        assert!(a.open(&sealed[..HEAD]).is_err());
        assert!(a.public().seal(&[]).is_err());
        assert!(a.public().seal(&vec![0; MAX_WIRE_BYTES + 1]).is_err());
        assert!(a.public().seal(&vec![1; MAX_WIRE_BYTES]).is_ok());
    }
}
