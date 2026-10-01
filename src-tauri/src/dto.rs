//! Plain data sent to the UI. Never contains keys; message text only where the
//! UI asked for that conversation.

use guft_app::{Body, ChatView, ContactView, Event, RoomInvitation, RoomView, Status, StoredMessage};
use serde::Serialize;

#[derive(Serialize)]
pub struct ContactDto {
    pub id: String,
    pub name: String,
    pub onion: String,
    pub verified: bool,
}

impl From<ContactView> for ContactDto {
    fn from(c: ContactView) -> Self {
        Self {
            id: c.id,
            name: c.name,
            onion: c.onion,
            verified: c.verified,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageDto {
    pub id: i64,
    pub chat: String,
    pub ts: u64,
    pub outgoing: bool,
    /// "text" or "file"
    pub kind: &'static str,
    pub text: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<usize>,
    /// "queued" | "sent" | "delivered" | "failed"
    pub status: &'static str,
    /// Contact id of the author, for room messages only.
    pub sender: Option<String>,
}

impl From<StoredMessage> for MessageDto {
    fn from(m: StoredMessage) -> Self {
        let (kind, text, file_name, file_size) = match m.body {
            Body::Text(t) => ("text", Some(t), None, None),
            Body::File { name, size } => ("file", None, Some(name), Some(size)),
        };
        let status = match m.status {
            Status::Queued => "queued",
            Status::Sent => "sent",
            Status::Delivered => "delivered",
            Status::Failed => "failed",
        };
        Self {
            id: m.id,
            chat: m.chat,
            ts: m.ts,
            outgoing: m.outgoing,
            kind,
            text,
            file_name,
            file_size,
            status,
            sender: m.sender,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomMemberDto {
    pub id: String,
    pub name: String,
    /// We have a session with them, so room messages reach them.
    pub online: bool,
    pub is_creator: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomDto {
    /// Chat key (`r-<hex>`); used with `messages`, `mark_read`, `delete_chat`.
    pub id: String,
    pub name: String,
    /// Contact id of the creator; `mine` says whether that is us.
    pub creator: String,
    pub mine: bool,
    pub open_invites: bool,
    /// Memory only: gone when guft locks or closes.
    pub temp: bool,
    /// A one-to-one temporary chat; `name` is the other person's name.
    pub direct: bool,
    /// The separate identity you appear as in this room, if it has its own (its name).
    pub identity: Option<String>,
    pub members: Vec<RoomMemberDto>,
    pub unread: u32,
    pub last: Option<MessageDto>,
}

impl From<RoomView> for RoomDto {
    fn from(r: RoomView) -> Self {
        let creator = r.creator;
        Self {
            members: r
                .members
                .into_iter()
                .map(|m| RoomMemberDto {
                    is_creator: m.id == creator,
                    id: m.id,
                    name: m.name,
                    online: m.connected,
                })
                .collect(),
            id: r.id,
            name: r.name,
            creator,
            mine: r.mine,
            open_invites: r.open_invites,
            temp: r.temp,
            direct: r.direct,
            identity: r.identity,
            unread: r.unread,
            last: r.last.map(Into::into),
        }
    }
}

/// A contact asked you to join a room; nothing has happened until you accept.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoomInvitationDto {
    pub room: String,
    pub from: String,
    pub from_name: String,
    pub name: String,
    pub members: Vec<String>,
    pub temp: bool,
    pub ts: u64,
}

impl From<RoomInvitation> for RoomInvitationDto {
    fn from(i: RoomInvitation) -> Self {
        Self { room: i.room, from: i.from, from_name: i.from_name, name: i.name, members: i.members, temp: i.temp, ts: i.ts }
    }
}

#[derive(Serialize)]
pub struct ChatDto {
    pub contact: ContactDto,
    pub last: Option<MessageDto>,
    pub unread: u32,
}

impl From<ChatView> for ChatDto {
    fn from(c: ChatView) -> Self {
        Self {
            contact: c.contact.into(),
            last: c.last.map(Into::into),
            unread: c.unread,
        }
    }
}

#[derive(Serialize, Clone)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum EventDto {
    Unlocked,
    Locked,
    NetworkReady { onion: String },
    NetworkError { message: String },
    ContactAdded { id: String, name: String },
    Message { chat: String, id: i64 },
    Delivered { chat: String, msg_id: i64 },
    SendFailed { chat: String, msg_id: i64 },
    RoomChanged { room: String },
    RoomRemoved { room: String },
    RoomInvited { room: String },
}

impl From<Event> for EventDto {
    fn from(e: Event) -> Self {
        match e {
            Event::Unlocked => Self::Unlocked,
            Event::Locked => Self::Locked,
            Event::NetworkReady { onion } => Self::NetworkReady { onion },
            Event::NetworkError(message) => Self::NetworkError { message },
            Event::ContactAdded { id, name } => Self::ContactAdded { id, name },
            Event::Message { chat, id } => Self::Message { chat, id },
            Event::Delivered { chat, msg_id } => Self::Delivered { chat, msg_id },
            Event::SendFailed { chat, msg_id } => Self::SendFailed { chat, msg_id },
            Event::RoomChanged { room } => Self::RoomChanged { room },
            Event::RoomRemoved { room } => Self::RoomRemoved { room },
            Event::RoomInvited { room } => Self::RoomInvited { room },
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusDto {
    pub has_profile: bool,
    pub unlocked: bool,
    pub name: Option<String>,
    pub onion: Option<String>,
    pub online_when_locked: bool,
}

#[derive(Serialize)]
pub struct InviteDto {
    pub invite: String,
    pub code: String,
}
