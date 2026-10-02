//! The only functions the UI can call. Each validates its input, returns plain
//! data, and turns errors into short messages that never contain secrets.

use std::time::Duration;

use guft_core::limits::{MAX_FILE_BYTES, MAX_INVITE_CHARS};
use tauri::State as TauriState;

use crate::dto::*;
use crate::State;

type Res<T> = Result<T, String>;

fn msg(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// A bounded string argument (the UI is untrusted input too).
fn bounded(s: &str, max: usize, what: &str) -> Res<()> {
    if s.len() > max {
        return Err(format!("{what} is too long"));
    }
    Ok(())
}

#[tauri::command]
pub fn status(state: TauriState<'_, State>) -> StatusDto {
    let app = &state.hub;
    StatusDto {
        has_profile: app.has_profile(),
        unlocked: app.is_unlocked(),
        name: app.my_name().ok(),
        onion: app.my_onion(),
        online_when_locked: app.online_when_locked().unwrap_or(false),
    }
}

#[tauri::command]
pub async fn create_profile(state: TauriState<'_, State>, passphrase: String, name: String) -> Res<()> {
    bounded(&passphrase, 1024, "passphrase")?;
    bounded(&name, 48, "name")?;
    state.hub.create_profile(&passphrase, name.trim()).await.map_err(msg)
}

#[tauri::command]
pub async fn unlock(state: TauriState<'_, State>, passphrase: String) -> Res<()> {
    bounded(&passphrase, 1024, "passphrase")?;
    state.hub.unlock(&passphrase).await.map_err(msg)
}

#[tauri::command]
pub async fn lock(state: TauriState<'_, State>) -> Res<()> {
    state.hub.lock().await.map_err(msg)
}

/// The UI reports user activity so the idle timer restarts.
#[tauri::command]
pub fn touch(state: TauriState<'_, State>) {
    state.hub.touch();
}

#[tauri::command]
pub fn chats(state: TauriState<'_, State>) -> Res<Vec<ChatDto>> {
    state.hub.chats().map(|c| c.into_iter().map(Into::into).collect()).map_err(msg)
}

#[tauri::command]
pub fn rooms(state: TauriState<'_, State>) -> Res<Vec<RoomDto>> {
    state.hub.rooms().map(|r| r.into_iter().map(Into::into).collect()).map_err(msg)
}

#[tauri::command]
pub fn messages(state: TauriState<'_, State>, contact: String, before: Option<i64>, limit: Option<u32>) -> Res<Vec<MessageDto>> {
    bounded(&contact, 64, "contact")?;
    state.hub.messages(&contact, before, limit.unwrap_or(50)).map(|m| m.into_iter().map(Into::into).collect()).map_err(msg)
}

#[tauri::command]
pub fn mark_read(state: TauriState<'_, State>, contact: String) -> Res<()> {
    bounded(&contact, 64, "contact")?;
    state.hub.mark_read(&contact).map_err(msg)
}

#[tauri::command]
pub async fn send_text(state: TauriState<'_, State>, contact: String, text: String) -> Res<i64> {
    bounded(&contact, 64, "contact")?;
    bounded(&text, 16 * 1024, "message")?;
    if text.trim().is_empty() {
        return Err("message is empty".into());
    }
    state.hub.send_text(&contact, &text).await.map_err(msg)
}

/// File bytes arrive base64-encoded; the size is checked before decoding.
fn decode_file(data: &str) -> Res<Vec<u8>> {
    if data.len() > MAX_FILE_BYTES.div_ceil(3) * 4 + 8 {
        return Err("file is larger than 1 MB".into());
    }
    let bytes = data_encoding::BASE64.decode(data.as_bytes()).map_err(|_| "bad file data".to_string())?;
    if bytes.len() > MAX_FILE_BYTES {
        return Err("file is larger than 1 MB".into());
    }
    Ok(bytes)
}

#[tauri::command]
pub async fn send_file(state: TauriState<'_, State>, contact: String, name: String, data: String) -> Res<i64> {
    bounded(&contact, 64, "contact")?;
    bounded(&name, 256, "file name")?;
    state.hub.send_file(&contact, &name, decode_file(&data)?).await.map_err(msg)
}

#[tauri::command]
pub fn new_invite(state: TauriState<'_, State>, label: String, ttl_minutes: u64) -> Res<InviteDto> {
    bounded(&label, 48, "label")?;
    let ttl = Duration::from_secs(ttl_minutes.clamp(15, 7 * 24 * 60) * 60);
    state.hub.new_invite(label.trim(), ttl).map(|(invite, code)| InviteDto { invite, code }).map_err(msg)
}

#[tauri::command]
pub async fn add_contact(state: TauriState<'_, State>, invite: String, code: String, identity: Option<String>) -> Res<String> {
    bounded(&invite, MAX_INVITE_CHARS, "invite")?;
    bounded(&code, 64, "code")?;
    let identity = identity.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
    if let Some(n) = &identity {
        bounded(n, 48, "identity name")?;
    }
    state.hub.add_contact(&invite, &code, identity.as_deref()).await.map_err(msg)
}

/// Whether an invite leads into a room (only those can be used with a separate identity).
#[tauri::command]
pub fn invite_is_room(state: TauriState<'_, State>, invite: String) -> bool {
    invite.len() <= MAX_INVITE_CHARS && state.hub.invite_is_room(&invite)
}

#[tauri::command]
pub fn safety_number(state: TauriState<'_, State>, contact: String) -> Res<String> {
    bounded(&contact, 64, "contact")?;
    state.hub.safety_number(&contact).map_err(msg)
}

#[tauri::command]
pub fn set_verified(state: TauriState<'_, State>, contact: String, verified: bool) -> Res<()> {
    bounded(&contact, 64, "contact")?;
    state.hub.set_verified(&contact, verified).map_err(msg)
}

#[tauri::command]
pub fn remove_contact(state: TauriState<'_, State>, contact: String) -> Res<()> {
    bounded(&contact, 64, "contact")?;
    state.hub.remove_contact(&contact).map_err(msg)
}

#[tauri::command]
pub fn delete_message(state: TauriState<'_, State>, msg_id: i64) -> Res<()> {
    state.hub.delete_message(msg_id).map_err(msg)
}

#[tauri::command]
pub fn rename_contact(state: TauriState<'_, State>, contact: String, name: String) -> Res<()> {
    bounded(&contact, 64, "contact")?;
    bounded(&name, 48, "name")?;
    state.hub.rename_contact(&contact, &name).map_err(msg)
}

#[tauri::command]
pub fn delete_chat(state: TauriState<'_, State>, contact: String) -> Res<()> {
    bounded(&contact, 64, "contact")?;
    state.hub.delete_chat(&contact).map_err(msg)
}

// ───────────── rooms ─────────────

#[tauri::command]
pub async fn create_room(state: TauriState<'_, State>, name: String, open_invites: bool, temp: Option<bool>, identity: Option<String>) -> Res<String> {
    bounded(&name, 48, "name")?;
    if name.trim().is_empty() {
        return Err("room needs a name".into());
    }
    let identity = identity.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
    if let Some(n) = &identity {
        bounded(n, 48, "identity name")?;
    }
    state.hub.create_room(name.trim(), open_invites, temp.unwrap_or(false), identity.as_deref()).await.map_err(msg)
}

#[tauri::command]
pub fn room_invitations(state: TauriState<'_, State>) -> Res<Vec<RoomInvitationDto>> {
    state.hub.room_invitations().map(|v| v.into_iter().map(Into::into).collect()).map_err(msg)
}

/// Join a room a contact invited you to. Returns the room's chat id.
#[tauri::command]
pub fn accept_room_invite(state: TauriState<'_, State>, room: String) -> Res<String> {
    bounded(&room, 64, "room")?;
    state.hub.accept_room_invite(&room).map_err(msg)
}

#[tauri::command]
pub fn decline_room_invite(state: TauriState<'_, State>, room: String) -> Res<()> {
    bounded(&room, 64, "room")?;
    state.hub.decline_room_invite(&room).map_err(msg)
}

/// One tap from a one-to-one chat: a memory-only chat with that contact.
#[tauri::command]
pub fn start_temp_chat(state: TauriState<'_, State>, contact: String) -> Res<String> {
    bounded(&contact, 64, "contact")?;
    state.hub.start_temp_chat(&contact).map_err(msg)
}

/// An invite that also puts whoever uses it into the room.
#[tauri::command]
pub fn room_invite(state: TauriState<'_, State>, room: String, label: String, ttl_minutes: u64) -> Res<InviteDto> {
    bounded(&room, 64, "room")?;
    bounded(&label, 48, "label")?;
    let ttl = Duration::from_secs(ttl_minutes.clamp(15, 7 * 24 * 60) * 60);
    state.hub.new_room_invite(&room, label.trim(), ttl).map(|(invite, code)| InviteDto { invite, code }).map_err(msg)
}

#[tauri::command]
pub fn add_to_room(state: TauriState<'_, State>, room: String, contact: String) -> Res<()> {
    bounded(&room, 64, "room")?;
    bounded(&contact, 64, "contact")?;
    state.hub.add_contact_to_room(&room, &contact).map_err(msg)
}

#[tauri::command]
pub fn leave_room(state: TauriState<'_, State>, room: String) -> Res<()> {
    bounded(&room, 64, "room")?;
    state.hub.leave_room(&room).map_err(msg)
}

#[tauri::command]
pub fn remove_member(state: TauriState<'_, State>, room: String, member: String) -> Res<()> {
    bounded(&room, 64, "room")?;
    bounded(&member, 64, "member")?;
    state.hub.remove_room_member(&room, &member).map_err(msg)
}

#[tauri::command]
pub fn rename_room(state: TauriState<'_, State>, room: String, name: String) -> Res<()> {
    bounded(&room, 64, "room")?;
    bounded(&name, 48, "name")?;
    if name.trim().is_empty() {
        return Err("room needs a name".into());
    }
    state.hub.rename_room(&room, name.trim()).map_err(msg)
}

#[tauri::command]
pub async fn send_room_text(state: TauriState<'_, State>, room: String, text: String) -> Res<i64> {
    bounded(&room, 64, "room")?;
    bounded(&text, 16 * 1024, "message")?;
    if text.trim().is_empty() {
        return Err("message is empty".into());
    }
    state.hub.send_room_text(&room, &text).await.map_err(msg)
}

#[tauri::command]
pub async fn send_room_file(state: TauriState<'_, State>, room: String, name: String, data: String) -> Res<i64> {
    bounded(&room, 64, "room")?;
    bounded(&name, 256, "file name")?;
    state.hub.send_room_file(&room, &name, decode_file(&data)?).await.map_err(msg)
}

/// Saves a received file into the downloads folder; returns where it went.
#[tauri::command]
pub fn save_file(state: TauriState<'_, State>, msg_id: i64) -> Res<String> {
    let dir = state.downloads.clone();
    state.hub.save_file(msg_id, &dir).map(|p| p.display().to_string()).map_err(msg)
}

/// The bytes of a stored file message, base64, so the UI can play a voice note. At most 1 MB.
#[tauri::command]
pub fn file_bytes(state: TauriState<'_, State>, msg_id: i64) -> Res<String> {
    state.hub.file_bytes(msg_id).map(|(_, data)| data_encoding::BASE64.encode(&data)).map_err(msg)
}

#[tauri::command]
pub fn set_online_when_locked(state: TauriState<'_, State>, on: bool) -> Res<()> {
    state.hub.set_online_when_locked(on).map_err(msg)
}

#[tauri::command]
pub fn set_idle_minutes(state: TauriState<'_, State>, minutes: u64) {
    state.hub.set_idle_timeout(Duration::from_secs(minutes.clamp(1, 240) * 60));
}
