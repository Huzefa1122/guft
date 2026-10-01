//! Separate identities.
//!
//! A [`Hub`] is one profile with any number of extra identities. Your main identity is
//! the profile itself. Every other identity ("persona") is a complete profile of its own
//! (own keys, own Tor address, own contacts, own history) kept in a sub-directory and
//! protected by a random secret that is stored inside the main profile. Nothing is shared
//! between identities that a room member could use to link them.
//!
//! A persona lives exactly as long as the room it was made for: it is created when you make
//! or join a room "as a new identity", and it is erased when you leave that room, are
//! removed from it, or the room is gone.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use data_encoding::HEXLOWER;
use guft_core::engine::now_secs;
use guft_core::payload::check_display_name;
use guft_core::Engine;
use guft_store::{Profile, StoredMessage};
use rand::rngs::OsRng;
use rand::TryRngCore;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use crate::{App, AppError, AppOptions, ChatView, ContactView, Event, NetworkBackend, Result, RoomInvitation, RoomView};

/// Most separate identities at once (each runs its own Tor client).
pub const MAX_PERSONAS: usize = 8;
/// Persona numbers are folded into message ids, which must stay below 2^53 for the UI.
const MAX_PERSONA_ID: u32 = 2000;
const PERSONA_SHIFT: u32 = 42;
const REGISTRY: &str = "personas";
/// A persona made to join a room by invite has this long to be let in before it is dropped.
const JOIN_GRACE_SECS: u64 = 8 * 24 * 3600;
/// A persona made to create a room has a room straight away; this only covers a failed start.
const CREATE_GRACE_SECS: u64 = 120;

fn invalid(what: &'static str) -> AppError {
    AppError::Core(guft_core::Error::Invalid(what))
}

// ───────────── ids ─────────────

/// The chat id the UI sees: unchanged for the main identity, `p<n>~<id>` for a persona.
pub fn ns_chat(persona: u32, local: &str) -> String {
    if persona == 0 {
        local.to_owned()
    } else {
        format!("p{persona}~{local}")
    }
}

pub fn split_chat(chat: &str) -> (u32, &str) {
    if let Some(rest) = chat.strip_prefix('p') {
        if let Some((n, local)) = rest.split_once('~') {
            if let Ok(n) = n.parse::<u32>() {
                if n > 0 && n <= MAX_PERSONA_ID {
                    return (n, local);
                }
            }
        }
    }
    (0, chat)
}

/// Message ids carry the persona number in their upper bits: `persona * 2^42 + id`.
pub fn ns_msg(persona: u32, id: i64) -> i64 {
    (i64::from(persona) << PERSONA_SHIFT) + id
}

pub fn split_msg(id: i64) -> (u32, i64) {
    if id < 0 {
        return (0, id);
    }
    ((id >> PERSONA_SHIFT) as u32, id & ((1 << PERSONA_SHIFT) - 1))
}

fn map_message(persona: u32, mut m: StoredMessage) -> StoredMessage {
    m.id = ns_msg(persona, m.id);
    m.chat = ns_chat(persona, &m.chat);
    m
}

fn map_event(persona: u32, e: Event) -> Option<Event> {
    Some(match e {
        Event::Message { chat, id } => Event::Message { chat: ns_chat(persona, &chat), id: ns_msg(persona, id) },
        Event::Delivered { chat, msg_id } => Event::Delivered { chat: ns_chat(persona, &chat), msg_id: ns_msg(persona, msg_id) },
        Event::SendFailed { chat, msg_id } => Event::SendFailed { chat: ns_chat(persona, &chat), msg_id: ns_msg(persona, msg_id) },
        Event::RoomChanged { room } => Event::RoomChanged { room: ns_chat(persona, &room) },
        Event::RoomRemoved { room } => Event::RoomRemoved { room: ns_chat(persona, &room) },
        Event::RoomInvited { room } => Event::RoomInvited { room: ns_chat(persona, &room) },
        // Lifecycle and contact events of a persona are not the user's business.
        _ => return None,
    })
}

// ───────────── registry ─────────────

#[derive(Clone, Serialize, Deserialize)]
struct PersonaRec {
    id: u32,
    /// Protects the persona's profile. Random, never typed, and itself stored sealed.
    secret: [u8; 32],
    alias: String,
    created: u64,
    /// Until then a persona without rooms is kept (an invite may still be answered).
    grace_until: u64,
}

