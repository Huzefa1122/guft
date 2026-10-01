//! Memory-only conversations. Messages of temporary rooms, and the encrypted
//! frames still waiting to be sent for them, live here and nowhere else: they
//! are never written to the history database or the outbox, and everything is
//! wiped when the app locks or exits.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use guft_core::Payload;
use guft_store::{Body, Status, StoredMessage};
use zeroize::{Zeroize, Zeroizing};

/// Ids at or above this are temporary messages (database ids never get near it). Kept far below
/// 2^53 so the UI, which reads ids as JavaScript numbers, never loses precision.
pub const TEMP_ID_BASE: i64 = 1 << 40;
const MAX_MSGS_PER_ROOM: usize = 500;
const MAX_BYTES: usize = 64 * 1024 * 1024;
/// Most unsent temporary frames kept per contact.
pub const MAX_QUEUED_PER_CONTACT: usize = 200;

pub fn is_temp_id(id: i64) -> bool {
    id >= TEMP_ID_BASE
}

struct TempMsg {
    msg: StoredMessage,
    file: Option<Zeroizing<Vec<u8>>>,
    read: bool,
    /// Members that have not acknowledged it yet.
    waiting: usize,
    size: usize,
}

impl Drop for TempMsg {
    fn drop(&mut self) {
        match &mut self.msg.body {
            Body::Text(t) => t.zeroize(),
            Body::File { name, .. } => name.zeroize(),
        }
    }
}

/// An invitation to a memory-only room that waits for the user. Like everything temporary,
/// it is never written to disk.
pub struct PendingInvite {
    pub from: String,
    pub ts: u64,
    pub payload: Payload,
}

#[derive(Default)]
pub struct TempStore {
    invites: HashMap<String, PendingInvite>,
    rooms: HashMap<String, Vec<TempMsg>>,
    next: i64,
    bytes: usize,
}

impl TempStore {
    /// Keep a message (ours or a member's). Returns its id.
    pub fn add(&mut self, chat: &str, ts: u64, outgoing: bool, sender: Option<&str>, payload: &Payload, waiting: usize) -> Option<i64> {
        let (body, file, size) = match payload {
            Payload::Text(t) => (Body::Text(t.clone()), None, t.len()),
            Payload::File { name, data } => (Body::File { name: name.clone(), size: data.len() }, Some(Zeroizing::new(data.clone())), data.len() + name.len()),
            _ => return None,
        };
        let id = TEMP_ID_BASE + self.next;
        self.next += 1;
        let status = if outgoing && waiting > 0 { Status::Queued } else if outgoing { Status::Sent } else { Status::Delivered };
        let msg = StoredMessage { id, chat: chat.to_owned(), ts, outgoing, body, status, sender: sender.map(str::to_owned) };
        self.bytes += size;
        let list = self.rooms.entry(chat.to_owned()).or_default();
        list.push(TempMsg { msg, file, read: outgoing, waiting, size });
        // Bounded: drop the oldest first.
        let mut freed = 0;
        while list.len() > MAX_MSGS_PER_ROOM || (self.bytes - freed > MAX_BYTES && list.len() > 1) {
            freed += list.remove(0).size;
        }
        self.bytes -= freed;
        Some(id)
    }

    /// Newest first, like the database: messages with an id below `before`.
    pub fn page(&self, chat: &str, before: Option<i64>, limit: u32) -> Vec<StoredMessage> {
        let before = before.unwrap_or(i64::MAX);
        self.rooms
            .get(chat)
            .map(|l| l.iter().rev().filter(|m| m.msg.id < before).take(limit.clamp(1, 200) as usize).map(|m| m.msg.clone()).collect())
            .unwrap_or_default()
    }

    pub fn summary(&self, chat: &str) -> (Option<StoredMessage>, u32) {
        let Some(list) = self.rooms.get(chat) else { return (None, 0) };
        (list.last().map(|m| m.msg.clone()), list.iter().filter(|m| !m.read).count() as u32)
    }

    pub fn mark_read(&mut self, chat: &str) {
        if let Some(list) = self.rooms.get_mut(chat) {
            list.iter_mut().for_each(|m| m.read = true);
        }
    }

