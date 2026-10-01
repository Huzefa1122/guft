//! Private rooms: a roster, with every message sent to each member over our
//! existing pairwise post-quantum sessions. There is no shared group key and no
//! host. Removing someone is therefore simple and strong: nobody encrypts to
//! them any more.
//!
//! Joining is hands-off. A room-bound invite makes the newcomer a member on the
//! inviter's side; the inviter sends a `RoomInvite` (the roster); the newcomer
//! creates a one-time invite for every other member and sends them to the
//! inviter (`Introduce`), who passes each on (`Introduction`) so members connect
//! to the newcomer automatically.

use std::collections::BTreeMap;

use data_encoding::HEXLOWER;
use rand::rngs::OsRng;
use rand::{RngCore, TryRngCore};

use crate::engine::{id_of, Engine};
use crate::error::{Error, Result};
use crate::invite::InviteV1;
use crate::limits::{MAX_ROOMS, MAX_ROOM_MEMBERS};
use crate::payload::{check_display_name, Payload, RosterEntry};
use crate::store::{Room, RoomKind, RoomMember};

pub type RoomId = [u8; 16];

pub fn room_key(id: &RoomId) -> String {
    HEXLOWER.encode(id)
}

pub fn parse_room_key(key: &str) -> Result<RoomId> {
    let v = HEXLOWER.decode(key.as_bytes()).map_err(|_| Error::Invalid("bad room id"))?;
    v.try_into().map_err(|_| Error::Invalid("bad room id"))
}

/// A memory-only room: the roster plus whether it is a one-to-one chat.
#[derive(Clone)]
pub struct TempRoom {
    pub room: Room,
    pub kind: RoomKind,
}

#[derive(Clone, Debug)]
pub struct RoomInfo {
    pub id: String,
    pub name: String,
    pub kind: RoomKind,
    pub creator: String,
    /// We created it.
    pub mine: bool,
    pub open_invites: bool,
    /// Everyone but us.
    pub members: Vec<RosterEntry>,
    /// Members we can actually message (we have a session with them).
    pub reachable: usize,
}

/// A room message to show the user.
#[derive(Clone, Debug)]
pub struct Delivery {
    pub room: String,
    /// The room lives in memory only: the message must not be written to disk.
    pub temp: bool,
    pub from: String,
    pub payload: Payload,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoomEvent {
    Joined { room: String, name: String },
    Changed { room: String },
    Removed { room: String },
    ContactAdded { id: String, name: String },
}

/// What the app must do after handling a room payload.
#[derive(Debug, Default)]
pub struct RoomEffects {
    /// Payloads to encrypt and queue: `(contact id, payload)`.
    pub send: Vec<(String, Payload)>,
    pub deliver: Option<Delivery>,
    pub events: Vec<RoomEvent>,
}

const INTRO_TTL_SECS: u64 = 7 * 24 * 3600;

impl Engine {
    pub(crate) fn room_get(&self, key: &str) -> Option<&Room> {
        self.st.rooms.get(key).or_else(|| self.temp.get(key).map(|t| &t.room))
    }

    pub(crate) fn room_get_mut(&mut self, key: &str) -> Option<&mut Room> {
        if self.st.rooms.contains_key(key) {
            self.st.rooms.get_mut(key)
        } else {
            self.temp.get_mut(key).map(|t| &mut t.room)
        }
    }

    pub(crate) fn kind_of(&self, key: &str) -> Option<RoomKind> {
        if self.st.rooms.contains_key(key) {
            Some(RoomKind::Normal)
        } else {
            self.temp.get(key).map(|t| t.kind)
        }
    }

    fn room_remove(&mut self, key: &str) {
        self.st.rooms.remove(key);
        self.temp.remove(key);
    }

    fn room_count(&self) -> usize {
        self.st.rooms.len() + self.temp.len()
    }

