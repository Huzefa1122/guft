//! Encryption at rest: Argon2id passphrase stretching + XChaCha20-Poly1305.
//!
//! File layout: `"NCV2" | m_kib u32 | t u32 | salt[16] | nonce[24] | ciphertext`.
//! The whole header is authenticated as AAD. One Argon2 run yields a master key;
//! purpose-specific subkeys (state file, history database) come from HKDF.

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rand::{RngCore, TryRngCore};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::error::{Error, Result};

const MAGIC: &[u8; 4] = b"NCV2";
/// 64 MiB, 3 passes, 1 lane.
const M_KIB: u32 = 64 * 1024;
const T_COST: u32 = 3;
/// Upper bounds for parameters read from a (possibly tampered) file.
const MAX_M_KIB: u32 = 512 * 1024;
const MAX_T_COST: u32 = 12;

/// Stretch a secret into a 32-byte key. Also used for invite codes.
pub fn stretch(secret: &[u8], salt: &[u8], m_kib: u32, t_cost: u32) -> Result<Zeroizing<[u8; 32]>> {
    if m_kib > MAX_M_KIB || t_cost > MAX_T_COST || salt.len() < 8 {
        return Err(Error::Vault);
    }
    let params = Params::new(m_kib, t_cost, 1, Some(32)).map_err(|_| Error::Vault)?;
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(secret, salt, &mut *out)
        .map_err(|_| Error::Vault)?;
    Ok(out)
}

/// Shortest passphrase we accept.
pub const MIN_PASSPHRASE_CHARS: usize = 8;

const HEADER: usize = 4 + 4 + 4 + 16;

/// The unlocked vault: header plus the master key. Dropping it wipes the key,
/// which is what "locking" means.
pub struct Vault {
    header: [u8; HEADER],
    master: Zeroizing<[u8; 32]>,
}

impl Vault {
    /// A new vault with a fresh random salt.
    pub fn create(passphrase: &str) -> Result<Self> {
        if passphrase.chars().count() < MIN_PASSPHRASE_CHARS {
            return Err(Error::Invalid("passphrase too short"));
        }
        let mut salt = [0u8; 16];
        rand::rngs::OsRng.unwrap_err().fill_bytes(&mut salt);
        let mut header = [0u8; HEADER];
        header[..4].copy_from_slice(MAGIC);
        header[4..8].copy_from_slice(&M_KIB.to_be_bytes());
        header[8..12].copy_from_slice(&T_COST.to_be_bytes());
        header[12..].copy_from_slice(&salt);
        let master = stretch(passphrase.as_bytes(), &salt, M_KIB, T_COST)?;
        Ok(Self { header, master })
    }

    /// Unlock from a sealed file; returns the vault and the decrypted contents.
    pub fn unlock(passphrase: &str, file: &[u8]) -> Result<(Self, Zeroizing<Vec<u8>>)> {
        if file.len() < HEADER + 24 + 16 || &file[..4] != MAGIC {
            return Err(Error::Vault);
        }
        let m = u32::from_be_bytes(file[4..8].try_into().unwrap());
        let t = u32::from_be_bytes(file[8..12].try_into().unwrap());
        let master = stretch(passphrase.as_bytes(), &file[12..HEADER], m, t)?;
        let mut header = [0u8; HEADER];
        header.copy_from_slice(&file[..HEADER]);
        let vault = Self { header, master };
        let nonce: [u8; 24] = file[HEADER..HEADER + 24].try_into().unwrap();
        let key = vault.subkey("state");
        let cipher = XChaCha20Poly1305::new_from_slice(&*key).map_err(|_| Error::Vault)?;
        let plain = cipher
            .decrypt(&XNonce::from(nonce), Payload { msg: &file[HEADER + 24..], aad: &vault.header })
            .map_err(|_| Error::Vault)?;
        Ok((vault, Zeroizing::new(plain)))
    }

    /// Encrypt `plaintext` for storage, with a fresh random nonce each time.
    pub fn seal(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        let mut nonce = [0u8; 24];
        rand::rngs::OsRng.unwrap_err().fill_bytes(&mut nonce);
        let key = self.subkey("state");
        let cipher = XChaCha20Poly1305::new_from_slice(&*key).map_err(|_| Error::Vault)?;
        let ct = cipher
            .encrypt(&XNonce::from(nonce), Payload { msg: plaintext, aad: &self.header })
            .map_err(|_| Error::Vault)?;
        let mut out = Vec::with_capacity(HEADER + 24 + ct.len());
        out.extend_from_slice(&self.header);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// An independent key for one purpose ("state", "history", ...).
    pub fn subkey(&self, label: &str) -> Zeroizing<[u8; 32]> {
        let mut out = Zeroizing::new([0u8; 32]);
        Hkdf::<Sha256>::new(None, &*self.master)
            .expand(format!("nochat vault subkey v1:{label}").as_bytes(), &mut *out)
            .expect("32 bytes");
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_wrong_pass_and_tamper() {
        let v = Vault::create("correct horse").unwrap();
        let blob = v.seal(b"secret state").unwrap();
        let (v2, plain) = Vault::unlock("correct horse", &blob).unwrap();
        assert_eq!(&*plain, b"secret state");
        assert_eq!(*v.subkey("history"), *v2.subkey("history"));
        assert_ne!(*v.subkey("history"), *v.subkey("state"));
        assert!(Vault::unlock("wrong horse!", &blob).is_err());
        for i in [0, 5, 20, 40, 60, blob.len() - 1] {
            let mut bad = blob.clone();
            bad[i] ^= 1;
            assert!(Vault::unlock("correct horse", &bad).is_err(), "byte {i}");
        }
        assert!(Vault::unlock("correct horse", &blob[..10]).is_err());
    }

    #[test]
    fn fresh_nonce_each_seal_and_short_passphrase_rejected() {
        let v = Vault::create("long enough pass").unwrap();
        assert_ne!(v.seal(b"x").unwrap(), v.seal(b"x").unwrap());
        assert!(Vault::create("short").is_err());
    }

    #[test]
    fn rejects_huge_kdf_params() {
        let v = Vault::create("long enough pass").unwrap();
        let mut blob = v.seal(b"d").unwrap();
        blob[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(Vault::unlock("long enough pass", &blob).is_err());
    }
}