struct Persona<B: NetworkBackend> {
    app: App<B>,
    alias: String,
    forwarder: JoinHandle<()>,
}

struct Inner<B: NetworkBackend> {
    dir: PathBuf,
    opts: AppOptions,
    factory: Box<dyn Fn(&Path) -> B + Send + Sync>,
    main: App<B>,
    personas: Mutex<BTreeMap<u32, Persona<B>>>,
    events: broadcast::Sender<Event>,
    /// Serialises creating, restoring and deleting personas.
    gate: tokio::sync::Mutex<()>,
}

/// The main identity plus its separate identities. Cheap to clone.
pub struct Hub<B: NetworkBackend> {
    inner: Arc<Inner<B>>,
}

impl<B: NetworkBackend> Clone for Hub<B> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

fn persona_options(base: &AppOptions) -> AppOptions {
    // The hub locks personas together with the main identity, so they never lock by themselves.
    AppOptions { idle_timeout: Duration::from_secs(10 * 365 * 24 * 3600), ..base.clone() }
}

fn copy_dir(from: &Path, to: &Path) {
    let Ok(rd) = std::fs::read_dir(from) else { return };
    let _ = std::fs::create_dir_all(to);
    for e in rd.flatten() {
        let (src, dst) = (e.path(), to.join(e.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst);
        } else {
            let _ = std::fs::copy(&src, &dst);
        }
    }
}

impl<B: NetworkBackend> Hub<B> {
    /// `factory` makes the network backend for a profile directory (the main one or a persona's).
    pub fn new(dir: PathBuf, opts: AppOptions, factory: impl Fn(&Path) -> B + Send + Sync + 'static) -> Self {
        let main = App::new(dir.clone(), factory(&dir), opts.clone());
        let (events, _) = broadcast::channel(256);
        let hub = Self {
            inner: Arc::new(Inner { dir, opts, factory: Box::new(factory), main, personas: Mutex::new(BTreeMap::new()), events, gate: tokio::sync::Mutex::new(()) }),
        };
        hub.spawn_main_forwarder();
        hub
    }

