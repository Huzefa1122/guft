//! The engine: identity, invites, contacts, and message encrypt/decrypt.
//!
//! Wire frame: `kind(1) | id(16) | outer_frame`, where the outer frame
//! (see [`crate::outer`]) wraps `msg_type(1) | libsignal ciphertext`.
//! `kind` 1 = established contact (id = sender), 2 = first contact via an invite
//! (id = invite id).

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use futures::executor::block_on;
use libsignal_protocol::*;
use rand::rngs::OsRng;
use rand::{CryptoRng, Rng, RngCore, TryRngCore};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::{Error, Result};
use crate::invite::{generate_code, InviteV1};
use crate::limits::*;
use crate::outer::{derive_root, OuterState};
use crate::payload::{check_display_name, check_onion, Payload};
use crate::store::*;

/// 32 lowercase hex chars derived from a contact's identity key.
pub type ContactId = String;

const KIND_CONTACT: u8 = 1;
const KIND_INVITE: u8 = 2;
const HEADER: usize = 17;
const DEVICE: u8 = 1;
/// Safety-number iterations.
const FP_ITERATIONS: u32 = 5200;

pub struct Received {
    pub from: ContactId,
    pub payload: Payload,
    /// True when this message created the contact (invite consumed).
    pub new_contact: bool,
    /// Set when the new contact used a room-bound invite: they should now be sent a `RoomInvite`.
    pub joined_room: Option<[u8; 16]>,
}

#[derive(Clone, Debug)]
pub struct ContactInfo {
    pub id: ContactId,
    pub name: String,
    pub onion: String,
    pub verified: bool,
}

pub struct Engine {
    pub(crate) st: State,
    pub(crate) me: ProtocolAddress,
    /// Memory-only rooms (`Temp` and `Direct`). Never part of `State`, so never serialized.
    pub(crate) temp: BTreeMap<String, crate::rooms::TempRoom>,
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn rng() -> impl Rng + CryptoRng {
    OsRng.unwrap_err()
}

pub(crate) fn id_of(key: &IdentityKey) -> ContactId {
    data_encoding::HEXLOWER.encode(&Sha256::digest(key.serialize())[..16])
}

fn addr(id: &str) -> Result<ProtocolAddress> {
    Ok(ProtocolAddress::new(id.to_owned(), DeviceId::new(DEVICE).map_err(|_| Error::Invalid("device"))?))
}

fn id_bytes(id: &str) -> Result<[u8; 16]> {
    let v = data_encoding::HEXLOWER.decode(id.as_bytes()).map_err(|_| Error::UnknownContact)?;
    v.try_into().map_err(|_| Error::UnknownContact)
}

impl Engine {
    pub fn create(display_name: &str) -> Result<Self> {
        check_display_name(display_name)?;
        let mut r = rng();
        let pair = IdentityKeyPair::generate(&mut r);
        let reg = r.random::<u32>() & 0x3fff;
        let mut onion_seed = Zeroizing::new([0u8; 32]);
        r.fill_bytes(&mut *onion_seed);
        let st = State {
            name: display_name.to_owned(),
            onion: None,
            onion_seed,
            spool: crate::spool::SpoolSecret::generate(),
            online_when_locked: false,
            ids: IdStore { pair: Zeroizing::new(pair.serialize().to_vec()), reg, known: BTreeMap::new() },
            sessions: Default::default(),
            prekeys: Default::default(),
            signed: Default::default(),
            kyber: Default::default(),
            contacts: BTreeMap::new(),
            invites: BTreeMap::new(),
            rooms: BTreeMap::new(),
        };
        Self::from_state(st)
    }

    fn from_state(st: State) -> Result<Self> {
        let pair = IdentityKeyPair::try_from(&st.ids.pair[..])?;
        let me = addr(&id_of(pair.identity_key()))?;
        Ok(Self { st, me, temp: BTreeMap::new() })
    }

    /// Serialized state; the caller seals it with a [`crate::vault::Vault`].
    pub fn to_bytes(&self) -> Result<Zeroizing<Vec<u8>>> {
        Ok(Zeroizing::new(postcard::to_allocvec(&self.st)?))
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        Self::from_state(postcard::from_bytes(bytes)?)
    }

