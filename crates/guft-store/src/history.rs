//! Chat history in a SQLCipher database, keyed by a vault subkey.

use std::fs::OpenOptions;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use guft_core::limits::{MAX_FILE_BYTES, MAX_TEXT_BYTES};
use guft_core::Payload;
use rusqlite::{params, Connection, OptionalExtension};
use zeroize::Zeroizing;

use crate::{Error, Result};

const MAX_PAGE: u32 = 200;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i64)]
pub enum Status {
    Queued = 0,
    Sent = 1,
    Delivered = 2,
    Failed = 3,
}

impl Status {
    fn from_i64(v: i64) -> Self {
        match v {
            1 => Status::Sent,
            2 => Status::Delivered,
            3 => Status::Failed,
            _ => Status::Queued,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    Text(String),
    /// Bytes are fetched on demand with [`History::file_bytes`].
    File { name: String, size: usize },
}

#[derive(Clone, Debug)]
pub struct StoredMessage {
    pub id: i64,
    pub chat: String,
    pub ts: u64,
    pub outgoing: bool,
    pub body: Body,
    pub status: Status,
    /// Who wrote it, for room messages (a contact id). `None` for one-to-one chats.
    pub sender: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ChatSummary {
    pub chat: String,
    pub last: StoredMessage,
    pub unread: u32,
}

/// A room invitation waiting for an answer: `(room, from, ts, payload)`.
pub type PendingRoom = (String, String, u64, Vec<u8>);

/// An encrypted frame waiting for delivery.
#[derive(Clone, Debug)]
pub struct OutboxItem {
    pub id: i64,
    pub msg_id: i64,
    pub frame: Vec<u8>,
    pub attempts: u32,
}

pub struct History {
    conn: Connection,
}

impl History {
    pub fn open(path: &Path, key: &[u8; 32]) -> Result<Self> {
        // Create the file private before SQLite does (it would use the umask);
        // SQLite gives its journal files the same mode as the database.
        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(false);
        #[cfg(unix)]
        opts.mode(0o600);
        opts.open(path)?;
        let conn = Connection::open(path)?;
        let hex = Zeroizing::new(data_encoding::HEXLOWER.encode(key));
        let pragma = Zeroizing::new(format!("x'{}'", *hex));
        conn.pragma_update(None, "key", &*pragma)?;
        conn.pragma_update(None, "cipher_memory_security", "ON")?;
        // A wrong key surfaces here, not at open.
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get::<_, i64>(0))?;
        conn.pragma_update(None, "secure_delete", "ON")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS messages(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat TEXT NOT NULL,
                ts INTEGER NOT NULL,
                outgoing INTEGER NOT NULL,
                text TEXT,
                file_name TEXT,
                file_data BLOB,
                status INTEGER NOT NULL DEFAULT 0,
                read INTEGER NOT NULL DEFAULT 0,
                sender TEXT
             );
             CREATE INDEX IF NOT EXISTS messages_chat ON messages(chat, id);
             CREATE TABLE IF NOT EXISTS seen(
                hash BLOB PRIMARY KEY,
                ts INTEGER NOT NULL
             ) WITHOUT ROWID;
             CREATE TABLE IF NOT EXISTS outbox(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat TEXT NOT NULL,
                msg_id INTEGER NOT NULL,
                frame BLOB NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0
             );
             -- A room invitation the user asked for (by using a room invite): who may send it.
             CREATE TABLE IF NOT EXISTS room_expect(
                contact TEXT NOT NULL,
                room BLOB NOT NULL,
                ts INTEGER NOT NULL,
                PRIMARY KEY(contact, room)
             ) WITHOUT ROWID;
             -- Invitations from contacts that wait for the user to accept or decline.
             CREATE TABLE IF NOT EXISTS room_pending(
                room TEXT PRIMARY KEY,
                from_id TEXT NOT NULL,
                ts INTEGER NOT NULL,
                payload BLOB NOT NULL
             ) WITHOUT ROWID;",
        )?;
        // Files created before rooms existed lack the column.
        let has_sender = conn
            .prepare("SELECT 1 FROM pragma_table_info('messages') WHERE name = 'sender'")?
            .exists([])?;
        if !has_sender {
            conn.execute("ALTER TABLE messages ADD COLUMN sender TEXT", [])?;
        }
        Ok(Self { conn })
    }

    /// Store a text or file message. `Hello` is protocol-internal and is rejected.
    pub fn add(&self, chat: &str, ts: u64, outgoing: bool, payload: &Payload) -> Result<i64> {
        self.add_from(chat, ts, outgoing, None, payload)
    }

    /// Like [`add`](Self::add), recording who wrote it (room messages).
    pub fn add_from(&self, chat: &str, ts: u64, outgoing: bool, sender: Option<&str>, payload: &Payload) -> Result<i64> {
        let (text, name, data): (Option<&str>, Option<&str>, Option<&[u8]>) = match payload {
            Payload::Text(t) if t.len() <= MAX_TEXT_BYTES => (Some(t), None, None),
            Payload::File { name, data } if data.len() <= MAX_FILE_BYTES => (None, Some(name), Some(data)),
            _ => return Err(guft_core::Error::Invalid("not a storable message").into()),
        };
        self.conn.execute(
            "INSERT INTO messages(chat, ts, outgoing, text, file_name, file_data, status, read, sender)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![chat, ts as i64, outgoing, text, name, data, if outgoing { 0 } else { 2 }, outgoing, sender],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Store an outgoing message and queue its encrypted frame in one transaction.
    pub fn add_outgoing(&self, chat: &str, ts: u64, payload: &Payload, frame: &[u8]) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        let id = self.add(chat, ts, true, payload)?;
        self.conn.execute("INSERT INTO outbox(chat, msg_id, frame) VALUES (?1, ?2, ?3)", params![chat, id, frame])?;
        tx.commit()?;
        Ok(id)
    }

    /// A room message: one stored message, plus one encrypted frame per member.
    /// With nobody to send to it counts as delivered at once.
    pub fn add_outgoing_multi(&self, chat: &str, ts: u64, payload: &Payload, frames: &[(String, Vec<u8>)]) -> Result<i64> {
        let tx = self.conn.unchecked_transaction()?;
        let id = self.add(chat, ts, true, payload)?;
        for (to, frame) in frames {
            self.conn.execute("INSERT INTO outbox(chat, msg_id, frame) VALUES (?1, ?2, ?3)", params![to, id, frame])?;
        }
        if frames.is_empty() {
            self.set_status(id, Status::Delivered)?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Frames still waiting for this message (0 once everyone has acknowledged it).
    pub fn outbox_remaining(&self, msg_id: i64) -> Result<u32> {
        Ok(self.conn.query_row("SELECT count(*) FROM outbox WHERE msg_id = ?1", [msg_id], |r| r.get::<_, i64>(0))? as u32)
    }

    pub fn message(&self, id: i64) -> Result<Option<StoredMessage>> {
        let sql = format!("SELECT {} FROM messages WHERE id = ?1", Self::COLS);
        Ok(self.conn.query_row(&sql, [id], Self::row).optional()?)
    }

    /// Frames we have already accepted (hash of the whole frame).
    pub fn seen_contains(&self, hash: &[u8; 16]) -> Result<bool> {
        Ok(self.conn.query_row("SELECT 1 FROM seen WHERE hash = ?1", [&hash[..]], |_| Ok(())).optional()?.is_some())
    }

    pub fn seen_insert(&self, hash: &[u8; 16], ts: u64) -> Result<()> {
        self.conn.execute("INSERT OR IGNORE INTO seen(hash, ts) VALUES (?1, ?2)", params![&hash[..], ts as i64])?;
        Ok(())
    }

    pub fn seen_prune(&self, older_than: u64) -> Result<()> {
        self.conn.execute("DELETE FROM seen WHERE ts < ?1", [older_than as i64])?;
        Ok(())
    }

    /// Chats that currently have frames waiting to be delivered.
    pub fn outbox_chats(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT DISTINCT chat FROM outbox")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<StoredMessage> {
        let text: Option<String> = r.get(4)?;
        let body = match text {
            Some(t) => Body::Text(t),
            None => Body::File { name: r.get::<_, Option<String>>(5)?.unwrap_or_default(), size: r.get::<_, i64>(6)? as usize },
        };
        Ok(StoredMessage {
            id: r.get(0)?,
            chat: r.get(1)?,
            ts: r.get::<_, i64>(2)? as u64,
            outgoing: r.get(3)?,
            body,
            status: Status::from_i64(r.get(7)?),
            sender: r.get(8)?,
        })
    }

    const COLS: &'static str = "id, chat, ts, outgoing, text, file_name, length(file_data), status, sender";

    /// Newest-first page of a chat; pass the smallest id seen to get older ones.
    pub fn page(&self, chat: &str, before: Option<i64>, limit: u32) -> Result<Vec<StoredMessage>> {
        let limit = limit.clamp(1, MAX_PAGE);
        let sql = format!(
            "SELECT {} FROM messages WHERE chat = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3",
            Self::COLS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map(params![chat, before.unwrap_or(i64::MAX), limit], Self::row)?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn file_bytes(&self, id: i64) -> Result<Option<Vec<u8>>> {
        Ok(self
            .conn
            .query_row("SELECT file_data FROM messages WHERE id = ?1", [id], |r| r.get::<_, Option<Vec<u8>>>(0))
            .optional()?
            .flatten())
    }

    pub fn summaries(&self) -> Result<Vec<ChatSummary>> {
        let sql = format!(
            "SELECT {cols}, (SELECT count(*) FROM messages u WHERE u.chat = m.chat AND u.read = 0)
             FROM messages m WHERE id IN (SELECT max(id) FROM messages GROUP BY chat) ORDER BY id DESC",
            cols = Self::COLS
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| {
            let last = Self::row(r)?;
            Ok(ChatSummary { chat: last.chat.clone(), last, unread: r.get::<_, i64>(9)? as u32 })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn mark_read(&self, chat: &str) -> Result<()> {
        self.conn.execute("UPDATE messages SET read = 1 WHERE chat = ?1 AND read = 0", [chat])?;
        Ok(())
    }

    pub fn set_status(&self, id: i64, status: Status) -> Result<()> {
        self.conn.execute("UPDATE messages SET status = ?2 WHERE id = ?1", params![id, status as i64])?;
        Ok(())
    }

    /// Delete one message, and cancel it if it is still waiting to be sent.
    pub fn delete_message(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM messages WHERE id = ?1", [id])?;
        self.conn.execute("DELETE FROM outbox WHERE msg_id = ?1 AND msg_id != 0", [id])?;
        Ok(())
    }

    pub fn delete_chat(&self, chat: &str) -> Result<()> {
        self.conn.execute("DELETE FROM messages WHERE chat = ?1", [chat])?;
        self.conn.execute("DELETE FROM outbox WHERE chat = ?1", [chat])?;
        Ok(())
    }

    /// Queue an already-encrypted frame for (re)delivery.
    pub fn outbox_push(&self, chat: &str, msg_id: i64, frame: &[u8]) -> Result<i64> {
        self.conn.execute("INSERT INTO outbox(chat, msg_id, frame) VALUES (?1, ?2, ?3)", params![chat, msg_id, frame])?;
        Ok(self.conn.last_insert_rowid())
    }

    /// Oldest-first frames for one chat; order matters for the ratchet.
    pub fn outbox_for(&self, chat: &str) -> Result<Vec<OutboxItem>> {
        let mut stmt = self.conn.prepare("SELECT id, msg_id, frame, attempts FROM outbox WHERE chat = ?1 ORDER BY id")?;
        let rows = stmt.query_map([chat], |r| {
            Ok(OutboxItem { id: r.get(0)?, msg_id: r.get(1)?, frame: r.get(2)?, attempts: r.get::<_, i64>(3)? as u32 })
        })?;
        Ok(rows.collect::<std::result::Result<_, _>>()?)
    }

    pub fn outbox_bump(&self, id: i64) -> Result<()> {
        self.conn.execute("UPDATE outbox SET attempts = attempts + 1 WHERE id = ?1", [id])?;
        Ok(())
    }

    pub fn outbox_done(&self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM outbox WHERE id = ?1", [id])?;
        Ok(())
    }

    // ───── room invitations ─────

    /// Remember that the user expects a room invitation for `room` from `contact`.
    pub fn expect_room(&self, contact: &str, room: &[u8; 16], ts: u64) -> Result<()> {
        self.conn.execute("INSERT OR REPLACE INTO room_expect(contact, room, ts) VALUES (?1, ?2, ?3)", params![contact, &room[..], ts as i64])?;
        Ok(())
    }

    /// If the user expected this invitation (and not too long ago), use up the expectation.
    pub fn take_expected_room(&self, contact: &str, room: &[u8; 16], not_before: u64) -> Result<bool> {
        let n = self.conn.execute("DELETE FROM room_expect WHERE contact = ?1 AND room = ?2 AND ts >= ?3", params![contact, &room[..], not_before as i64])?;
        Ok(n > 0)
    }

    /// Keep a received invitation until the user decides. `false` if it was already waiting.
    pub fn add_pending_room(&self, room: &str, from: &str, ts: u64, payload: &[u8]) -> Result<bool> {
        let n = self.conn.execute("INSERT OR IGNORE INTO room_pending(room, from_id, ts, payload) VALUES (?1, ?2, ?3, ?4)", params![room, from, ts as i64, payload])?;
        Ok(n > 0)
    }

    /// Oldest first.
    pub fn pending_rooms(&self) -> Result<Vec<PendingRoom>> {
        let mut st = self.conn.prepare("SELECT room, from_id, ts, payload FROM room_pending ORDER BY ts, room")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? as u64, r.get::<_, Vec<u8>>(3)?)))?;
        Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
    }

    pub fn pending_room(&self, room: &str) -> Result<Option<(String, u64, Vec<u8>)>> {
        Ok(self
            .conn
            .query_row("SELECT from_id, ts, payload FROM room_pending WHERE room = ?1", [room], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u64, r.get::<_, Vec<u8>>(2)?)))
            .optional()?)
    }

    pub fn delete_pending_room(&self, room: &str) -> Result<()> {
        self.conn.execute("DELETE FROM room_pending WHERE room = ?1", [room])?;
        Ok(())
    }

    /// Forget everything about a contact's invitations (they were removed).
    pub fn forget_room_invites_from(&self, contact: &str) -> Result<()> {
        self.conn.execute("DELETE FROM room_pending WHERE from_id = ?1", [contact])?;
        self.conn.execute("DELETE FROM room_expect WHERE contact = ?1", [contact])?;
        Ok(())
    }

    /// Drop invitations and expectations older than `before`.
    pub fn prune_room_invites(&self, before: u64) -> Result<()> {
        self.conn.execute("DELETE FROM room_pending WHERE ts < ?1", [before as i64])?;
        self.conn.execute("DELETE FROM room_expect WHERE ts < ?1", [before as i64])?;
        Ok(())
    }

    pub fn close(self) -> Result<()> {
        self.conn.close().map_err(|_| Error::Db)
    }
}