    /// A removed contact leaves every roster we keep; a one-to-one temp chat with them ends.
    pub(crate) fn forget_member(&mut self, id: &str) {
        for r in self.st.rooms.values_mut() {
            r.members.remove(id);
        }
        self.temp.retain(|_, t| !(t.kind == RoomKind::Direct && t.room.members.contains_key(id)));
        for t in self.temp.values_mut() {
            t.room.members.remove(id);
        }
    }

    pub(crate) fn rename_member(&mut self, id: &str, name: &str) {
        let rooms = self.st.rooms.values_mut().chain(self.temp.values_mut().map(|t| &mut t.room));
        for r in rooms {
            if let Some(m) = r.members.get_mut(id) {
                m.name = name.to_owned();
            }
        }
    }

    /// True for rooms that exist in memory only (their messages must stay there too).
    pub fn is_temp_room(&self, key: &str) -> bool {
        self.temp.contains_key(key)
    }

    fn room_ref(&self, key: &str) -> Result<&Room> {
        self.room_get(key).ok_or(Error::Invalid("no such room"))
    }

    fn can_invite(&self, key: &str, room: &Room) -> bool {
        self.kind_of(key) != Some(RoomKind::Direct) && (room.creator == self.my_id() || room.open_invites)
    }

    fn me_entry(&self) -> Result<RosterEntry> {
        Ok(RosterEntry {
            id: self.my_id().to_owned(),
            name: self.st.name.clone(),
            onion: self.st.onion.clone().ok_or(Error::Invalid("onion service not ready"))?,
        })
    }

    pub fn create_room(&mut self, name: &str, open_invites: bool) -> Result<String> {
        self.create_room_of(name, open_invites, RoomKind::Normal)
    }

    /// A group room that lives in memory only: nothing about it is ever saved.
    pub fn create_temp_room(&mut self, name: &str, open_invites: bool) -> Result<String> {
        self.create_room_of(name, open_invites, RoomKind::Temp)
    }

    fn create_room_of(&mut self, name: &str, open_invites: bool, kind: RoomKind) -> Result<String> {
        check_display_name(name)?;
        if name.trim().is_empty() {
            return Err(Error::Invalid("room needs a name"));
        }
        if self.room_count() >= MAX_ROOMS {
            return Err(Error::Limit("too many rooms"));
        }
        let mut id = [0u8; 16];
        OsRng.unwrap_err().fill_bytes(&mut id);
        let key = room_key(&id);
        let room = Room { name: name.trim().to_owned(), creator: self.my_id().to_owned(), open_invites, members: BTreeMap::new(), invited_by: None };
        self.insert_room(key.clone(), room, kind);
        Ok(key)
    }

    fn insert_room(&mut self, key: String, room: Room, kind: RoomKind) {
        if kind == RoomKind::Normal {
            self.st.rooms.insert(key, room);
        } else {
            self.temp.insert(key, TempRoom { room, kind });
        }
    }

    /// Start (or find) the memory-only chat with one contact. The payload, when
    /// there is one, must be sent to the contact so their side opens it too.
    pub fn start_direct_temp(&mut self, contact: &str) -> Result<(String, Option<Payload>)> {
        let c = self.st.contacts.get(contact).ok_or(Error::UnknownContact)?.clone();
        if let Some((key, _)) = self.temp.iter().find(|(_, t)| t.kind == RoomKind::Direct && t.room.members.contains_key(contact)) {
            return Ok((key.clone(), None));
        }
        if self.room_count() >= MAX_ROOMS {
            return Err(Error::Limit("too many rooms"));
        }
        let mut id = [0u8; 16];
        OsRng.unwrap_err().fill_bytes(&mut id);
        let key = room_key(&id);
        let mut members = BTreeMap::new();
        members.insert(contact.to_owned(), RoomMember { name: c.name, onion: c.onion });
        // The wire name is a placeholder: both sides show the other person's name.
        let room = Room { name: "Temp chat".to_owned(), creator: self.my_id().to_owned(), open_invites: false, members, invited_by: None };
        self.insert_room(key.clone(), room, RoomKind::Direct);
        let invite = self.room_invite_for(&key, contact)?;
        Ok((key, Some(invite)))
    }