    pub fn file(&self, id: i64) -> Option<(String, Vec<u8>)> {
        let m = self.rooms.values().flatten().find(|m| m.msg.id == id)?;
        match (&m.msg.body, &m.file) {
            (Body::File { name, .. }, Some(data)) => Some((name.clone(), data.to_vec())),
            _ => None,
        }
    }

    /// One member acknowledged. Returns the message's new status, if it still exists.
    pub fn acknowledged(&mut self, id: i64) -> Option<Status> {
        let m = self.rooms.values_mut().flatten().find(|m| m.msg.id == id)?;
        m.waiting = m.waiting.saturating_sub(1);
        m.msg.status = if m.waiting == 0 { Status::Delivered } else { Status::Sent };
        Some(m.msg.status)
    }

    pub fn failed(&mut self, id: i64) {
        if let Some(m) = self.rooms.values_mut().flatten().find(|m| m.msg.id == id) {
            m.msg.status = Status::Failed;
        }
    }

    pub fn remove(&mut self, id: i64) {
        for list in self.rooms.values_mut() {
            if let Some(i) = list.iter().position(|m| m.msg.id == id) {
                self.bytes -= list.remove(i).size;
                return;
            }
        }
    }

    pub fn drop_room(&mut self, chat: &str) {
        if let Some(list) = self.rooms.remove(chat) {
            self.bytes -= list.iter().map(|m| m.size).sum::<usize>();
        }
    }

    /// Hold an invitation to a temporary room. `false` if one for it is already waiting.
    pub fn add_invite(&mut self, room: &str, from: &str, ts: u64, payload: Payload) -> bool {
        if self.invites.contains_key(room) {
            return false;
        }
        self.invites.insert(room.to_owned(), PendingInvite { from: from.to_owned(), ts, payload });
        true
    }

    pub fn invites(&self) -> Vec<(String, &PendingInvite)> {
        self.invites.iter().map(|(k, v)| (k.clone(), v)).collect()
    }

    pub fn take_invite(&mut self, room: &str) -> Option<PendingInvite> {
        self.invites.remove(room)
    }

    pub fn drop_invites_from(&mut self, contact: &str) {
        self.invites.retain(|_, v| v.from != contact);
    }

    /// Forget everything (dropping zeroizes the text and file bytes).
    pub fn clear(&mut self) {
        self.invites.clear();
        self.rooms.clear();
        self.bytes = 0;
    }
}

#[derive(Clone)]
pub struct TempOut {
    /// The message this frame belongs to, or 0 for room control messages.
    pub msg_id: i64,
    pub frame: Vec<u8>,
    pub attempts: u32,
}

/// Frames of temporary rooms waiting for a contact to be reachable.
#[derive(Default)]
pub struct TempOutbox(Mutex<HashMap<String, VecDeque<TempOut>>>);

impl TempOutbox {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, VecDeque<TempOut>>> {
        self.0.lock().expect("poisoned")
    }

    pub fn push(&self, to: &str, item: TempOut) {
        self.lock().entry(to.to_owned()).or_default().push_back(item);
    }

    pub fn queued(&self, to: &str) -> usize {
        self.lock().get(to).map_or(0, VecDeque::len)
    }

    pub fn front(&self, to: &str) -> Option<TempOut> {
        self.lock().get(to).and_then(|q| q.front().cloned())
    }

    pub fn pop(&self, to: &str) {
        let mut g = self.lock();
        if let Some(q) = g.get_mut(to) {
            q.pop_front();
            if q.is_empty() {
                g.remove(to);
            }
        }
    }

    pub fn bump(&self, to: &str) {
        if let Some(f) = self.lock().get_mut(to).and_then(VecDeque::front_mut) {
            f.attempts += 1;
        }
    }

    pub fn contacts(&self) -> Vec<String> {
        self.lock().keys().cloned().collect()
    }

    /// Cancel every unsent frame of one message.
    pub fn cancel(&self, msg_id: i64) {
        let mut g = self.lock();
        g.values_mut().for_each(|q| q.retain(|f| f.msg_id != msg_id));
        g.retain(|_, q| !q.is_empty());
    }

    pub fn drop_contact(&self, to: &str) {
        self.lock().remove(to);
    }

    pub fn clear(&self) {
        self.lock().clear();
    }
}
