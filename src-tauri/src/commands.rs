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
    let app = &state.app;
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
    state.app.create_profile(&passphrase, name.trim()).await.map_err(msg)
}

#[tauri::command]
pub async fn unlock(state: TauriState<'_, State>, passphrase: String) -> Res<()> {
    bounded(&passphrase, 1024, "passphrase")?;
    state.app.unlock(&passphrase).await.map_err(msg)
}

#[tauri::command]
pub async fn lock(state: TauriState<'_, State>) -> Res<()> {
    state.app.lock().await.map_err(msg)
}

/// The UI reports user activity so the idle timer restarts.
#[tauri::command]
pub fn touch(state: TauriState<'_, State>) {
    state.app.touch();
}

#[tauri::command]
pub fn chats(state: TauriState<'_, State>) -> Res<Vec<ChatDto>> {
    state.app.chats().map(|c| c.into_iter().map(Into::into).collect()).map_err(msg)
}

#[tauri::command]
pub fn rooms(state: TauriState<'_, State>) -> Res<Vec<RoomDto>> {
    state.app.rooms().map(|r| r.into_iter().map(Into::into).collect()).map_err(msg)
}

#[tauri::command]
pub fn messages(state: TauriState<'_, State>, contact: String, before: Option<i64>, limit: Option<u32>) -> Res<Vec<MessageDto>> {
    bounded(&contact, 64, "contact")?;
    state.app.messages(&contact, before, limit.unwrap_or(50)).map(|m| m.into_iter().map(Into::into).collect()).map_err(msg)
}

#[tauri::command]
pub fn mark_read(state: TauriState<'_, State>, contact: String) -> Res<()> {
    bounded(&contact, 64, "contact")?;
    state.app.mark_read(&contact).map_err(msg)
}

#[tauri::command]
pub async fn send_text(state: TauriState<'_, State>, contact: String, text: String) -> Res<i64> {
    bounded(&contact, 64, "contact")?;
    bounded(&text, 16 * 1024, "message")?;
    if text.trim().is_empty() {
        return Err("message is empty".into());
    }
    state.app.send_text(&contact, &text).await.map_err(msg)
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
    state.app.send_file(&contact, &name, decode_file(&data)?).await.map_err(msg)
}

#[tauri::command]
pub fn new_invite(state: TauriState<'_, State>, label: String, ttl_minutes: u64) -> Res<InviteDto> {
    bounded(&label, 48, "label")?;
    let ttl = Duration::from_secs(ttl_minutes.clamp(15, 7 * 24 * 60) * 60);
    state.app.new_invite(label.trim(), ttl).map(|(invite, code)| InviteDto { invite, code }).map_err(msg)
}

#[tauri::command]
pub async fn add_contact(state: TauriState<'_, State>, invite: String, code: String) -> Res<String> {
    bounded(&invite, MAX_INVITE_CHARS, "invite")?;
    bounded(&code, 64, "code")?;
    state.app.add_contact(&invite, &code).await.map_err(msg)
}

#[tauri::command]
pub fn safety_number(state: TauriState<'_, State>, contact: String) -> Res<String> {
    bounded(&contact, 64, "contact")?;
    state.app.safety_number(&contact).map_err(msg)
}

#[tauri::command]
pub fn set_verified(state: TauriState<'_, State>, contact: String, verified: bool) -> Res<()> {
    bounded(&contact, 64, "contact")?;
    state.app.set_verified(&contact, verified).map_err(msg)
}

#[tauri::command]
pub fn remove_contact(state: TauriState<'_, State>, contact: String) -> Res<()> {
    bounded(&contact, 64, "contact")?;
    state.app.remove_contact(&contact).map_err(msg)
}

#[tauri::command]
pub fn delete_message(state: TauriState<'_, State>, msg_id: i64) -> Res<()> {
    state.app.delete_message(msg_id).map_err(msg)
}

#[tauri::command]
pub fn rename_contact(state: TauriState<'_, State>, contact: String, name: String) -> Res<()> {
    bounded(&contact, 64, "contact")?;
    bounded(&name, 48, "name")?;
    state.app.rename_contact(&contact, &name).map_err(msg)
}

#[tauri::command]
pub fn delete_chat(state: TauriState<'_, State>, contact: String) -> Res<()> {
    bounded(&contact, 64, "contact")?;
    state.app.delete_chat(&contact).map_err(msg)
}