    pub fn rooms(&self) -> Vec<RoomInfo> {
        let normal = self.st.rooms.iter().map(|(id, r)| (id, r, RoomKind::Normal));
        let temp = self.temp.iter().map(|(id, t)| (id, &t.room, t.kind));
        normal
            .chain(temp)
            .map(|(id, r, kind)| RoomInfo {
                id: id.clone(),
                name: r.name.clone(),
                kind,
                creator: r.creator.clone(),
                mine: r.creator == self.my_id(),
                open_invites: r.open_invites,
                members: r.members.iter().map(|(mid, m)| RosterEntry { id: mid.clone(), name: m.name.clone(), onion: m.onion.clone() }).collect(),
                reachable: r.members.keys().filter(|m| self.st.contacts.contains_key(*m)).count(),
            })
            .collect()
    }

    /// Members we have a session with: where a room message must be sent.
    pub fn room_recipients(&self, key: &str) -> Result<Vec<String>> {
        Ok(self.room_ref(key)?.members.keys().filter(|m| self.st.contacts.contains_key(*m)).cloned().collect())
    }

    /// The roster a new member receives: everyone but them, including us.
    fn roster_for(&self, room: &Room, exclude: &str) -> Result<Vec<RosterEntry>> {
        let mut roster = vec![self.me_entry()?];
        roster.extend(
            room.members.iter().filter(|(id, _)| id.as_str() != exclude).map(|(id, m)| RosterEntry { id: id.clone(), name: m.name.clone(), onion: m.onion.clone() }),
        );
        Ok(roster)
    }

    /// An invite that also puts whoever uses it into the room.
    pub fn new_room_invite(&mut self, key: &str, label: &str, ttl_secs: u64, now: u64) -> Result<(String, String)> {
        let room = self.room_ref(key)?;
        if !self.can_invite(key, room) {
            return Err(Error::Invalid("only the creator can invite to this room"));
        }
        if room.members.len() + 1 >= MAX_ROOM_MEMBERS {
            return Err(Error::Limit("room is full"));
        }
        let id = parse_room_key(key)?;
        self.new_invite_bound(label, ttl_secs, now, Some(id), None)
    }

    /// The `RoomInvite` to send a new member (also used when their invite is accepted).
    pub fn room_invite_for(&self, key: &str, member: &str) -> Result<Payload> {
        let room = self.room_ref(key)?;
        Ok(Payload::RoomInvite {
            room: parse_room_key(key)?,
            name: room.name.clone(),
            creator: room.creator.clone(),
            open_invites: room.open_invites,
            kind: self.kind_of(key).unwrap_or(RoomKind::Normal),
            members: self.roster_for(room, member)?,
        })
    }

    /// Add an existing contact to a room. Returns the payload to send them.
    pub fn add_contact_to_room(&mut self, key: &str, contact: &str) -> Result<Payload> {
        let c = self.st.contacts.get(contact).ok_or(Error::UnknownContact)?.clone();
        let room = self.room_ref(key)?;
        if !self.can_invite(key, room) {
            return Err(Error::Invalid("only the creator can invite to this room"));
        }
        if room.members.contains_key(contact) {
            return Err(Error::Invalid("already in the room"));
        }
        if room.members.len() + 1 >= MAX_ROOM_MEMBERS {
            return Err(Error::Limit("room is full"));
        }
        let payload = self.room_invite_for(key, contact)?;
        let room = self.room_get_mut(key).expect("checked above");
        room.members.insert(contact.to_owned(), RoomMember { name: c.name, onion: c.onion });
        Ok(payload)
    }

    fn broadcast(&self, key: &str, payload: Payload, also: &[String]) -> Result<Vec<(String, Payload)>> {
        let mut to = self.room_recipients(key)?;
        for extra in also {
            if self.st.contacts.contains_key(extra) && !to.contains(extra) {
                to.push(extra.clone());
            }
        }
        Ok(to.into_iter().map(|t| (t, payload.clone())).collect())
    }