    pub fn my_id(&self) -> &str {
        self.me.name()
    }

    pub fn display_name(&self) -> &str {
        &self.st.name
    }

    /// Seed for our onion service identity (see `guft-net`).
    pub fn onion_seed(&self) -> Zeroizing<[u8; 32]> {
        self.st.onion_seed.clone()
    }

    pub fn onion(&self) -> Option<&str> {
        self.st.onion.as_deref()
    }

    /// Public half of the spool key; keep it to seal frames while locked.
    pub fn spool_public(&self) -> crate::spool::SpoolPublic {
        self.st.spool.public()
    }

    pub fn open_spooled(&self, blob: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
        self.st.spool.open(blob)
    }

    pub fn online_when_locked(&self) -> bool {
        self.st.online_when_locked
    }

    pub fn set_online_when_locked(&mut self, on: bool) {
        self.st.online_when_locked = on;
    }

    /// A key derived from the vault's onion seed, so per-contact auth keys need no storage.
    fn auth_secret(&self, label: &str) -> Zeroizing<[u8; 32]> {
        let mut out = Zeroizing::new([0u8; 32]);
        hkdf::Hkdf::<Sha256>::new(None, &*self.st.onion_seed)
            .expand(format!("guft client auth v1:{label}").as_bytes(), &mut *out)
            .expect("32 bytes");
        out
    }

    fn invite_label(invite_id: &[u8; 16]) -> String {
        format!("i-{}", data_encoding::HEXLOWER.encode(invite_id))
    }

    fn contact_label(contact: &str) -> String {
        format!("c-{contact}")
    }

    /// Who may discover our onion service: `(nickname, key secret)`. Pending invites and
    /// every contact. The network layer turns the secrets into public keys.
    pub fn authorized_clients(&self, now: u64) -> Vec<(String, Zeroizing<[u8; 32]>)> {
        let mut out = Vec::new();
        for r in self.st.invites.values().filter(|r| !r.consumed && r.expires_at > now) {
            let label = Self::invite_label(&r.invite_id);
            out.push((label.clone(), self.auth_secret(&label)));
        }
        for id in self.st.contacts.keys() {
            let label = Self::contact_label(id);
            out.push((label.clone(), self.auth_secret(&label)));
        }
        out
    }

    /// Keys to present when connecting out: `(their onion, key secret)`.
    pub fn connect_keys(&self) -> Vec<(String, Zeroizing<[u8; 32]>)> {
        self.st
            .contacts
            .values()
            .filter_map(|c| c.connect_key.clone().map(|k| (c.onion.clone(), k)))
            .collect()
    }

    /// Our first message to a contact: where to reach us, and the key to do it with.
    pub fn hello_for(&self, contact: &str) -> Result<Payload> {
        if !self.st.contacts.contains_key(contact) {
            return Err(Error::UnknownContact);
        }
        Ok(Payload::Hello {
            onion: self.st.onion.clone().ok_or(Error::Invalid("onion service not ready"))?,
            name: self.st.name.clone(),
            auth: *self.auth_secret(&Self::contact_label(contact)),
        })
    }

    /// Hand a new contact the permanent key that replaces the invite's one-use key.
    pub fn auth_key_for(&self, contact: &str) -> Result<Payload> {
        if !self.st.contacts.contains_key(contact) {
            return Err(Error::UnknownContact);
        }
        Ok(Payload::AuthKey { key: *self.auth_secret(&Self::contact_label(contact)) })
    }

    /// Set once the onion service is up.
    pub fn set_onion(&mut self, onion: &str) -> Result<()> {
        check_onion(onion)?;
        self.st.onion = Some(onion.to_owned());
        Ok(())
    }

    pub fn contacts(&self) -> Vec<ContactInfo> {
        self.st
            .contacts
            .iter()
            .map(|(id, c)| ContactInfo { id: id.clone(), name: c.name.clone(), onion: c.onion.clone(), verified: c.verified })
            .collect()
    }