// ───────────── rooms ─────────────

#[tauri::command]
pub fn create_room(state: TauriState<'_, State>, name: String, open_invites: bool, temp: Option<bool>) -> Res<String> {
    bounded(&name, 48, "name")?;
    if name.trim().is_empty() {
        return Err("room needs a name".into());
    }
    if temp.unwrap_or(false) {
        state.app.create_temp_room(name.trim(), open_invites).map_err(msg)
    } else {
        state.app.create_room(name.trim(), open_invites).map_err(msg)
    }
}

/// One tap from a one-to-one chat: a memory-only chat with that contact.
#[tauri::command]
pub fn start_temp_chat(state: TauriState<'_, State>, contact: String) -> Res<String> {
    bounded(&contact, 64, "contact")?;
    state.app.start_temp_chat(&contact).map_err(msg)
}

/// An invite that also puts whoever uses it into the room.
#[tauri::command]
pub fn room_invite(state: TauriState<'_, State>, room: String, label: String, ttl_minutes: u64) -> Res<InviteDto> {
    bounded(&room, 64, "room")?;
    bounded(&label, 48, "label")?;
    let ttl = Duration::from_secs(ttl_minutes.clamp(15, 7 * 24 * 60) * 60);
    state.app.new_room_invite(&room, label.trim(), ttl).map(|(invite, code)| InviteDto { invite, code }).map_err(msg)
}

#[tauri::command]
pub fn add_to_room(state: TauriState<'_, State>, room: String, contact: String) -> Res<()> {
    bounded(&room, 64, "room")?;
    bounded(&contact, 64, "contact")?;
    state.app.add_contact_to_room(&room, &contact).map_err(msg)
}

#[tauri::command]
pub fn leave_room(state: TauriState<'_, State>, room: String) -> Res<()> {
    bounded(&room, 64, "room")?;
    state.app.leave_room(&room).map_err(msg)
}

#[tauri::command]
pub fn remove_member(state: TauriState<'_, State>, room: String, member: String) -> Res<()> {
    bounded(&room, 64, "room")?;
    bounded(&member, 64, "member")?;
    state.app.remove_room_member(&room, &member).map_err(msg)
}

#[tauri::command]
pub fn rename_room(state: TauriState<'_, State>, room: String, name: String) -> Res<()> {
    bounded(&room, 64, "room")?;
    bounded(&name, 48, "name")?;
    if name.trim().is_empty() {
        return Err("room needs a name".into());
    }
    state.app.rename_room(&room, name.trim()).map_err(msg)
}

#[tauri::command]
pub async fn send_room_text(state: TauriState<'_, State>, room: String, text: String) -> Res<i64> {
    bounded(&room, 64, "room")?;
    bounded(&text, 16 * 1024, "message")?;
    if text.trim().is_empty() {
        return Err("message is empty".into());
    }
    state.app.send_room_text(&room, &text).await.map_err(msg)
}

#[tauri::command]
pub async fn send_room_file(state: TauriState<'_, State>, room: String, name: String, data: String) -> Res<i64> {
    bounded(&room, 64, "room")?;
    bounded(&name, 256, "file name")?;
    state.app.send_room_file(&room, &name, decode_file(&data)?).await.map_err(msg)
}

/// Saves a received file into the downloads folder; returns where it went.
#[tauri::command]
pub fn save_file(state: TauriState<'_, State>, msg_id: i64) -> Res<String> {
    let dir = crate::sandbox::Dirs::resolve().downloads;
    state.app.save_file(msg_id, &dir).map(|p| p.display().to_string()).map_err(msg)
}

/// The bytes of a stored file message, base64, so the UI can play a voice note. At most 1 MB.
#[tauri::command]
pub fn file_bytes(state: TauriState<'_, State>, msg_id: i64) -> Res<String> {
    state.app.file_bytes(msg_id).map(|(_, data)| data_encoding::BASE64.encode(&data)).map_err(msg)
}

#[tauri::command]
pub fn set_online_when_locked(state: TauriState<'_, State>, on: bool) -> Res<()> {
    state.app.set_online_when_locked(on).map_err(msg)
}

#[tauri::command]
pub fn set_idle_minutes(state: TauriState<'_, State>, minutes: u64) {
    state.app.set_idle_timeout(Duration::from_secs(minutes.clamp(1, 240) * 60));
}