    /// Leave: tell everyone, then forget the room.
    pub fn leave_room(&mut self, key: &str) -> Result<Vec<(String, Payload)>> {
        let id = parse_room_key(key)?;
        let out = self.broadcast(key, Payload::RoomLeave { room: id }, &[])?;
        self.room_remove(key);
        Ok(out)
    }

    /// Creator only. The removed member is told too, and nobody encrypts to them again.
    pub fn remove_member(&mut self, key: &str, member: &str) -> Result<Vec<(String, Payload)>> {
        let room = self.room_ref(key)?;
        if room.creator != self.my_id() || self.kind_of(key) == Some(RoomKind::Direct) {
            return Err(Error::Invalid("only the creator can remove members"));
        }
        if !room.members.contains_key(member) {
            return Err(Error::UnknownContact);
        }
        let id = parse_room_key(key)?;
        let out = self.broadcast(key, Payload::RoomRemove { room: id, member: member.to_owned() }, &[])?;
        self.room_get_mut(key).expect("checked").members.remove(member);
        Ok(out)
    }

    pub fn rename_room(&mut self, key: &str, name: &str) -> Result<Vec<(String, Payload)>> {
        check_display_name(name)?;
        if name.trim().is_empty() {
            return Err(Error::Invalid("room needs a name"));
        }
        if self.room_ref(key)?.creator != self.my_id() || self.kind_of(key) == Some(RoomKind::Direct) {
            return Err(Error::Invalid("only the creator can rename the room"));
        }
        let id = parse_room_key(key)?;
        self.room_get_mut(key).expect("checked").name = name.trim().to_owned();
        self.broadcast(key, Payload::RoomRename { room: id, name: name.trim().to_owned() }, &[])
    }

    /// Verify an introduction's invite really belongs to `expect`, then use it.
    fn add_contact_expecting(&mut self, invite: &str, code: &str, expect: &str, now: u64) -> Result<String> {
        let inv = InviteV1::decode(invite)?;
        if id_of(&inv.identity_key()?) != expect {
            return Err(Error::Invalid("introduction does not match the member"));
        }
        self.add_contact(invite, code, now)
    }

