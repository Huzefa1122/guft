//! Chat history in a SQLCipher database, keyed by a vault subkey.

use std::fs::OpenOptions;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use nochat_core::limits::{MAX_FILE_BYTES, MAX_TEXT_BYTES};
use nochat_core::Payload;
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
}

#[derive(Clone, Debug)]
pub struct ChatSummary {
    pub chat: String,
    pub last: StoredMessage,
    pub unread: u32,
}

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
        OpenOptions::new().write(true).create(true).truncate(false).mode(0o600).open(path)?;
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
                read INTEGER NOT NULL DEFAULT 0
             );
             CREATE INDEX IF NOT EXISTS messages_chat ON messages(chat, id);
             CREATE TABLE IF NOT EXISTS outbox(
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                chat TEXT NOT NULL,
                msg_id INTEGER NOT NULL,
                frame BLOB NOT NULL,
                attempts INTEGER NOT NULL DEFAULT 0
             );",
        )?;
        Ok(Self { conn })
    }

    /// Store a text or file message. `Hello` is protocol-internal and is rejected.
    pub fn add(&self, chat: &str, ts: u64, outgoing: bool, payload: &Payload) -> Result<i64> {
        let (text, name, data): (Option<&str>, Option<&str>, Option<&[u8]>) = match payload {
            Payload::Text(t) if t.len() <= MAX_TEXT_BYTES => (Some(t), None, None),
            Payload::File { name, data } if data.len() <= MAX_FILE_BYTES => (None, Some(name), Some(data)),
            _ => return Err(nochat_core::Error::Invalid("not a storable message").into()),
        };
        self.conn.execute(
            "INSERT INTO messages(chat, ts, outgoing, text, file_name, file_data, status, read)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![chat, ts as i64, outgoing, text, name, data, if outgoing { 0 } else { 2 }, outgoing],
        )?;
        Ok(self.conn.last_insert_rowid())
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
        })
    }

    const COLS: &'static str = "id, chat, ts, outgoing, text, file_name, length(file_data), status";

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
            Ok(ChatSummary { chat: last.chat.clone(), last, unread: r.get::<_, i64>(8)? as u32 })
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

    pub fn close(self) -> Result<()> {
        self.conn.close().map_err(|_| Error::Db)
    }
}