    pub fn mark_verified(&mut self, id: &str, verified: bool) -> Result<()> {
        self.st.contacts.get_mut(id).ok_or(Error::UnknownContact)?.verified = verified;
        Ok(())
    }

    pub fn remove_contact(&mut self, id: &str) {
        self.st.contacts.remove(id);
        self.st.sessions.0.remove(id);
        self.st.ids.known.remove(id);
        self.forget_member(id);
    }

    /// A name only you see. The contact is not told, and it is shown in rooms too.
    pub fn rename_contact(&mut self, id: &str, name: &str) -> Result<()> {
        check_display_name(name)?;
        let name = name.trim();
        if name.is_empty() {
            return Err(Error::Invalid("name is empty"));
        }
        self.st.contacts.get_mut(id).ok_or(Error::UnknownContact)?.name = name.to_owned();
        self.rename_member(id, name);
        Ok(())
    }

    /// Numeric safety number, identical on both sides.
    pub fn safety_number(&self, id: &str) -> Result<String> {
        let pair = block_on(self.st.ids.get_identity_key_pair())?;
        let theirs = block_on(self.st.ids.get_identity(&addr(id)?))?.ok_or(Error::UnknownContact)?;
        let fp = Fingerprint::new(2, FP_ITERATIONS, self.my_id().as_bytes(), pair.identity_key(), id.as_bytes(), &theirs)
            .map_err(|e| Error::Protocol(e.to_string()))?;
        fp.display_string().map_err(|e| Error::Protocol(e.to_string()))
    }

    /// Drop expired invites and their prekeys.
    pub fn purge_expired(&mut self, now: u64) {
        let expired: Vec<u32> = self.st.invites.iter().filter(|(_, r)| r.expires_at <= now).map(|(k, _)| *k).collect();
        for k in expired {
            self.drop_invite_keys(k);
            self.st.invites.remove(&k);
        }
    }

    fn drop_invite_keys(&mut self, signed_id: u32) {
        if let Some(rec) = self.st.invites.get(&signed_id) {
            self.st.kyber.keys.remove(&rec.kyber_id);
            self.st.prekeys.0.remove(&rec.prekey_id);
        }
        self.st.signed.0.remove(&signed_id);
    }

    pub fn pending_invites(&self) -> Vec<(String, u64)> {
        self.st.invites.values().filter(|r| !r.consumed).map(|r| (r.label.clone(), r.expires_at)).collect()
    }

    /// Returns `(invite, one_time_code)`. Deliver them over different channels.
    pub fn new_invite(&mut self, label: &str, ttl_secs: u64, now: u64) -> Result<(String, String)> {
        self.new_invite_bound(label, ttl_secs, now, None, None)
    }