    /// Apply a room-related payload that arrived from contact `from`.
    pub fn on_room_payload(&mut self, from: &str, payload: &Payload, now: u64) -> Result<RoomEffects> {
        let mut fx = RoomEffects::default();
        match payload {
            Payload::RoomInvite { room, name, creator, open_invites, kind, members } => {
                let key = room_key(room);
                if self.room_get(&key).is_some() || self.room_count() >= MAX_ROOMS {
                    return Ok(fx);
                }
                // A one-to-one chat is between its creator and us, nobody else.
                if *kind == RoomKind::Direct && creator != from {
                    return Err(Error::Invalid("room invite is inconsistent"));
                }
                let me = self.my_id().to_owned();
                let roster: BTreeMap<String, RoomMember> = members
                    .iter()
                    .filter(|e| e.id != me)
                    .map(|e| (e.id.clone(), RoomMember { name: e.name.clone(), onion: e.onion.clone() }))
                    .collect();
                if !roster.contains_key(from) || roster.len() + 1 > MAX_ROOM_MEMBERS {
                    return Err(Error::Invalid("room invite is inconsistent"));
                }
                // Ask whoever added us to introduce us to every member we don't know yet.
                let strangers: Vec<String> = roster.keys().filter(|id| id.as_str() != from && !self.st.contacts.contains_key(*id)).cloned().collect();
                let joined = Room { name: name.clone(), creator: creator.clone(), open_invites: *open_invites, members: roster, invited_by: Some(from.to_owned()) };
                self.insert_room(key.clone(), joined, *kind);
                for id in strangers {
                    let (invite, code) = self.new_invite_bound(&format!("room {name}"), INTRO_TTL_SECS, now, None, Some(id.clone()))?;
                    fx.send.push((from.to_owned(), Payload::Introduce { room: *room, to: id, invite, code }));
                }
                fx.events.push(RoomEvent::Joined { room: key, name: name.clone() });
            }

            Payload::Introduce { room, to, invite, code } => {
                let key = room_key(room);
                let Some(r) = self.room_get(&key) else { return Ok(fx) };
                // We only vouch for a member we know, towards a member we know, and only if we may invite.
                let (Some(newcomer), true, true) = (r.members.get(from), r.members.contains_key(to), self.can_invite(&key, r)) else { return Ok(fx) };
                let entry = RosterEntry { id: from.to_owned(), name: newcomer.name.clone(), onion: newcomer.onion.clone() };
                fx.send.push((to.clone(), Payload::Introduction { room: *room, from: entry, invite: invite.clone(), code: code.clone() }));
            }

            Payload::Introduction { room, from: entry, invite, code } => {
                let key = room_key(room);
                let me = self.my_id().to_owned();
                let may_vouch = |r: &Room| r.members.contains_key(from) && (r.creator == from || r.open_invites);
                let direct = self.kind_of(&key) == Some(RoomKind::Direct);
                let Some(r) = self.room_get_mut(&key) else { return Ok(fx) };
                if direct || !may_vouch(r) || entry.id == me {
                    return Ok(fx);
                }
                if !r.members.contains_key(&entry.id) && r.members.len() + 1 >= MAX_ROOM_MEMBERS {
                    return Ok(fx);
                }
                r.members.insert(entry.id.clone(), RoomMember { name: entry.name.clone(), onion: entry.onion.clone() });
                fx.events.push(RoomEvent::Changed { room: key });
                if !self.st.contacts.contains_key(&entry.id) {
                    // A bad or expired introduction leaves them on the roster without a session.
                    if let Ok(cid) = self.add_contact_expecting(invite, code, &entry.id, now) {
                        fx.send.push((cid.clone(), self.hello_for(&cid)?));
                        fx.events.push(RoomEvent::ContactAdded { id: cid, name: entry.name.clone() });
                    }
                }
            }

            Payload::RoomText { room, .. } | Payload::RoomFile { room, .. } => {
                let key = room_key(room);
                if self.room_get(&key).is_some_and(|r| r.members.contains_key(from)) {
                    let temp = self.is_temp_room(&key);
                    fx.deliver = Some(Delivery { room: key, temp, from: from.to_owned(), payload: payload.clone() });
                }
            }

            Payload::RoomLeave { room } => {
                let key = room_key(room);
                let direct = self.kind_of(&key) == Some(RoomKind::Direct);
                if let Some(r) = self.room_get_mut(&key) {
                    if r.members.remove(from).is_some() {
                        if direct {
                            // Ending a one-to-one temp chat ends it for both sides.
                            self.room_remove(&key);
                            fx.events.push(RoomEvent::Removed { room: key });
                        } else {
                            fx.events.push(RoomEvent::Changed { room: key });
                        }
                    }
                }
            }

            Payload::RoomRemove { room, member } => {
                let key = room_key(room);
                let me = self.my_id().to_owned();
                let Some(r) = self.room_get_mut(&key) else { return Ok(fx) };
                if r.creator != from {
                    return Ok(fx);
                }
                if *member == me {
                    self.room_remove(&key);
                    fx.events.push(RoomEvent::Removed { room: key });
                } else if r.members.remove(member).is_some() {
                    fx.events.push(RoomEvent::Changed { room: key });
                }
            }

            Payload::RoomRename { room, name } => {
                let key = room_key(room);
                let direct = self.kind_of(&key) == Some(RoomKind::Direct);
                if let Some(r) = self.room_get_mut(&key).filter(|_| !direct) {
                    if r.creator == from {
                        r.name = name.clone();
                        fx.events.push(RoomEvent::Changed { room: key });
                    }
                }
            }
            _ => {}
        }
        Ok(fx)
    }
}
