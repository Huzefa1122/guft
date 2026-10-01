//! Invite wire format. Public data only: the outer-layer secret is useless
//! without the separately delivered one-time code.

use data_encoding::BASE64URL_NOPAD;
use libsignal_protocol::{kem, DeviceId, IdentityKey, PreKeyBundle, PublicKey};
use rand::RngCore;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::limits::MAX_INVITE_CHARS;
use crate::payload::{check_display_name, check_onion};

const PREFIX: &str = "nochat1:";

#[derive(Serialize, Deserialize)]
pub struct InviteV1 {
    pub invite_id: [u8; 16],
    pub secret: [u8; 32],
    pub expires_at: u64,
    pub onion: String,
    pub name: String,
    pub reg: u32,
    pub device: u8,
    pub prekey_id: u32,
    pub prekey: Vec<u8>,
    pub signed_id: u32,
    pub signed: Vec<u8>,
    pub signed_sig: Vec<u8>,
    pub kyber_id: u32,
    pub kyber: Vec<u8>,
    pub kyber_sig: Vec<u8>,
    pub identity: Vec<u8>,
}

impl InviteV1 {
    pub fn encode(&self) -> Result<String> {
        Ok(format!("{PREFIX}{}", BASE64URL_NOPAD.encode(&postcard::to_allocvec(self)?)))
    }

    pub fn decode(text: &str) -> Result<Self> {
        if text.len() > MAX_INVITE_CHARS {
            return Err(Error::Limit("invite too long"));
        }
        let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
        let body = compact.strip_prefix(PREFIX).ok_or(Error::Invalid("not a nochat invite"))?;
        let bytes = BASE64URL_NOPAD.decode(body.as_bytes()).map_err(|_| Error::Invalid("not a nochat invite"))?;
        let inv: InviteV1 = postcard::from_bytes(&bytes)?;
        check_onion(&inv.onion)?;
        check_display_name(&inv.name)?;
        Ok(inv)
    }

    pub fn identity_key(&self) -> Result<IdentityKey> {
        Ok(IdentityKey::decode(&self.identity)?)
    }

    pub fn bundle(&self) -> Result<PreKeyBundle> {
        let device = DeviceId::new(self.device).map_err(|_| Error::Invalid("bad device id"))?;
        Ok(PreKeyBundle::new(
            self.reg,
            device,
            Some((self.prekey_id.into(), PublicKey::deserialize(&self.prekey).map_err(|_| Error::Invalid("bad prekey"))?)),
            self.signed_id.into(),
            PublicKey::deserialize(&self.signed).map_err(|_| Error::Invalid("bad signed prekey"))?,
            self.signed_sig.clone(),
            self.kyber_id.into(),
            kem::PublicKey::deserialize(&self.kyber)?,
            self.kyber_sig.clone(),
            self.identity_key()?,
        )?)
    }
}

/// 12 characters (60 bits) of lowercase base32, as `xxxx-xxxx-xxxx`.
pub fn generate_code(rng: &mut impl RngCore) -> String {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut s = String::new();
    for i in 0..12 {
        if i > 0 && i % 4 == 0 {
            s.push('-');
        }
        let mut b = [0u8; 1];
        rng.fill_bytes(&mut b);
        // 256 is a multiple of 32, so masking is unbiased.
        s.push(ALPHABET[(b[0] & 31) as usize] as char);
    }
    s
}