    /// Like [`new_invite`](Self::new_invite), optionally tied to a room and/or one identity.
    pub(crate) fn new_invite_bound(
        &mut self,
        label: &str,
        ttl_secs: u64,
        now: u64,
        room: Option<[u8; 16]>,
        expect: Option<String>,
    ) -> Result<(String, String)> {
        check_display_name(label)?;
        self.purge_expired(now);
        let onion = self.st.onion.clone().ok_or(Error::Invalid("onion service not ready"))?;
        if self.st.invites.values().filter(|r| !r.consumed).count() >= MAX_PENDING_INVITES {
            return Err(Error::Limit("too many pending invites"));
        }
        if !(900..=7 * 24 * 3600).contains(&ttl_secs) {
            return Err(Error::Invalid("invite lifetime must be 15 minutes to 7 days"));
        }

        let mut r = rng();
        let pair = block_on(self.st.ids.get_identity_key_pair())?;
        let prekey = KeyPair::generate(&mut r);
        let signed = KeyPair::generate(&mut r);
        let kyber = kem::KeyPair::generate(kem::KeyType::Kyber1024, &mut r);
        let signed_sig = pair.private_key().calculate_signature(&signed.public_key.serialize(), &mut r).map_err(|_| Error::Invalid("signing failed"))?;
        let kyber_sig = pair.private_key().calculate_signature(&kyber.public_key.serialize(), &mut r).map_err(|_| Error::Invalid("signing failed"))?;
        let (prekey_id, signed_id, kyber_id) = (self.fresh_id(&mut r, 0), self.fresh_id(&mut r, 1), self.fresh_id(&mut r, 2));

        block_on(self.st.prekeys.save_pre_key(prekey_id.into(), &PreKeyRecord::new(prekey_id.into(), &prekey)))?;
        block_on(self.st.signed.save_signed_pre_key(
            signed_id.into(),
            &SignedPreKeyRecord::new(signed_id.into(), Timestamp::from_epoch_millis(now * 1000), &signed, &signed_sig),
        ))?;
        block_on(self.st.kyber.save_kyber_pre_key(
            kyber_id.into(),
            &KyberPreKeyRecord::new(kyber_id.into(), Timestamp::from_epoch_millis(now * 1000), &kyber, &kyber_sig),
        ))?;

        let mut invite_id = [0u8; 16];
        let mut secret = [0u8; 32];
        r.fill_bytes(&mut invite_id);
        r.fill_bytes(&mut secret);
        let code = generate_code(&mut r);
        let root = derive_root(&secret, &code)?;
        let expires_at = now + ttl_secs;

        let auth = *self.auth_secret(&Self::invite_label(&invite_id));
        let invite = InviteV1 {
            invite_id,
            secret,
            auth,
            expires_at,
            onion,
            name: self.st.name.clone(),
            reg: self.st.ids.reg,
            device: DEVICE,
            prekey_id,
            prekey: prekey.public_key.serialize().to_vec(),
            signed_id,
            signed: signed.public_key.serialize().to_vec(),
            signed_sig: signed_sig.to_vec(),
            kyber_id,
            kyber: kyber.public_key.serialize().to_vec(),
            kyber_sig: kyber_sig.to_vec(),
            identity: pair.identity_key().serialize().to_vec(),
            room,
        };
        let text = invite.encode()?;
        self.st.invites.insert(
            signed_id,
            InviteRec { label: label.to_owned(), invite_id, expires_at, root, kyber_id, prekey_id, consumed: false, consumed_by: None, room, expect },
        );
        Ok((text, code))
    }

    /// An id not already used in the matching store (they are random u32s).
    fn fresh_id(&self, r: &mut impl Rng, which: u8) -> u32 {
        loop {
            let id: u32 = r.random_range(1..u32::MAX);
            let taken = match which {
                0 => self.st.prekeys.0.contains_key(&id),
                1 => self.st.signed.0.contains_key(&id),
                _ => self.st.kyber.keys.contains_key(&id),
            };
            if !taken {
                return id;
            }
        }
    }

    pub fn revoke_invite(&mut self, signed_id_or_label: &str) {
        let hit: Vec<u32> = self
            .st
            .invites
            .iter()
            .filter(|(_, r)| !r.consumed && r.label == signed_id_or_label)
            .map(|(k, _)| *k)
            .collect();
        for k in hit {
            self.drop_invite_keys(k);
            self.st.invites.remove(&k);
        }
    }

    /// Import an invite plus its one-time code. Afterwards, send a
    /// [`Payload::Hello`] first.
    /// The room an invite leads into, if it is a room invite. Lets the app recognise the
    /// room invitation that follows as one the user asked for.
    pub fn invite_room(invite: &str) -> Option<[u8; 16]> {
        InviteV1::decode(invite).ok().and_then(|i| i.room)
    }