    /// The main identity's app, for everything that is not about rooms or ids.
    pub fn main(&self) -> &App<B> {
        &self.inner.main
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    fn emit(&self, e: Event) {
        let _ = self.inner.events.send(e);
    }

    fn persona_dir(&self, id: u32) -> PathBuf {
        self.inner.dir.join("personas").join(id.to_string())
    }

    // ───────────── background tasks ─────────────

    fn spawn_main_forwarder(&self) {
        let weak: Weak<Inner<B>> = Arc::downgrade(&self.inner);
        let mut rx = self.inner.main.subscribe();
        tokio::spawn(async move {
            loop {
                let e = match rx.recv().await {
                    Ok(e) => e,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let Some(inner) = weak.upgrade() else { break };
                let hub = Hub { inner };
                hub.emit(e.clone());
                match e {
                    // The main profile is open: open the others too.
                    Event::Unlocked => {
                        tokio::spawn(async move { hub.restore_personas().await });
                    }
                    // Locked, by the user or by the idle timer: lock every identity.
                    Event::Locked => {
                        tokio::spawn(async move { hub.lock_personas().await });
                    }
                    _ => {}
                }
            }
        });
    }

    fn spawn_persona_forwarder(&self, id: u32, app: &App<B>) -> JoinHandle<()> {
        let weak: Weak<Inner<B>> = Arc::downgrade(&self.inner);
        let mut rx = app.subscribe();
        tokio::spawn(async move {
            loop {
                let e = match rx.recv().await {
                    Ok(e) => e,
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };
                let Some(inner) = weak.upgrade() else { break };
                let hub = Hub { inner };
                let gone = matches!(e, Event::RoomRemoved { .. });
                if std::env::var_os("GUFT_DEBUG_PERSONA").is_some() {
                    eprintln!("persona {id}: {e:?}");
                }
                if let Some(mapped) = map_event(id, e) {
                    hub.emit(mapped);
                }
                // The room this identity was made for is over: erase the identity.
                if gone {
                    tokio::spawn(async move { hub.erase_when_sent(id).await });
                }
            }
        })
    }

    // ───────────── registry ─────────────

    fn load_registry(&self) -> Result<Vec<PersonaRec>> {
        match self.inner.main.aux_read(REGISTRY)? {
            Some(bytes) => postcard::from_bytes(&bytes).map_err(|_| invalid("identity list is damaged")),
            None => Ok(Vec::new()),
        }
    }

    fn save_registry(&self, recs: &[PersonaRec]) -> Result<()> {
        let bytes = postcard::to_allocvec(recs).map_err(|_| invalid("cannot save identities"))?;
        self.inner.main.aux_write(REGISTRY, &bytes)
    }

    fn app_of(&self, id: u32) -> Option<App<B>> {
        self.inner.personas.lock().expect("poisoned").get(&id).map(|p| p.app.clone())
    }

    fn all_personas(&self) -> Vec<(u32, String, App<B>)> {
        self.inner.personas.lock().expect("poisoned").iter().map(|(id, p)| (*id, p.alias.clone(), p.app.clone())).collect()
    }

    // ───────────── persona lifecycle ─────────────

    /// Open every saved identity (called when the main profile opens).
    async fn restore_personas(&self) {
        let _gate = self.inner.gate.lock().await;
        let Ok(recs) = self.load_registry() else { return };
        let mut keep = recs.clone();
        let mut dirty = false;
        for rec in &recs {
            let dir = self.persona_dir(rec.id);
            if !Profile::exists(&dir) {
                keep.retain(|r| r.id != rec.id);
                dirty = true;
                continue;
            }
            let existing = self.app_of(rec.id);
            let app = match existing {
                Some(app) => app,
                None => {
                    let app = App::new(dir.clone(), (self.inner.factory)(&dir), persona_options(&self.inner.opts));
                    let forwarder = self.spawn_persona_forwarder(rec.id, &app);
                    self.inner.personas.lock().expect("poisoned").insert(rec.id, Persona { app: app.clone(), alias: rec.alias.clone(), forwarder });
                    app
                }
            };
            if app.unlock_in_background(&HEXLOWER.encode(&rec.secret)).await.is_err() {
                continue;
            }
            let rooms = app.rooms().unwrap_or_default();
            let invites = app.room_invitations().unwrap_or_default();
            if rooms.is_empty() && invites.is_empty() && now_secs() > rec.grace_until {
                let (hub, id) = (self.clone(), rec.id);
                tokio::spawn(async move { hub.erase_when_sent(id).await });
                continue;
            }
            for r in rooms {
                self.emit(Event::RoomChanged { room: ns_chat(rec.id, &r.id) });
            }
        }
        if dirty {
            let _ = self.save_registry(&keep);
        }
    }

    async fn lock_personas(&self) {
        for (_, _, app) in self.all_personas() {
            let _ = app.lock().await;
        }
    }

    /// Make a new identity named `alias`. Returns its number and app (already open).
    async fn new_persona(&self, alias: &str, grace_secs: u64) -> Result<(u32, App<B>)> {
        let alias = alias.trim();
        if alias.is_empty() {
            return Err(invalid("choose a name for this identity"));
        }
        check_display_name(alias)?;
        let _gate = self.inner.gate.lock().await;
        let mut recs = self.load_registry()?;
        if recs.len() >= MAX_PERSONAS {
            return Err(AppError::Core(guft_core::Error::Limit("too many separate identities")));
        }
        let id = (1..=MAX_PERSONA_ID).find(|i| !recs.iter().any(|r| r.id == *i)).ok_or(AppError::Core(guft_core::Error::Limit("too many separate identities")))?;
        let mut secret = [0u8; 32];
        OsRng.try_fill_bytes(&mut secret).map_err(|_| invalid("no randomness"))?;
        let dir = self.persona_dir(id);
        // Tor's downloaded directory documents are public: reuse them so the new identity connects faster.
        copy_dir(&self.inner.dir.join("tor/cache"), &dir.join("tor/cache"));
        let app = App::new(dir.clone(), (self.inner.factory)(&dir), persona_options(&self.inner.opts));
        if let Err(e) = app.create_profile_for_random_secret(&HEXLOWER.encode(&secret), alias).await {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
        recs.push(PersonaRec { id, secret, alias: alias.to_owned(), created: now_secs(), grace_until: now_secs() + grace_secs });
        if let Err(e) = self.save_registry(&recs) {
            let _ = app.lock().await;
            let _ = std::fs::remove_dir_all(&dir);
            return Err(e);
        }
        let forwarder = self.spawn_persona_forwarder(id, &app);
        self.inner.personas.lock().expect("poisoned").insert(id, Persona { app: app.clone(), alias: alias.to_owned(), forwarder });
        Ok((id, app))
    }

    /// Erase an identity: stop it, delete its files, forget it. The gate must be held.
    async fn delete_persona_locked(&self, id: u32) {
        let removed = self.inner.personas.lock().expect("poisoned").remove(&id);
        if let Some(p) = removed {
            p.forwarder.abort();
            let _ = p.app.lock().await;
        }
        let _ = std::fs::remove_dir_all(self.persona_dir(id));
        if let Ok(mut recs) = self.load_registry() {
            recs.retain(|r| r.id != id);
            let _ = self.save_registry(&recs);
        }
        self.emit(Event::RoomChanged { room: format!("p{id}~") });
    }

    /// Erase an identity that has no room left, but first let it finish sending (a leave
    /// notice, for instance) so the others learn it is gone. Gives up waiting after ten minutes.
    async fn erase_when_sent(&self, id: u32) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(600);
        loop {
            let Some(app) = self.app_of(id) else { return };
            match app.has_unsent() {
                Ok(false) => break,
                // Locked meanwhile: the next unlock will look at it again.
                Err(_) => return,
                Ok(true) => {}
            }
            if tokio::time::Instant::now() >= deadline {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        self.delete_persona_if_unused(id).await;
    }

    async fn delete_persona_if_unused(&self, id: u32) {
        let _gate = self.inner.gate.lock().await;
        let Some(app) = self.app_of(id) else { return };
        let unused = app.rooms().map(|r| r.is_empty()).unwrap_or(false) && app.room_invitations().map(|i| i.is_empty()).unwrap_or(true);
        if unused {
            self.delete_persona_locked(id).await;
        }
    }

    /// Whether an invite leads into a room (only those can be used with a new identity).
    pub fn invite_is_room(&self, invite: &str) -> bool {
        Engine::invite_room(invite).is_some()
    }

    fn route(&self, chat: &str) -> Result<(u32, String, App<B>)> {
        let (id, local) = split_chat(chat);
        if id == 0 {
            return Ok((0, local.to_owned(), self.inner.main.clone()));
        }
        let app = self.app_of(id).ok_or_else(|| invalid("that identity is not available"))?;
        Ok((id, local.to_owned(), app))
    }

    fn route_msg(&self, msg_id: i64) -> Result<(u32, i64, App<B>)> {
        let (id, local) = split_msg(msg_id);
        if id == 0 {
            return Ok((0, local, self.inner.main.clone()));
        }
        let app = self.app_of(id).ok_or_else(|| invalid("that identity is not available"))?;
        Ok((id, local, app))
    }

    // ───────────── lifecycle ─────────────

    pub fn has_profile(&self) -> bool {
        self.inner.main.has_profile()
    }

    pub fn is_unlocked(&self) -> bool {
        self.inner.main.is_unlocked()
    }

    pub async fn create_profile(&self, passphrase: &str, display_name: &str) -> Result<()> {
        self.inner.main.create_profile(passphrase, display_name).await
    }

    pub async fn unlock(&self, passphrase: &str) -> Result<()> {
        self.inner.main.unlock(passphrase).await
    }

    /// Lock every identity, then the main profile.
    pub async fn lock(&self) -> Result<()> {
        self.lock_personas().await;
        self.inner.main.lock().await
    }

    pub fn touch(&self) {
        self.inner.main.touch();
    }

    pub fn my_name(&self) -> Result<String> {
        self.inner.main.my_name()
    }

    pub fn my_onion(&self) -> Option<String> {
        self.inner.main.my_onion()
    }

    pub fn online_when_locked(&self) -> Result<bool> {
        self.inner.main.online_when_locked()
    }

    pub fn set_online_when_locked(&self, on: bool) -> Result<()> {
        self.inner.main.set_online_when_locked(on)
    }

    pub fn set_idle_timeout(&self, t: Duration) {
        self.inner.main.set_idle_timeout(t);
    }

    // ───────────── main identity only: contacts and one-to-one chats ─────────────

    pub fn contacts(&self) -> Result<Vec<ContactView>> {
        self.inner.main.contacts()
    }

    pub fn chats(&self) -> Result<Vec<ChatView>> {
        self.inner.main.chats()
    }

    pub fn new_invite(&self, label: &str, ttl: Duration) -> Result<(String, String)> {
        self.inner.main.new_invite(label, ttl)
    }

    pub fn safety_number(&self, contact: &str) -> Result<String> {
        self.inner.main.safety_number(contact)
    }

    pub fn set_verified(&self, contact: &str, verified: bool) -> Result<()> {
        self.inner.main.set_verified(contact, verified)
    }

    pub fn remove_contact(&self, contact: &str) -> Result<()> {
        self.inner.main.remove_contact(contact)
    }

    pub fn rename_contact(&self, contact: &str, name: &str) -> Result<()> {
        self.inner.main.rename_contact(contact, name)
    }

    pub async fn send_text(&self, contact: &str, text: &str) -> Result<i64> {
        self.inner.main.send_text(contact, text).await
    }

    pub async fn send_file(&self, contact: &str, name: &str, data: Vec<u8>) -> Result<i64> {
        self.inner.main.send_file(contact, name, data).await
    }

    pub fn start_temp_chat(&self, contact: &str) -> Result<String> {
        self.inner.main.start_temp_chat(contact)
    }

    /// Import an invite. With `identity`, do it as a brand-new identity of that name: only a
    /// room invite can be used that way, and the identity exists only for that room.
    pub async fn add_contact(&self, invite: &str, code: &str, identity: Option<&str>) -> Result<String> {
        let Some(alias) = identity else { return self.inner.main.add_contact(invite, code).await };
        if !self.invite_is_room(invite) {
            return Err(invalid("a new identity can only be used with a room invite"));
        }
        let (id, app) = self.new_persona(alias, JOIN_GRACE_SECS).await?;
        match app.add_contact(invite, code).await {
            Ok(contact) => Ok(ns_chat(id, &contact)),
            Err(e) => {
                let _gate = self.inner.gate.lock().await;
                self.delete_persona_locked(id).await;
                Err(e)
            }
        }
    }

    // ───────────── rooms and their messages: routed by id ─────────────

    pub fn rooms(&self) -> Result<Vec<RoomView>> {
        let mut out = self.inner.main.rooms()?;
        for (id, alias, app) in self.all_personas() {
            // A persona that is still opening has nothing to show yet.
            for mut r in app.rooms().unwrap_or_default() {
                r.id = ns_chat(id, &r.id);
                r.identity = Some(alias.clone());
                r.last = r.last.map(|m| map_message(id, m));
                out.push(r);
            }
        }
        out.sort_by_key(|r| std::cmp::Reverse(r.last.as_ref().map(|m| (m.ts, m.id))));
        Ok(out)
    }

    /// Create a room. With `identity`, as a brand-new identity of that name (new keys, new
    /// Tor address); invite people with a room invite and a code.
    pub async fn create_room(&self, name: &str, open_invites: bool, temp: bool, identity: Option<&str>) -> Result<String> {
        let make = |app: &App<B>| if temp { app.create_temp_room(name, open_invites) } else { app.create_room(name, open_invites) };
        let Some(alias) = identity else { return make(&self.inner.main) };
        let (id, app) = self.new_persona(alias, CREATE_GRACE_SECS).await?;
        match make(&app) {
            Ok(room) => Ok(ns_chat(id, &room)),
            Err(e) => {
                let _gate = self.inner.gate.lock().await;
                self.delete_persona_locked(id).await;
                Err(e)
            }
        }
    }

    pub fn room_invitations(&self) -> Result<Vec<RoomInvitation>> {
        let mut out = self.inner.main.room_invitations()?;
        for (id, _, app) in self.all_personas() {
            for mut i in app.room_invitations().unwrap_or_default() {
                i.room = ns_chat(id, &i.room);
                out.push(i);
            }
        }
        out.sort_by_key(|i| (i.ts, i.room.clone()));
        Ok(out)
    }

    pub fn accept_room_invite(&self, room: &str) -> Result<String> {
        let (id, local, app) = self.route(room)?;
        app.accept_room_invite(&local).map(|r| ns_chat(id, &r))
    }

    pub fn decline_room_invite(&self, room: &str) -> Result<()> {
        let (_, local, app) = self.route(room)?;
        app.decline_room_invite(&local)
    }

    pub fn new_room_invite(&self, room: &str, label: &str, ttl: Duration) -> Result<(String, String)> {
        let (_, local, app) = self.route(room)?;
        app.new_room_invite(&local, label, ttl)
    }

    /// Add someone you already talk to. Not for a room with its own identity: your main
    /// contacts must not be tied to it.
    pub fn add_contact_to_room(&self, room: &str, contact: &str) -> Result<()> {
        let (id, local, app) = self.route(room)?;
        if id != 0 {
            return Err(invalid("a room with its own identity can only take people through a room invite"));
        }
        app.add_contact_to_room(&local, contact)
    }

    pub fn leave_room(&self, room: &str) -> Result<()> {
        let (_, local, app) = self.route(room)?;
        app.leave_room(&local)
    }

    pub fn remove_room_member(&self, room: &str, member: &str) -> Result<()> {
        let (_, local, app) = self.route(room)?;
        app.remove_room_member(&local, member)
    }

    pub fn rename_room(&self, room: &str, name: &str) -> Result<()> {
        let (_, local, app) = self.route(room)?;
        app.rename_room(&local, name)
    }

    pub async fn send_room_text(&self, room: &str, text: &str) -> Result<i64> {
        let (id, local, app) = self.route(room)?;
        app.send_room_text(&local, text).await.map(|m| ns_msg(id, m))
    }

    pub async fn send_room_file(&self, room: &str, name: &str, data: Vec<u8>) -> Result<i64> {
        let (id, local, app) = self.route(room)?;
        app.send_room_file(&local, name, data).await.map(|m| ns_msg(id, m))
    }

    pub fn messages(&self, chat: &str, before: Option<i64>, limit: u32) -> Result<Vec<StoredMessage>> {
        let (id, local, app) = self.route(chat)?;
        let before = before.map(|b| split_msg(b).1);
        Ok(app.messages(&local, before, limit)?.into_iter().map(|m| map_message(id, m)).collect())
    }

    pub fn mark_read(&self, chat: &str) -> Result<()> {
        let (_, local, app) = self.route(chat)?;
        app.mark_read(&local)
    }

    pub fn delete_chat(&self, chat: &str) -> Result<()> {
        let (_, local, app) = self.route(chat)?;
        app.delete_chat(&local)
    }

    pub fn delete_message(&self, msg_id: i64) -> Result<()> {
        let (_, local, app) = self.route_msg(msg_id)?;
        app.delete_message(local)
    }

    pub fn save_file(&self, msg_id: i64, dir: &Path) -> Result<PathBuf> {
        let (_, local, app) = self.route_msg(msg_id)?;
        app.save_file(local, dir)
    }

    pub fn file_bytes(&self, msg_id: i64) -> Result<(String, Vec<u8>)> {
        let (_, local, app) = self.route_msg(msg_id)?;
        app.file_bytes(local)
    }

    /// Numbers of the separate identities that exist (for tests and diagnostics).
    pub fn persona_ids(&self) -> Vec<u32> {
        self.inner.personas.lock().expect("poisoned").keys().copied().collect()
    }

    /// The onion address a separate identity uses (its own, unrelated to your main one).
    pub fn persona_onion(&self, id: u32) -> Option<String> {
        self.app_of(id).and_then(|a| a.my_onion())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_stay_javascript_safe() {
        assert_eq!(ns_chat(0, "r-ab"), "r-ab");
        assert_eq!(ns_chat(3, "r-ab"), "p3~r-ab");
        assert_eq!(split_chat("p3~r-ab"), (3, "r-ab"));
        assert_eq!(split_chat("r-ab"), (0, "r-ab"));
        assert_eq!(split_chat("p0~x"), (0, "p0~x"), "persona 0 is not a persona");
        assert_eq!(split_chat("pabc~x"), (0, "pabc~x"));
        assert_eq!(split_chat("p99999~x").0, 0);
        for persona in [0u32, 1, 7, MAX_PERSONA_ID] {
            for local in [1i64, 12345, (1 << 40) + 5, (1 << 42) - 1] {
                let id = ns_msg(persona, local);
                assert!(id < (1 << 53), "{id} would lose precision in JavaScript");
                assert_eq!(split_msg(id), (persona, local));
            }
        }
    }
}
