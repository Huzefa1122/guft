//! Persistent implementations of libsignal's store traits.
//!
//! Everything lives in one serializable [`State`]; the whole thing is sealed by
//! [`crate::vault`] before touching disk. Secrets are wrapped in `Zeroizing`.

use std::collections::{BTreeMap, BTreeSet};

use async_trait::async_trait;
use libsignal_protocol::*;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::outer::OuterState;

type Secret = Zeroizing<Vec<u8>>;
type SResult<T> = std::result::Result<T, SignalProtocolError>;

#[derive(Clone, Serialize, Deserialize)]
pub struct IdStore {
    pub pair: Secret,
    pub reg: u32,
    pub known: BTreeMap<String, Vec<u8>>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SessionStoreImpl(pub BTreeMap<String, Secret>);
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct PreKeys(pub BTreeMap<u32, Secret>);
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct SignedPreKeys(pub BTreeMap<u32, Secret>);
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct KyberPreKeys {
    pub keys: BTreeMap<u32, Secret>,
    pub used: BTreeSet<(u32, u32, Vec<u8>)>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Contact {
    pub name: String,
    pub onion: String,
    pub verified: bool,
    pub outer: OuterState,
    /// Tor client authorization key for reaching their onion service.
    pub connect_key: Option<Zeroizing<[u8; 32]>>,
    /// Set on the side that imported an invite until the other side replies.
    pub pending_invite: Option<[u8; 16]>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RoomMember {
    pub name: String,
    pub onion: String,
}

/// How long a room lives. Only `Normal` rooms are ever written to disk.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoomKind {
    Normal,
    /// A group room whose roster and messages exist in memory only.
    Temp,
    /// A one-to-one memory-only chat with a contact: nobody else can join.
    Direct,
}

/// A room is a roster; messages go to each member over our pairwise sessions.
#[derive(Clone, Serialize, Deserialize)]
pub struct Room {
    pub name: String,
    /// Contact id of whoever created it (our own id when we did).
    pub creator: String,
    pub open_invites: bool,
    /// Everyone but us.
    pub members: BTreeMap<String, RoomMember>,
    /// Who added us (introductions are relayed through them); `None` if we created it.
    pub invited_by: Option<String>,
}

/// An invite we issued and have not yet seen used.
#[derive(Clone, Serialize, Deserialize)]
pub struct InviteRec {
    pub label: String,
    pub invite_id: [u8; 16],
    pub expires_at: u64,
    pub root: Zeroizing<[u8; 32]>,
    pub kyber_id: u32,
    pub prekey_id: u32,
    pub consumed: bool,
    pub consumed_by: Option<String>,
    /// Room-bound invite: whoever uses it is added to this room.
    pub room: Option<[u8; 16]>,
    /// Only this identity may use the invite (introductions inside a room).
    pub expect: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct State {
    pub name: String,
    pub onion: Option<String>,
    /// Seed of the ed25519 key behind our onion address; never leaves the vault.
    pub onion_seed: Zeroizing<[u8; 32]>,
    pub spool: crate::spool::SpoolSecret,
    /// Keep the onion service up (spooling sealed frames) while the app is locked.
    pub online_when_locked: bool,
    pub ids: IdStore,
    pub sessions: SessionStoreImpl,
    pub prekeys: PreKeys,
    pub signed: SignedPreKeys,
    pub kyber: KyberPreKeys,
    pub contacts: BTreeMap<String, Contact>,
    /// Keyed by signed pre-key id.
    pub invites: BTreeMap<u32, InviteRec>,
    /// Keyed by room id in lowercase hex.
    pub rooms: BTreeMap<String, Room>,
}

fn secret<T>(v: SResult<T>, f: impl FnOnce(T) -> Vec<u8>) -> SResult<Secret> {
    v.map(|x| Zeroizing::new(f(x)))
}

#[async_trait(?Send)]
impl IdentityKeyStore for IdStore {
    async fn get_identity_key_pair(&self) -> SResult<IdentityKeyPair> {
        IdentityKeyPair::try_from(&self.pair[..])
    }
    async fn get_local_registration_id(&self) -> SResult<u32> {
        Ok(self.reg)
    }
    async fn save_identity(&mut self, addr: &ProtocolAddress, id: &IdentityKey) -> SResult<IdentityChange> {
        let new = id.serialize().to_vec();
        let old = self.known.insert(addr.name().to_owned(), new.clone());
        Ok(IdentityChange::from_changed(matches!(old, Some(o) if o != new)))
    }
    async fn is_trusted_identity(&self, addr: &ProtocolAddress, id: &IdentityKey, _: Direction) -> SResult<bool> {
        Ok(match self.known.get(addr.name()) {
            None => true,
            Some(k) => k.as_slice() == &*id.serialize(),
        })
    }
    async fn get_identity(&self, addr: &ProtocolAddress) -> SResult<Option<IdentityKey>> {
        self.known.get(addr.name()).map(|b| IdentityKey::decode(b)).transpose()
    }
}

#[async_trait(?Send)]
impl SessionStore for SessionStoreImpl {
    async fn load_session(&self, addr: &ProtocolAddress) -> SResult<Option<SessionRecord>> {
        self.0.get(addr.name()).map(|b| SessionRecord::deserialize(b)).transpose()
    }
    async fn store_session(&mut self, addr: &ProtocolAddress, rec: &SessionRecord) -> SResult<()> {
        self.0.insert(addr.name().to_owned(), secret(rec.serialize(), |v| v)?);
        Ok(())
    }
}

#[async_trait(?Send)]
impl PreKeyStore for PreKeys {
    async fn get_pre_key(&self, id: PreKeyId) -> SResult<PreKeyRecord> {
        let b = self.0.get(&u32::from(id)).ok_or(SignalProtocolError::InvalidPreKeyId)?;
        PreKeyRecord::deserialize(b)
    }
    async fn save_pre_key(&mut self, id: PreKeyId, rec: &PreKeyRecord) -> SResult<()> {
        self.0.insert(id.into(), secret(rec.serialize(), |v| v)?);
        Ok(())
    }
    async fn remove_pre_key(&mut self, id: PreKeyId) -> SResult<()> {
        self.0.remove(&u32::from(id));
        Ok(())
    }
}

#[async_trait(?Send)]
impl SignedPreKeyStore for SignedPreKeys {
    async fn get_signed_pre_key(&self, id: SignedPreKeyId) -> SResult<SignedPreKeyRecord> {
        let b = self.0.get(&u32::from(id)).ok_or(SignalProtocolError::InvalidSignedPreKeyId)?;
        SignedPreKeyRecord::deserialize(b)
    }
    async fn save_signed_pre_key(&mut self, id: SignedPreKeyId, rec: &SignedPreKeyRecord) -> SResult<()> {
        self.0.insert(id.into(), secret(rec.serialize(), |v| v)?);
        Ok(())
    }
}

#[async_trait(?Send)]
impl KyberPreKeyStore for KyberPreKeys {
    async fn get_kyber_pre_key(&self, id: KyberPreKeyId) -> SResult<KyberPreKeyRecord> {
        let b = self.keys.get(&u32::from(id)).ok_or(SignalProtocolError::InvalidKyberPreKeyId)?;
        KyberPreKeyRecord::deserialize(b)
    }
    async fn save_kyber_pre_key(&mut self, id: KyberPreKeyId, rec: &KyberPreKeyRecord) -> SResult<()> {
        self.keys.insert(id.into(), secret(rec.serialize(), |v| v)?);
        Ok(())
    }
    async fn mark_kyber_pre_key_used(&mut self, id: KyberPreKeyId, ec: SignedPreKeyId, base: &PublicKey) -> SResult<()> {
        let entry = (u32::from(id), u32::from(ec), base.serialize().to_vec());
        if !self.used.insert(entry) {
            return Err(SignalProtocolError::InvalidMessage(
                CiphertextMessageType::PreKey,
                "reused base key".to_owned(),
            ));
        }
        Ok(())
    }
}