    pub fn add_contact(&mut self, invite: &str, code: &str, now: u64) -> Result<ContactId> {
        let inv = InviteV1::decode(invite)?;
        if inv.expires_at <= now {
            return Err(Error::Invalid("invite expired"));
        }
        if self.st.contacts.len() >= MAX_CONTACTS {
            return Err(Error::Limit("too many contacts"));
        }
        let identity = inv.identity_key()?;
        let id = id_of(&identity);
        if id == self.my_id() {
            return Err(Error::Invalid("that is your own invite"));
        }
        if self.st.contacts.contains_key(&id) {
            return Err(Error::Invalid("already a contact"));
        }
        let root = derive_root(&inv.secret, code)?;
        let bundle = inv.bundle()?;
        let remote = addr(&id)?;
        let mut r = rng();
        block_on(self.st.ids.save_identity(&remote, &identity))?;
        let result = block_on(process_prekey_bundle(
            &remote,
            &self.me,
            &mut self.st.sessions,
            &mut self.st.ids,
            &bundle,
            SystemTime::now(),
            &mut r,
        ));
        if let Err(e) = result {
            self.st.ids.known.remove(&id);
            return Err(e.into());
        }
        self.st.contacts.insert(
            id.clone(),
            Contact {
                name: inv.name,
                onion: inv.onion,
                verified: false,
                outer: OuterState::new(&root, true),
                connect_key: Some(Zeroizing::new(inv.auth)),
                pending_invite: Some(inv.invite_id),
            },
        );
        Ok(id)
    }

    pub fn encrypt(&mut self, to: &str, payload: &Payload) -> Result<Vec<u8>> {
        let plain = Zeroizing::new(payload.encode()?);
        let (kind, hid) = {
            let c = self.st.contacts.get(to).ok_or(Error::UnknownContact)?;
            match c.pending_invite {
                Some(inv) => (KIND_INVITE, inv),
                None => (KIND_CONTACT, self.my_id_bytes()?),
            }
        };
        let remote = addr(to)?;
        let mut r = rng();
        let ct = block_on(message_encrypt(
            &plain,
            &remote,
            &self.me,
            &mut self.st.sessions,
            &mut self.st.ids,
            SystemTime::now(),
            &mut r,
        ))?;
        let mut inner = vec![ct.message_type() as u8];
        inner.extend_from_slice(ct.serialize());

        let mut header = vec![kind];
        header.extend_from_slice(&hid);
        let contact = self.st.contacts.get_mut(to).ok_or(Error::UnknownContact)?;
        let outer = contact.outer.seal(&header, &inner)?;
        header.extend_from_slice(&outer);
        if header.len() > MAX_WIRE_BYTES {
            return Err(Error::Limit("frame too large"));
        }
        Ok(header)
    }

    fn my_id_bytes(&self) -> Result<[u8; 16]> {
        id_bytes(self.my_id())
    }

    pub fn decrypt(&mut self, wire: &[u8], now: u64) -> Result<Received> {
        if wire.len() > MAX_WIRE_BYTES || wire.len() < HEADER + 8 + 16 {
            return Err(Error::Invalid("bad frame size"));
        }
        let (header, outer_frame) = wire.split_at(HEADER);
        let mut hid = [0u8; 16];
        hid.copy_from_slice(&header[1..]);

        match header[0] {
            KIND_CONTACT => {
                let from = data_encoding::HEXLOWER.encode(&hid);
                self.decrypt_from(&from, header, outer_frame)
            }
            KIND_INVITE => {
                // An invite already consumed by a contact keeps working for that contact
                // until they have heard back from us.
                if let Some(cid) = self.st.invites.values().find(|r| r.invite_id == hid && r.consumed).and_then(|r| r.consumed_by.clone()) {
                    return self.decrypt_from(&cid, header, outer_frame);
                }
                self.accept_first_contact(hid, header, outer_frame, now)
            }
            _ => Err(Error::Invalid("unknown frame kind")),
        }
    }

    fn decrypt_from(&mut self, from: &str, header: &[u8], outer_frame: &[u8]) -> Result<Received> {
        let contact = self.st.contacts.get_mut(from).ok_or(Error::UnknownContact)?;
        let inner = contact.outer.open(header, outer_frame)?;
        let payload = self.signal_decrypt(from, &inner)?;
        if let Some(c) = self.st.contacts.get_mut(from) {
            c.pending_invite = None;
            if let Payload::AuthKey { key } = &payload {
                c.connect_key = Some(Zeroizing::new(*key));
            }
        }
        Ok(Received { from: from.to_owned(), payload, new_contact: false, joined_room: None })
    }

    fn signal_decrypt(&mut self, from: &str, inner: &[u8]) -> Result<Payload> {
        let (ty, body) = inner.split_first().ok_or(Error::Invalid("empty message"))?;
        let msg = match *ty {
            t if t == CiphertextMessageType::Whisper as u8 => CiphertextMessage::SignalMessage(SignalMessage::try_from(body)?),
            t if t == CiphertextMessageType::PreKey as u8 => CiphertextMessage::PreKeySignalMessage(PreKeySignalMessage::try_from(body)?),
            _ => return Err(Error::Invalid("unknown message type")),
        };
        let remote = addr(from)?;
        let mut r = rng();
        let plain = Zeroizing::new(block_on(message_decrypt(
            &msg,
            &remote,
            &self.me,
            &mut self.st.sessions,
            &mut self.st.ids,
            &mut self.st.prekeys,
            &self.st.signed,
            &mut self.st.kyber,
            &mut r,
        ))?);
        Payload::decode(plain.to_vec())
    }

    fn accept_first_contact(&mut self, invite_id: [u8; 16], header: &[u8], outer_frame: &[u8], now: u64) -> Result<Received> {
        let (signed_id, rec) = self
            .st
            .invites
            .iter()
            .find(|(_, r)| r.invite_id == invite_id && !r.consumed)
            .map(|(k, r)| (*k, r.clone()))
            .ok_or(Error::UnknownContact)?;
        if rec.expires_at <= now {
            return Err(Error::Invalid("invite expired"));
        }
        let mut outer = OuterState::new(&rec.root, false);
        let inner = outer.open(header, outer_frame)?;
        let (ty, body) = inner.split_first().ok_or(Error::Invalid("empty message"))?;
        if *ty != CiphertextMessageType::PreKey as u8 {
            return Err(Error::Invalid("first message must be a prekey message"));
        }
        let pre = PreKeySignalMessage::try_from(body)?;
        if u32::from(pre.signed_pre_key_id()) != signed_id {
            return Err(Error::Invalid("wrong invite"));
        }
        let from = id_of(pre.identity_key());
        if from == self.my_id() || self.st.contacts.contains_key(&from) || self.st.contacts.len() >= MAX_CONTACTS {
            return Err(Error::Invalid("cannot accept contact"));
        }
        if rec.expect.as_deref().is_some_and(|e| e != from) {
            return Err(Error::Invalid("invite is for someone else"));
        }

        let backup = self.st.clone();
        let outcome = (|| -> Result<Received> {
            let payload = self.signal_decrypt(&from, &inner)?;
            let Payload::Hello { onion, name, auth } = &payload else {
                return Err(Error::Invalid("first message must be a hello"));
            };
            self.st.contacts.insert(
                from.clone(),
                Contact {
                    name: name.clone(),
                    onion: onion.clone(),
                    verified: false,
                    outer,
                    connect_key: Some(Zeroizing::new(*auth)),
                    pending_invite: None,
                },
            );
            self.drop_invite_keys(signed_id);
            if let Some(r) = self.st.invites.get_mut(&signed_id) {
                r.consumed = true;
                r.consumed_by = Some(from.clone());
            }
            // A room-bound invite makes the newcomer a member on our side right away.
            let mut joined_room = None;
            if let (Some(room_id), Payload::Hello { onion, name, .. }) = (rec.room, &payload) {
                let key = data_encoding::HEXLOWER.encode(&room_id);
                // A one-to-one temp chat never takes anyone else.
                let direct = self.kind_of(&key) == Some(crate::store::RoomKind::Direct);
                if let Some(room) = self.room_get_mut(&key).filter(|_| !direct) {
                    if room.members.len() + 1 < MAX_ROOM_MEMBERS {
                        room.members.insert(from.clone(), crate::store::RoomMember { name: name.clone(), onion: onion.clone() });
                        joined_room = Some(room_id);
                    }
                }
            }
            Ok(Received { from: from.clone(), payload, new_contact: true, joined_room })
        })();
        if outcome.is_err() {
            self.st = backup;
        }
        outcome
    }
}
