use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use guft_core::engine::now_secs;
use guft_core::payload::check_file_name;
use guft_core::rooms::{parse_room_key, room_key, RoomEffects, RoomEvent, RoomInfo};
use guft_core::RoomKind;
use guft_core::Payload;
use guft_core::spool::SpoolPublic;
use guft_net::access::{Access, AccessControl};
use guft_net::{Inbound, Transport};
use guft_store::{Locker, Profile, Status, StoredMessage};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, Notify};
use tokio::task::JoinHandle;
use tokio::time::sleep;
use zeroize::Zeroizing;

use crate::backend::{NetworkBackend, Running};
use crate::temp::{is_temp_id, TempOut, TempOutbox, TempStore, MAX_QUEUED_PER_CONTACT};
use guft_core::Engine;
use crate::{AppError, Event, Result};

const SEEN_KEEP_SECS: u64 = 30 * 24 * 3600;
/// A room invite we imported is expected to be followed by its room invitation for this long
/// (a bit longer than the longest invite lifetime).
const ROOM_EXPECT_SECS: u64 = 8 * 24 * 3600;
/// Invitations nobody answered are dropped after this.
const INVITATION_KEEP_SECS: u64 = 14 * 24 * 3600;
const MAX_PENDING_INVITATIONS: usize = 30;
const MAX_PENDING_PER_CONTACT: usize = 5;

#[derive(Clone, Debug)]
pub struct AppOptions {
    /// Relock after this much inactivity.
    pub idle_timeout: Duration,
    /// How often we check the idle timer and the outbox.
    pub tick: Duration,
    /// Retry delay after a failed send doubles from here up to `retry_max`.
    pub retry_base: Duration,
    pub retry_max: Duration,
    /// Give up on a queued message after this many failed attempts.
    pub max_attempts: u32,
}

impl Default for AppOptions {
    fn default() -> Self {
        Self {
            idle_timeout: Duration::from_secs(5 * 60),
            tick: Duration::from_secs(2),
            retry_base: Duration::from_secs(5),
            retry_max: Duration::from_secs(600),
            max_attempts: 300,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ContactView {
    pub id: String,
    pub name: String,
    pub onion: String,
    pub verified: bool,
}

#[derive(Clone, Debug)]
pub struct ChatView {
    pub contact: ContactView,
    pub last: Option<StoredMessage>,
    pub unread: u32,
}

#[derive(Clone, Debug)]
pub struct MemberView {
    pub id: String,
    pub name: String,
    /// We have a session with them, so we can message them.
    pub connected: bool,
}

/// A contact asked you to join a room. Nothing happens, and nobody in the room learns about you,
/// until you accept.
#[derive(Clone, Debug)]
pub struct RoomInvitation {
    /// The room's chat id (`r-<hex>`).
    pub room: String,
    pub from: String,
    pub from_name: String,
    pub name: String,
    /// Names of the people already in it (not counting you).
    pub members: Vec<String>,
    pub temp: bool,
    pub ts: u64,
}

/// A room as the UI sees it. `id` is the chat key used with `messages`, `mark_read`, etc.
#[derive(Clone, Debug)]
pub struct RoomView {
    pub id: String,
    pub name: String,
    /// Contact id of whoever created the room.
    pub creator: String,
    pub mine: bool,
    pub open_invites: bool,
    /// Lives in memory only: messages vanish when guft locks or closes.
    pub temp: bool,
    /// A one-to-one temporary chat (`name` is the other person's name).
    pub direct: bool,
    /// The separate identity you appear as in this room (its name), or `None` for your main one.
    pub identity: Option<String>,
    pub members: Vec<MemberView>,
    pub last: Option<StoredMessage>,
    pub unread: u32,
}

/// History key for a room's messages (room ids are hex, contact ids are hex too, so the prefix keeps them apart).
fn chat_key(room: &str) -> String {
    format!("r-{room}")
}

/// Is `chat` a memory-only room? Its messages must never go to the database.
fn is_temp_chat(p: &Profile, chat: &str) -> bool {
    chat.strip_prefix("r-").is_some_and(|k| p.engine.is_temp_room(k))
}

fn room_of(chat: &str) -> Result<&str> {
    chat.strip_prefix("r-").ok_or(AppError::Core(guft_core::Error::Invalid("not a room")))
}

struct RunState<T> {
    transport: Arc<T>,
    access: Arc<dyn AccessControl>,
    tasks: Vec<JoinHandle<()>>,
    stop: Box<dyn FnOnce() -> futures::future::BoxFuture<'static, ()> + Send>,
}

struct Inner<B: NetworkBackend> {
    backend: B,
    opts: AppOptions,
    dir: PathBuf,
    locker: Mutex<Locker>,
    run: Mutex<Option<RunState<B::T>>>,
    events: broadcast::Sender<Event>,
    wake: Notify,
    retry_at: Mutex<HashMap<String, Instant>>,
    in_flight: Mutex<HashSet<String>>,
    /// Bumped on every unlock and lock; background loops from an older epoch exit.
    epoch: AtomicU64,
    /// Last authorization state pushed to the network layer.
    last_access: Mutex<Option<Access>>,
    /// Set while "stay online when locked" is on: the public spool key for sealing frames.
    keep: Mutex<Option<SpoolPublic>>,
    /// Messages of memory-only rooms. Never touches the database.
    temp: Mutex<TempStore>,
    /// Unsent frames of memory-only rooms.
    temp_out: TempOutbox,
}

pub struct App<B: NetworkBackend> {
    inner: Arc<Inner<B>>,
}

impl<B: NetworkBackend> Clone for App<B> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

/// Encrypt `payload` for `to` and queue it. Must run inside `with`/`with_bg`; the caller saves.
fn queue_payload(p: &mut Profile, to: &str, payload: &Payload, msg_id: i64) -> guft_store::Result<()> {
    let frame = p.engine.encrypt(to, payload).map_err(guft_store::Error::from)?;
    p.history.outbox_push(to, msg_id, &frame)?;
    Ok(())
}

/// Like [`queue_payload`] for room control messages. A memory-only room's frames wait in memory.
fn queue_ctl(p: &mut Profile, out: &TempOutbox, temp: bool, to: &str, payload: &Payload) -> guft_store::Result<()> {
    if !temp {
        return queue_payload(p, to, payload, 0);
    }
    let frame = p.engine.encrypt(to, payload).map_err(guft_store::Error::from)?;
    out.push(to, TempOut { msg_id: 0, frame, attempts: 0 });
    Ok(())
}

/// The room a room payload is about.
fn room_id_of(p: &Payload) -> Option<[u8; 16]> {
    match p {
        Payload::RoomInvite { room, .. }
        | Payload::RoomText { room, .. }
        | Payload::RoomFile { room, .. }
        | Payload::Introduce { room, .. }
        | Payload::Introduction { room, .. }
        | Payload::RoomLeave { room }
        | Payload::RoomRemove { room, .. }
        | Payload::RoomRename { room, .. } => Some(*room),
        _ => None,
    }
}

/// Where the memory-only parts of the app live while a message is handled.
struct TempSink<'a> {
    store: &'a mut TempStore,
    out: &'a TempOutbox,
    /// The payload being handled belongs to a memory-only room.
    temp: bool,
}

/// Apply what the room logic asks for: messages to send, a message to show, events to raise.
fn apply_room_effects(p: &mut Profile, fx: RoomEffects, now: u64, events: &mut Vec<Event>, sink: &mut TempSink<'_>) -> guft_store::Result<()> {
    for (to, payload) in &fx.send {
        queue_ctl(p, sink.out, sink.temp, to, payload)?;
    }
    if let Some(d) = fx.deliver {
        let mapped = match d.payload {
            Payload::RoomText { text, .. } => Payload::Text(text),
            Payload::RoomFile { name, data, .. } => Payload::File { name, data },
            _ => return Ok(()),
        };
        let chat = chat_key(&d.room);
        let id = if d.temp {
            // Memory only: never written to the history database.
            sink.store.add(&chat, now, false, Some(&d.from), &mapped, 0)
        } else {
            Some(p.history.add_from(&chat, now, false, Some(&d.from), &mapped)?)
        };
        if let Some(id) = id {
            events.push(Event::Message { chat, id });
        }
    }
    for ev in fx.events {
        events.push(match ev {
            RoomEvent::Joined { room, .. } | RoomEvent::Changed { room } => Event::RoomChanged { room: chat_key(&room) },
            RoomEvent::Removed { room } => {
                sink.store.drop_room(&chat_key(&room));
                Event::RoomRemoved { room: chat_key(&room) }
            }
            RoomEvent::ContactAdded { id, name } => Event::ContactAdded { id, name },
        });
    }
    Ok(())
}

/// Create a new file that only the current user can read (mode 0600 on Unix; on Windows the
/// per-user folder ACL applies). Fails if it already exists.
fn private_new_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut opts = OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    opts.mode(0o600);
    opts.open(path)
}

/// One frame to send: from the database outbox (`db_id`) or from memory.
struct Job {
    db_id: Option<i64>,
    msg_id: i64,
    frame: Vec<u8>,
    attempts: u32,
}

fn frame_hash(frame: &[u8]) -> [u8; 16] {
    let mut h = [0u8; 16];
    h.copy_from_slice(&Sha256::digest(frame)[..16]);
    h
}

impl<B: NetworkBackend> App<B> {
    pub fn new(dir: PathBuf, backend: B, opts: AppOptions) -> Self {
        let (events, _) = broadcast::channel(256);
        let locker = Locker::new(dir.clone(), opts.idle_timeout);
        Self {
            inner: Arc::new(Inner {
                backend,
                opts,
                dir,
                locker: Mutex::new(locker),
                run: Mutex::new(None),
                events,
                wake: Notify::new(),
                retry_at: Mutex::new(HashMap::new()),
                in_flight: Mutex::new(HashSet::new()),
                epoch: AtomicU64::new(0),
                last_access: Mutex::new(None),
                keep: Mutex::new(None),
                temp: Mutex::new(TempStore::default()),
                temp_out: TempOutbox::default(),
            }),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    fn emit(&self, e: Event) {
        let _ = self.inner.events.send(e);
    }

    /// Run `f` on the unlocked profile. Counts as activity; fails with `Locked` otherwise.
    fn with<R>(&self, f: impl FnOnce(&mut Profile) -> guft_store::Result<R>) -> Result<R> {
        Ok(self.inner.locker.lock().expect("poisoned").with(f)?)
    }

    /// For background work: same as `with`, but never extends the idle timer.
    fn with_bg<R>(&self, f: impl FnOnce(&mut Profile) -> guft_store::Result<R>) -> Result<R> {
        Ok(self.inner.locker.lock().expect("poisoned").peek(f)?)
    }

    // ───────────── lifecycle ─────────────

    pub fn has_profile(&self) -> bool {
        Profile::exists(&self.inner.dir)
    }

    pub fn is_unlocked(&self) -> bool {
        self.inner.locker.lock().expect("poisoned").is_unlocked()
    }

    /// Create the profile (runs Argon2, so off the async threads), then unlock it.
    pub async fn create_profile(&self, passphrase: &str, display_name: &str) -> Result<()> {
        let (dir, pass, name) = (self.inner.dir.clone(), Zeroizing::new(passphrase.to_owned()), display_name.to_owned());
        tokio::task::spawn_blocking(move || Profile::create(&dir, &pass, &name).and_then(Profile::lock)).await??;
        self.unlock(passphrase).await
    }

    /// Create a profile protected by a random 256-bit secret (a separate identity). Cheap to open.
    pub async fn create_profile_for_random_secret(&self, secret: &str, display_name: &str) -> Result<()> {
        let (dir, secret_owned, name) = (self.inner.dir.clone(), Zeroizing::new(secret.to_owned()), display_name.to_owned());
        tokio::task::spawn_blocking(move || Profile::create_for_random_secret(&dir, &secret_owned, &name).and_then(Profile::lock)).await??;
        self.unlock_in_background(secret).await
    }

    /// Like [`unlock`](Self::unlock), but the network starts in the background instead of
    /// being waited for: the data is usable at once.
    pub async fn unlock_in_background(&self, passphrase: &str) -> Result<()> {
        self.unlock_inner(passphrase, false).await
    }

    pub async fn unlock(&self, passphrase: &str) -> Result<()> {
        self.unlock_inner(passphrase, true).await
    }

    /// Small extra data sealed inside the profile (the list of separate identities).
    pub fn aux_read(&self, name: &str) -> Result<Option<Vec<u8>>> {
        self.with_bg(|p| p.read_aux(name))
    }

    pub fn aux_write(&self, name: &str, data: &[u8]) -> Result<()> {
        self.with_bg(|p| p.write_aux(name, data))
    }

    async fn unlock_inner(&self, passphrase: &str, wait_for_network: bool) -> Result<()> {
        if self.is_unlocked() {
            return Ok(());
        }
        self.wipe_temp();
        let (inner, pass) = (self.inner.clone(), Zeroizing::new(passphrase.to_owned()));
        tokio::task::spawn_blocking(move || inner.locker.lock().expect("poisoned").unlock(&pass)).await??;
        let epoch = self.inner.epoch.fetch_add(1, Ordering::SeqCst) + 1;

        let seed = self.with(|p| Ok(p.engine.onion_seed()))?;
        let onion = self.inner.backend.onion_for_seed(&seed)?;
        self.with(|p| {
            p.engine.set_onion(&onion).map_err(guft_store::Error::from)?;
            p.history.seen_prune(now_secs().saturating_sub(SEEN_KEEP_SECS))?;
            p.history.prune_room_invites(now_secs().saturating_sub(INVITATION_KEEP_SECS))?;
            p.save()
        })?;
        let keep = self.with_bg(|p| Ok(p.engine.online_when_locked().then(|| p.engine.spool_public())))?;
        *self.inner.keep.lock().expect("poisoned") = keep;
        self.emit(Event::Unlocked);
        self.spawn_maintenance(epoch);
        if wait_for_network {
            self.start_network().await;
        } else {
            let app = self.clone();
            tokio::spawn(async move { app.start_network().await });
        }
        self.sync_access();
        // Frames that arrived while locked (sealed to the spool key) are read now.
        let app = self.clone();
        tokio::task::spawn_blocking(move || app.drain_spool()).await?;
        Ok(())
    }

    /// (Re)start the network; the app stays usable offline if this fails.
    pub async fn start_network(&self) {
        if self.inner.run.lock().expect("poisoned").is_some() || !self.is_unlocked() {
            return;
        }
        let Ok(seed) = self.with_bg(|p| Ok(p.engine.onion_seed())) else { return };
        let Ok(access) = self.current_access() else { return };
        match self.inner.backend.start(&seed, access.clone()).await {
            Ok(Running { transport, inbound, onion, access: control, stop }) => {
                let task = self.spawn_inbound(inbound);
                *self.inner.last_access.lock().expect("poisoned") = Some(access);
                *self.inner.run.lock().expect("poisoned") = Some(RunState { transport, access: control, tasks: vec![task], stop });
                self.emit(Event::NetworkReady { onion });
                self.inner.wake.notify_one();
            }
            Err(e) => self.emit(Event::NetworkError(e.to_string())),
        }
    }

    async fn stop_network(&self) {
        let state = self.inner.run.lock().expect("poisoned").take();
        if let Some(RunState { tasks, stop, .. }) = state {
            for t in tasks {
                t.abort();
            }
            stop().await;
        }
    }

    /// Save, close the database, wipe keys and decrypted state, and go offline.
    pub async fn lock(&self) -> Result<()> {
        self.inner.epoch.fetch_add(1, Ordering::SeqCst);
        self.go_quiet().await;
        let inner = self.inner.clone();
        let was = self.is_unlocked();
        tokio::task::spawn_blocking(move || inner.locker.lock().expect("poisoned").lock()).await??;
        if was {
            self.emit(Event::Locked);
        }
        Ok(())
    }

    /// Call on any user activity so the idle timer restarts.
    pub fn touch(&self) {
        self.inner.locker.lock().expect("poisoned").touch();
    }

    pub fn set_idle_timeout(&self, t: Duration) {
        self.inner.locker.lock().expect("poisoned").set_idle_timeout(t);
    }

    pub fn my_onion(&self) -> Option<String> {
        self.with_bg(|p| Ok(p.engine.onion().map(str::to_owned))).ok().flatten()
    }

    pub fn my_name(&self) -> Result<String> {
        self.with(|p| Ok(p.engine.display_name().to_owned()))
    }

    fn transport(&self) -> Option<Arc<B::T>> {
        self.inner.run.lock().expect("poisoned").as_ref().map(|r| r.transport.clone())
    }

    // ───────────── background tasks ─────────────

    /// Idle-lock timer plus outbox scheduler. Exits when the epoch moves on.
    fn spawn_maintenance(&self, epoch: u64) {
        let app = self.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = sleep(app.inner.opts.tick) => {}
                    _ = app.inner.wake.notified() => {}
                }
                if app.inner.epoch.load(Ordering::SeqCst) != epoch {
                    return;
                }
                let _ = app.inner.locker.lock().expect("poisoned").tick();
                if !app.is_unlocked() {
                    // Locked by the idle timer (explicit locks bump the epoch first).
                    app.inner.epoch.fetch_add(1, Ordering::SeqCst);
                    app.go_quiet().await;
                    app.emit(Event::Locked);
                    return;
                }
                app.sync_access();
                app.schedule_flushes();
            }
        });
    }

    /// Forget every memory-only message and unsent frame.
    fn wipe_temp(&self) {
        self.inner.temp.lock().expect("poisoned").clear();
        self.inner.temp_out.clear();
    }

    /// On lock: go fully offline, or, if the user opted in, stay reachable and only
    /// spool sealed frames (no outgoing links, no keys that read messages).
    async fn go_quiet(&self) {
        self.wipe_temp();
        let stay = self.inner.keep.lock().expect("poisoned").is_some();
        if stay {
            if let Some(t) = self.transport() {
                t.close_links().await;
            }
        } else {
            self.stop_network().await;
        }
    }

    pub fn online_when_locked(&self) -> Result<bool> {
        self.with_bg(|p| Ok(p.engine.online_when_locked()))
    }

    /// Opt in or out of staying reachable while locked. While locked, the onion
    /// identity key stays in memory (the vault key and message keys do not).
    pub fn set_online_when_locked(&self, on: bool) -> Result<()> {
        let keep = self.with(|p| {
            p.engine.set_online_when_locked(on);
            p.save()?;
            Ok(on.then(|| p.engine.spool_public()))
        })?;
        *self.inner.keep.lock().expect("poisoned") = keep;
        Ok(())
    }

    fn spool_dir(&self) -> PathBuf {
        self.inner.dir.join("spool")
    }

    /// Seal a frame to the spool key and store it. Returns whether to acknowledge.
    fn spool_frame(&self, frame: &[u8]) -> bool {
        const MAX_ENTRIES: usize = 2000;
        const MAX_BYTES: u64 = 64 * 1024 * 1024;
        let Some(public) = self.inner.keep.lock().expect("poisoned").clone() else { return false };
        let Ok(sealed) = public.seal(frame) else { return false };
        let dir = self.spool_dir();
        if std::fs::create_dir_all(&dir).is_err() {
            return false;
        }
        #[cfg(unix)]
        let _ = std::fs::set_permissions(&dir, std::os::unix::fs::PermissionsExt::from_mode(0o700));
        let (mut count, mut bytes) = (0usize, 0u64);
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for e in rd.flatten() {
                count += 1;
                bytes += e.metadata().map(|m| m.len()).unwrap_or(0);
            }
        }
        if count >= MAX_ENTRIES || bytes + sealed.len() as u64 > MAX_BYTES {
            return false;
        }
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let name = format!("{nanos:024}-{:016x}.frm", rand::random::<u64>());
        let written = private_new_file(&dir.join(name)).and_then(|mut f| {
            f.write_all(&sealed)?;
            f.sync_all()
        });
        written.is_ok()
    }

    /// Process (and delete) everything spooled while we were locked, oldest first.
    fn drain_spool(&self) {
        let dir = self.spool_dir();
        let Ok(rd) = std::fs::read_dir(&dir) else { return };
        let mut names: Vec<_> = rd.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "frm")).collect();
        names.sort();
        for path in names {
            if let Ok(blob) = std::fs::read(&path) {
                if let Ok(Ok(frame)) = self.with_bg(|p| Ok(p.engine.open_spooled(&blob))) {
                    self.process_frame_blocking(&frame);
                }
            }
            // Processed, duplicate or unreadable: in every case it must not linger.
            let _ = std::fs::remove_file(&path);
        }
    }

    // ───────────── network access keys ─────────────

    fn current_access(&self) -> Result<Access> {
        let now = now_secs();
        self.with_bg(|p| Ok(Access { authorized: p.engine.authorized_clients(now), connect: p.engine.connect_keys() }))
    }

    /// Push authorization changes (new contact, expired invite, replaced key) to the network.
    fn sync_access(&self) {
        let Ok(access) = self.current_access() else { return };
        if self.inner.last_access.lock().expect("poisoned").as_ref() == Some(&access) {
            return;
        }
        let control = self.inner.run.lock().expect("poisoned").as_ref().map(|r| r.access.clone());
        if let Some(control) = control {
            if control.update(access.clone()).is_ok() {
                *self.inner.last_access.lock().expect("poisoned") = Some(access);
            }
        }
    }

    fn spawn_inbound(&self, mut inbound: tokio::sync::mpsc::Receiver<Inbound>) -> JoinHandle<()> {
        let app = self.clone();
        tokio::spawn(async move {
            while let Some(Inbound { frame, ack }) = inbound.recv().await {
                let app = app.clone();
                tokio::spawn(async move {
                    let accepted = tokio::task::spawn_blocking(move || app.process_frame_blocking(&frame)).await.unwrap_or(false);
                    let _ = ack.send(accepted);
                });
            }
        })
    }

    /// Decrypt and store one inbound frame. Returns whether to acknowledge it.
    pub async fn process_frame(&self, frame: &[u8]) -> bool {
        let (app, frame) = (self.clone(), frame.to_vec());
        tokio::task::spawn_blocking(move || app.process_frame_blocking(&frame)).await.unwrap_or(false)
    }

    fn process_frame_blocking(&self, frame: &[u8]) -> bool {
        if !self.is_unlocked() {
            return self.spool_frame(frame);
        }
        let hash = frame_hash(frame);
        let now = now_secs();
        let outcome = self.with_bg(|p| {
            // Already accepted earlier (our acknowledgement was lost): just re-acknowledge.
            if p.history.seen_contains(&hash)? {
                return Ok(Some(Vec::new()));
            }
            let Ok(rec) = p.engine.decrypt(frame, now) else { return Ok(None) };
            let mut events = Vec::new();
            if rec.new_contact {
                let name = p.engine.contacts().into_iter().find(|c| c.id == rec.from).map(|c| c.name).unwrap_or_default();
                events.push(Event::ContactAdded { id: rec.from.clone(), name });
            }
            if rec.new_contact {
                // Give them the permanent key that replaces the invite's one-use key.
                let key_msg = p.engine.auth_key_for(&rec.from).map_err(guft_store::Error::from)?;
                queue_payload(p, &rec.from, &key_msg, 0)?;
                // A room-bound invite: they are in the room on our side; now tell them who is in it.
                if let Some(room) = rec.joined_room {
                    let key = room_key(&room);
                    let invite = p.engine.room_invite_for(&key, &rec.from).map_err(guft_store::Error::from)?;
                    queue_ctl(p, &self.inner.temp_out, p.engine.is_temp_room(&key), &rec.from, &invite)?;
                    events.push(Event::RoomChanged { room: chat_key(&key) });
                }
            }
            match &rec.payload {
                // Being added to a room by a contact needs your say-so: joining tells every member
                // who you are. Only an invitation you asked for (you used a room invite), or a
                // one-to-one temporary chat (nobody else is involved), goes through at once.
                Payload::RoomInvite { room, kind, .. } => {
                    let asked_for = p.history.take_expected_room(&rec.from, room, now.saturating_sub(ROOM_EXPECT_SECS))?;
                    if asked_for || *kind == RoomKind::Direct {
                        self.apply_room_payload(p, &rec.from, &rec.payload, now, &mut events)?;
                    } else {
                        self.hold_invitation(p, &rec.from, &rec.payload, now, &mut events)?;
                    }
                }
                Payload::RoomText { .. }
                | Payload::RoomFile { .. }
                | Payload::Introduce { .. }
                | Payload::Introduction { .. }
                | Payload::RoomLeave { .. }
                | Payload::RoomRemove { .. }
                | Payload::RoomRename { .. } => self.apply_room_payload(p, &rec.from, &rec.payload, now, &mut events)?,
                _ => {}
            }
            if let Payload::Text(_) | Payload::File { .. } = rec.payload {
                let id = p.history.add(&rec.from, now, false, &rec.payload)?;
                events.push(Event::Message { chat: rec.from.clone(), id });
            }
            // History first, state second: a crash in between only costs a re-delivery
            // that the `seen` table absorbs, never a lost message.
            p.history.seen_insert(&hash, now)?;
            p.save()?;
            Ok(Some(events))
        });
        match outcome {
            Ok(Some(events)) => {
                for e in events {
                    self.emit(e);
                }
                self.sync_access();
                self.inner.wake.notify_one();
                true
            }
            // Locked between the check above and now: keep it sealed instead of dropping it.
            Err(AppError::Locked) => self.spool_frame(frame),
            _ => false,
        }
    }

    /// Run a room payload through the room logic and carry out what it asks for.
    fn apply_room_payload(&self, p: &mut Profile, from: &str, payload: &Payload, now: u64, events: &mut Vec<Event>) -> guft_store::Result<()> {
        // A malformed or hostile room message must not stop us acknowledging the frame.
        let in_temp = |p: &Profile| room_id_of(payload).is_some_and(|r| p.engine.is_temp_room(&room_key(&r)));
        let was_temp = in_temp(p);
        if let Ok(fx) = p.engine.on_room_payload(from, payload, now) {
            let mut store = self.inner.temp.lock().expect("poisoned");
            let temp = was_temp || in_temp(p);
            apply_room_effects(p, fx, now, events, &mut TempSink { store: &mut store, out: &self.inner.temp_out, temp })?;
        }
        Ok(())
    }

    /// Keep an invitation for the user to answer. Group rooms go to the encrypted database;
    /// temporary ones stay in memory.
    fn hold_invitation(&self, p: &mut Profile, from: &str, payload: &Payload, now: u64, events: &mut Vec<Event>) -> guft_store::Result<()> {
        let Payload::RoomInvite { room, kind, .. } = payload else { return Ok(()) };
        let key = room_key(room);
        if p.engine.rooms().iter().any(|r| r.id == key) || !p.engine.contacts().iter().any(|c| c.id == from) {
            return Ok(());
        }
        let waiting = self.collect_invitations(p)?;
        if waiting.len() >= MAX_PENDING_INVITATIONS || waiting.iter().filter(|i| i.from == from).count() >= MAX_PENDING_PER_CONTACT {
            return Ok(());
        }
        let added = if *kind == RoomKind::Normal {
            p.history.add_pending_room(&key, from, now, &payload.encode()?)?
        } else {
            self.inner.temp.lock().expect("poisoned").add_invite(&key, from, now, payload.clone())
        };
        if added {
            events.push(Event::RoomInvited { room: chat_key(&key) });
        }
        Ok(())
    }

    fn collect_invitations(&self, p: &Profile) -> guft_store::Result<Vec<RoomInvitation>> {
        let names: HashMap<String, String> = p.engine.contacts().into_iter().map(|c| (c.id, c.name)).collect();
        let view = |key: &str, from: &str, ts: u64, payload: &Payload| match payload {
            Payload::RoomInvite { name, kind, members, .. } => Some(RoomInvitation {
                room: chat_key(key),
                from: from.to_owned(),
                from_name: names.get(from).cloned().unwrap_or_default(),
                name: name.clone(),
                members: members.iter().map(|m| m.name.clone()).collect(),
                temp: *kind != RoomKind::Normal,
                ts,
            }),
            _ => None,
        };
        let mut out = Vec::new();
        for (key, from, ts, blob) in p.history.pending_rooms()? {
            if let Some(v) = Payload::decode(blob).ok().and_then(|pl| view(&key, &from, ts, &pl)) {
                out.push(v);
            }
        }
        for (key, inv) in self.inner.temp.lock().expect("poisoned").invites() {
            if let Some(v) = view(&key, &inv.from, inv.ts, &inv.payload) {
                out.push(v);
            }
        }
        out.sort_by_key(|i| (i.ts, i.room.clone()));
        Ok(out)
    }

    /// Take a waiting invitation out of wherever it is kept.
    fn take_invitation(&self, p: &mut Profile, key: &str) -> guft_store::Result<Option<(String, Payload)>> {
        if let Some((from, _ts, blob)) = p.history.pending_room(key)? {
            p.history.delete_pending_room(key)?;
            return Ok(Payload::decode(blob).ok().map(|pl| (from, pl)));
        }
        Ok(self.inner.temp.lock().expect("poisoned").take_invite(key).map(|i| (i.from, i.payload)))
    }

    /// Rooms contacts have invited you to, oldest first.
    pub fn room_invitations(&self) -> Result<Vec<RoomInvitation>> {
        self.with(|p| self.collect_invitations(p))
    }

    /// Join. Only now do the members learn about you. Returns the room's chat id.
    pub fn accept_room_invite(&self, room: &str) -> Result<String> {
        let key = room_of(room)?.to_owned();
        let mut events = Vec::new();
        self.with(|p| {
            let Some((from, payload)) = self.take_invitation(p, &key)? else {
                return Err(guft_core::Error::Invalid("that invitation is gone").into());
            };
            if !p.engine.contacts().iter().any(|c| c.id == from) {
                return Err(guft_core::Error::UnknownContact.into());
            }
            self.apply_room_payload(p, &from, &payload, now_secs(), &mut events)?;
            if !p.engine.rooms().iter().any(|r| r.id == key) {
                return Err(guft_core::Error::Invalid("that invitation is no longer valid").into());
            }
            p.save()
        })?;
        for e in events {
            self.emit(e);
        }
        self.inner.wake.notify_one();
        Ok(chat_key(&key))
    }

    /// Say no. The inviter is told quietly (as if you left) so they stop counting you in.
    pub fn decline_room_invite(&self, room: &str) -> Result<()> {
        let key = room_of(room)?.to_owned();
        self.with(|p| {
            if let Some((from, Payload::RoomInvite { room: id, kind, .. })) = self.take_invitation(p, &key)? {
                if p.engine.contacts().iter().any(|c| c.id == from) {
                    queue_ctl(p, &self.inner.temp_out, kind != RoomKind::Normal, &from, &Payload::RoomLeave { room: id })?;
                    p.save()?;
                }
            }
            Ok(())
        })?;
        self.emit(Event::RoomChanged { room: chat_key(&key) });
        self.inner.wake.notify_one();
        Ok(())
    }

    fn schedule_flushes(&self) {
        let Ok(mut chats) = self.with_bg(|p| p.history.outbox_chats()) else { return };
        for c in self.inner.temp_out.contacts() {
            if !chats.contains(&c) {
                chats.push(c);
            }
        }
        let now = Instant::now();
        for chat in chats {
            let due = self.inner.retry_at.lock().expect("poisoned").get(&chat).is_none_or(|t| *t <= now);
            if due && self.inner.in_flight.lock().expect("poisoned").insert(chat.clone()) {
                let app = self.clone();
                tokio::spawn(async move {
                    app.flush_contact(&chat).await;
                    app.inner.in_flight.lock().expect("poisoned").remove(&chat);
                });
            }
        }
    }

    /// Send queued frames for one contact, oldest first, until one fails. Frames
    /// of memory-only rooms wait in memory and go out after the persistent ones.
    async fn flush_contact(&self, chat: &str) {
        loop {
            let next = self.with_bg(|p| {
                let Some(onion) = p.engine.contacts().into_iter().find(|c| c.id == chat).map(|c| c.onion) else { return Ok(None) };
                if let Some(i) = p.history.outbox_for(chat)?.into_iter().next() {
                    return Ok(Some((Job { db_id: Some(i.id), msg_id: i.msg_id, frame: i.frame, attempts: i.attempts }, onion)));
                }
                Ok(self.inner.temp_out.front(chat).map(|t| (Job { db_id: None, msg_id: t.msg_id, frame: t.frame, attempts: t.attempts }, onion)))
            });
            let Ok(Some((job, onion))) = next else {
                // Nothing queued, contact removed, or locked.
                self.inner.retry_at.lock().expect("poisoned").remove(chat);
                return;
            };
            let Some(transport) = self.transport() else { return };
            match transport.send(&onion, job.frame.clone()).await {
                Ok(()) => {
                    let done = self.with_bg(|p| match job.db_id {
                        Some(id) => {
                            p.history.outbox_done(id)?;
                            if job.msg_id != 0 {
                                // A room message is "delivered" only once every member has it.
                                let all = p.history.outbox_remaining(job.msg_id)? == 0;
                                p.history.set_status(job.msg_id, if all { Status::Delivered } else { Status::Sent })?;
                            }
                            Ok(())
                        }
                        None => {
                            self.inner.temp_out.pop(chat);
                            if job.msg_id != 0 {
                                self.inner.temp.lock().expect("poisoned").acknowledged(job.msg_id);
                            }
                            Ok(())
                        }
                    });
                    if done.is_err() {
                        return;
                    }
                    if job.msg_id != 0 {
                        self.emit(Event::Delivered { chat: chat.to_owned(), msg_id: job.msg_id });
                    }
                }
                Err(_) => {
                    let attempts = job.attempts + 1;
                    let give_up = attempts >= self.inner.opts.max_attempts;
                    let _ = self.with_bg(|p| {
                        match job.db_id {
                            Some(id) if give_up => {
                                p.history.outbox_done(id)?;
                                if job.msg_id != 0 {
                                    p.history.set_status(job.msg_id, Status::Failed)?;
                                }
                            }
                            Some(id) => p.history.outbox_bump(id)?,
                            None if give_up => {
                                self.inner.temp_out.pop(chat);
                                self.inner.temp.lock().expect("poisoned").failed(job.msg_id);
                            }
                            None => self.inner.temp_out.bump(chat),
                        }
                        Ok(())
                    });
                    if give_up && job.msg_id != 0 {
                        self.emit(Event::SendFailed { chat: chat.to_owned(), msg_id: job.msg_id });
                    }
                    let o = &self.inner.opts;
                    let delay = o.retry_base.saturating_mul(1u32 << attempts.min(16)).min(o.retry_max);
                    self.inner.retry_at.lock().expect("poisoned").insert(chat.to_owned(), Instant::now() + delay);
                    return;
                }
            }
        }
    }

    // ───────────── contacts and invites ─────────────

    pub fn contacts(&self) -> Result<Vec<ContactView>> {
        self.with(|p| {
            Ok(p.engine
                .contacts()
                .into_iter()
                .map(|c| ContactView { id: c.id, name: c.name, onion: c.onion, verified: c.verified })
                .collect())
        })
    }

    /// Returns `(invite, one_time_code)`; share them over different channels.
    pub fn new_invite(&self, label: &str, ttl: Duration) -> Result<(String, String)> {
        self.with(|p| {
            let r = p.engine.new_invite(label, ttl.as_secs(), now_secs()).map_err(guft_store::Error::from)?;
            p.save()?;
            Ok(r)
        })
        .inspect(|_| self.sync_access())
    }

    pub fn pending_invites(&self) -> Result<Vec<(String, u64)>> {
        self.with(|p| Ok(p.engine.pending_invites()))
    }

    pub fn revoke_invite(&self, label: &str) -> Result<()> {
        self.with(|p| {
            p.engine.revoke_invite(label);
            p.save()
        })
        .inspect(|_| self.sync_access())
    }

    /// Import an invite and its code, then queue our hello so they can reply.
    pub async fn add_contact(&self, invite: &str, code: &str) -> Result<String> {
        let (invite, code) = (invite.to_owned(), Zeroizing::new(code.to_owned()));
        let app = self.clone();
        // Argon2 inside: keep it off the async threads.
        let (id, name) = tokio::task::spawn_blocking(move || {
            app.with(|p| {
                let now = now_secs();
                let room = Engine::invite_room(&invite);
                let id = p.engine.add_contact(&invite, &code, now).map_err(guft_store::Error::from)?;
                // Using a room invite is asking to join that room: its invitation needs no second yes.
                if let Some(room) = room {
                    p.history.expect_room(&id, &room, now)?;
                }
                let hello = p.engine.hello_for(&id).map_err(guft_store::Error::from)?;
                let frame = p.engine.encrypt(&id, &hello).map_err(guft_store::Error::from)?;
                p.history.outbox_push(&id, 0, &frame)?;
                p.save()?;
                let name = p.engine.contacts().into_iter().find(|c| c.id == id).map(|c| c.name).unwrap_or_default();
                Ok((id, name))
            })
        })
        .await??;
        self.emit(Event::ContactAdded { id: id.clone(), name });
        self.sync_access();
        self.inner.wake.notify_one();
        Ok(id)
    }

    pub fn safety_number(&self, contact: &str) -> Result<String> {
        self.with(|p| p.engine.safety_number(contact).map_err(Into::into))
    }

    pub fn set_verified(&self, contact: &str, verified: bool) -> Result<()> {
        self.with(|p| {
            p.engine.mark_verified(contact, verified).map_err(guft_store::Error::from)?;
            p.save()
        })
    }

    pub fn remove_contact(&self, contact: &str) -> Result<()> {
        self.with(|p| {
            p.engine.remove_contact(contact);
            p.history.delete_chat(contact)?;
            self.inner.temp_out.drop_contact(contact);
            self.inner.temp.lock().expect("poisoned").drop_invites_from(contact);
            p.history.forget_room_invites_from(contact)?;
            p.save()
        })
        .inspect(|_| self.sync_access())
    }

    // ───────────── messaging ─────────────

    pub fn chats(&self) -> Result<Vec<ChatView>> {
        self.with(|p| {
            let mut summaries: HashMap<_, _> = p.history.summaries()?.into_iter().map(|s| (s.chat.clone(), s)).collect();
            let mut chats: Vec<ChatView> = p
                .engine
                .contacts()
                .into_iter()
                .map(|c| {
                    let s = summaries.remove(&c.id);
                    ChatView {
                        contact: ContactView { id: c.id, name: c.name, onion: c.onion, verified: c.verified },
                        unread: s.as_ref().map_or(0, |s| s.unread),
                        last: s.map(|s| s.last),
                    }
                })
                .collect();
            // Most recent conversation first; contacts with no messages last.
            chats.sort_by_key(|c| std::cmp::Reverse(c.last.as_ref().map(|m| (m.ts, m.id))));
            Ok(chats)
        })
    }

    /// Newest-first page; pass the smallest id you have to get older messages.
    pub fn messages(&self, contact: &str, before: Option<i64>, limit: u32) -> Result<Vec<StoredMessage>> {
        self.with(|p| {
            if is_temp_chat(p, contact) {
                return Ok(self.inner.temp.lock().expect("poisoned").page(contact, before, limit));
            }
            p.history.page(contact, before, limit)
        })
    }

    pub fn mark_read(&self, contact: &str) -> Result<()> {
        self.with(|p| {
            if is_temp_chat(p, contact) {
                self.inner.temp.lock().expect("poisoned").mark_read(contact);
                return Ok(());
            }
            p.history.mark_read(contact)
        })
    }

    pub async fn send_text(&self, contact: &str, text: &str) -> Result<i64> {
        self.send(contact, Payload::Text(text.to_owned())).await
    }

    pub async fn send_file(&self, contact: &str, name: &str, data: Vec<u8>) -> Result<i64> {
        self.send(contact, Payload::File { name: name.to_owned(), data }).await
    }

    async fn send(&self, contact: &str, payload: Payload) -> Result<i64> {
        let id = self.with(|p| {
            // Encrypt, store and queue atomically; the ratchet state is saved right after.
            let frame = p.engine.encrypt(contact, &payload).map_err(guft_store::Error::from)?;
            let id = p.history.add_outgoing(contact, now_secs(), &payload, &frame)?;
            p.save()?;
            Ok(id)
        })?;
        self.inner.retry_at.lock().expect("poisoned").remove(contact);
        self.inner.wake.notify_one();
        Ok(id)
    }

    /// Whether anything encrypted is still waiting to be sent (queued messages, a leave notice...).
    pub fn has_unsent(&self) -> Result<bool> {
        let on_disk = self.with_bg(|p| Ok(!p.history.outbox_chats()?.is_empty()))?;
        Ok(on_disk || !self.inner.temp_out.contacts().is_empty())
    }

    /// Delete one message from this device. If it has not been sent yet, it is never sent.
    pub fn delete_message(&self, msg_id: i64) -> Result<()> {
        self.with(|p| {
            if is_temp_id(msg_id) {
                self.inner.temp.lock().expect("poisoned").remove(msg_id);
                self.inner.temp_out.cancel(msg_id);
                return Ok(());
            }
            p.history.delete_message(msg_id)
        })
    }

    /// Rename a contact on this device only.
    pub fn rename_contact(&self, contact: &str, name: &str) -> Result<()> {
        self.with(|p| {
            p.engine.rename_contact(contact, name).map_err(guft_store::Error::from)?;
            p.save()
        })?;
        self.emit(Event::ContactAdded { id: contact.to_owned(), name: name.trim().to_owned() });
        Ok(())
    }

    pub fn delete_chat(&self, contact: &str) -> Result<()> {
        self.with(|p| {
            if is_temp_chat(p, contact) {
                // The room stays; only what was said is forgotten.
                self.inner.temp.lock().expect("poisoned").drop_room(contact);
                return Ok(());
            }
            p.history.delete_chat(contact)
        })
    }

    // ───────────── rooms ─────────────

    fn view_room(&self, p: &mut Profile, info: RoomInfo) -> guft_store::Result<RoomView> {
        let chat = chat_key(&info.id);
        let temp = info.kind != RoomKind::Normal;
        let (last, unread) = if temp {
            self.inner.temp.lock().expect("poisoned").summary(&chat)
        } else {
            let last = p.history.page(&chat, None, 1)?.into_iter().next();
            let unread = p.history.summaries()?.into_iter().find(|s| s.chat == chat).map_or(0, |s| s.unread);
            (last, unread)
        };
        let connected: std::collections::HashSet<String> = p.engine.contacts().into_iter().map(|c| c.id).collect();
        let direct = info.kind == RoomKind::Direct;
        let members: Vec<MemberView> = info.members.into_iter().map(|m| MemberView { connected: connected.contains(&m.id), id: m.id, name: m.name }).collect();
        // A one-to-one chat is named after the other person.
        let name = if direct { members.first().map(|m| m.name.clone()).unwrap_or(info.name) } else { info.name };
        Ok(RoomView {
            id: chat,
            name,
            creator: info.creator,
            mine: info.mine,
            open_invites: info.open_invites,
            temp,
            direct,
            identity: None,
            members,
            last,
            unread,
        })
    }

    pub fn rooms(&self) -> Result<Vec<RoomView>> {
        self.with(|p| {
            let infos = p.engine.rooms();
            let mut out = Vec::with_capacity(infos.len());
            for info in infos {
                out.push(self.view_room(p, info)?);
            }
            out.sort_by_key(|r| std::cmp::Reverse(r.last.as_ref().map(|m| (m.ts, m.id))));
            Ok(out)
        })
    }

    /// Returns the room's chat id.
    pub fn create_room(&self, name: &str, open_invites: bool) -> Result<String> {
        self.with(|p| {
            let key = p.engine.create_room(name, open_invites).map_err(guft_store::Error::from)?;
            p.save()?;
            Ok(chat_key(&key))
        })
        .inspect(|id| self.emit(Event::RoomChanged { room: id.clone() }))
    }

    /// A group room that lives in memory only. Returns the room's chat id.
    pub fn create_temp_room(&self, name: &str, open_invites: bool) -> Result<String> {
        self.with(|p| {
            // Nothing to save: the room is not part of the persistent state.
            let key = p.engine.create_temp_room(name, open_invites).map_err(guft_store::Error::from)?;
            Ok(chat_key(&key))
        })
        .inspect(|id| self.emit(Event::RoomChanged { room: id.clone() }))
    }

    /// One tap: a memory-only chat with one contact (reuses the open one). Returns its chat id.
    pub fn start_temp_chat(&self, contact: &str) -> Result<String> {
        let id = self.with(|p| {
            let (key, invite) = p.engine.start_direct_temp(contact).map_err(guft_store::Error::from)?;
            if let Some(invite) = invite {
                queue_ctl(p, &self.inner.temp_out, true, contact, &invite)?;
                p.save()?;
            }
            Ok(chat_key(&key))
        })?;
        self.emit(Event::RoomChanged { room: id.clone() });
        self.inner.wake.notify_one();
        Ok(id)
    }

    /// An invite that also adds whoever uses it to the room. Returns `(invite, code)`.
    pub fn new_room_invite(&self, room: &str, label: &str, ttl: Duration) -> Result<(String, String)> {
        let key = room_of(room)?.to_owned();
        self.with(|p| {
            let r = p.engine.new_room_invite(&key, label, ttl.as_secs(), now_secs()).map_err(guft_store::Error::from)?;
            p.save()?;
            Ok(r)
        })
        .inspect(|_| self.sync_access())
    }

    /// Add someone you already talk to; they join straight away.
    pub fn add_contact_to_room(&self, room: &str, contact: &str) -> Result<()> {
        let key = room_of(room)?.to_owned();
        self.with(|p| {
            let temp = p.engine.is_temp_room(&key);
            let invite = p.engine.add_contact_to_room(&key, contact).map_err(guft_store::Error::from)?;
            queue_ctl(p, &self.inner.temp_out, temp, contact, &invite)?;
            p.save()
        })?;
        self.emit(Event::RoomChanged { room: room.to_owned() });
        self.inner.wake.notify_one();
        Ok(())
    }

    fn queue_all(&self, out: Vec<(String, Payload)>, p: &mut Profile, temp: bool) -> guft_store::Result<()> {
        for (to, payload) in out {
            queue_ctl(p, &self.inner.temp_out, temp, &to, &payload)?;
        }
        Ok(())
    }

    pub fn leave_room(&self, room: &str) -> Result<()> {
        let key = room_of(room)?.to_owned();
        self.with(|p| {
            let temp = p.engine.is_temp_room(&key);
            let out = p.engine.leave_room(&key).map_err(guft_store::Error::from)?;
            self.queue_all(out, p, temp)?;
            if temp {
                self.inner.temp.lock().expect("poisoned").drop_room(&chat_key(&key));
            } else {
                p.history.delete_chat(&chat_key(&key))?;
            }
            p.save()
        })?;
        self.emit(Event::RoomRemoved { room: room.to_owned() });
        self.inner.wake.notify_one();
        Ok(())
    }

    pub fn remove_room_member(&self, room: &str, member: &str) -> Result<()> {
        let key = room_of(room)?.to_owned();
        self.with(|p| {
            let temp = p.engine.is_temp_room(&key);
            let out = p.engine.remove_member(&key, member).map_err(guft_store::Error::from)?;
            self.queue_all(out, p, temp)?;
            p.save()
        })?;
        self.emit(Event::RoomChanged { room: room.to_owned() });
        self.inner.wake.notify_one();
        Ok(())
    }

    pub fn rename_room(&self, room: &str, name: &str) -> Result<()> {
        let key = room_of(room)?.to_owned();
        self.with(|p| {
            let temp = p.engine.is_temp_room(&key);
            let out = p.engine.rename_room(&key, name).map_err(guft_store::Error::from)?;
            self.queue_all(out, p, temp)?;
            p.save()
        })?;
        self.emit(Event::RoomChanged { room: room.to_owned() });
        self.inner.wake.notify_one();
        Ok(())
    }

    pub async fn send_room_text(&self, room: &str, text: &str) -> Result<i64> {
        let id = parse_room_key(room_of(room)?)?;
        self.send_room(room, Payload::RoomText { room: id, text: text.to_owned() }, Payload::Text(text.to_owned())).await
    }

    pub async fn send_room_file(&self, room: &str, name: &str, data: Vec<u8>) -> Result<i64> {
        let id = parse_room_key(room_of(room)?)?;
        self.send_room(room, Payload::RoomFile { room: id, name: name.to_owned(), data: data.clone() }, Payload::File { name: name.to_owned(), data }).await
    }

    /// Encrypt once per member, store one message, queue every frame, all atomically.
    async fn send_room(&self, room: &str, wire: Payload, stored: Payload) -> Result<i64> {
        let key = room_of(room)?.to_owned();
        let id = self.with(|p| {
            let recipients = p.engine.room_recipients(&key).map_err(guft_store::Error::from)?;
            let temp = p.engine.is_temp_room(&key);
            if temp && recipients.iter().any(|to| self.inner.temp_out.queued(to) >= MAX_QUEUED_PER_CONTACT) {
                return Err(guft_core::Error::Limit("too many unsent temporary messages").into());
            }
            let mut frames = Vec::with_capacity(recipients.len());
            for to in recipients {
                let frame = p.engine.encrypt(&to, &wire).map_err(guft_store::Error::from)?;
                frames.push((to, frame));
            }
            let id = if temp {
                // Memory only: the message and its frames never reach the database.
                let id = self
                    .inner
                    .temp
                    .lock()
                    .expect("poisoned")
                    .add(&chat_key(&key), now_secs(), true, None, &stored, frames.len())
                    .ok_or(guft_core::Error::Invalid("not a message"))?;
                for (to, frame) in frames {
                    self.inner.temp_out.push(&to, TempOut { msg_id: id, frame, attempts: 0 });
                }
                id
            } else {
                p.history.add_outgoing_multi(&chat_key(&key), now_secs(), &stored, &frames)?
            };
            p.save()?;
            Ok(id)
        })?;
        self.inner.wake.notify_one();
        Ok(id)
    }

    /// The name and bytes of a stored file message (for playing a voice note in the UI).
    pub fn file_bytes(&self, msg_id: i64) -> Result<(String, Vec<u8>)> {
        let (name, data) = self.load_file(msg_id)?;
        check_file_name(&name)?;
        Ok((name, data))
    }

    fn load_file(&self, msg_id: i64) -> Result<(String, Vec<u8>)> {
        let found = if is_temp_id(msg_id) {
            self.with(|_| Ok(self.inner.temp.lock().expect("poisoned").file(msg_id)))?
        } else {
            self.with(|p| {
                let msg = p.history.message(msg_id)?;
                let data = p.history.file_bytes(msg_id)?;
                Ok(msg.zip(data))
            })?
            .and_then(|(m, d)| match m.body {
                guft_store::Body::File { name, .. } => Some((name, d)),
                _ => None,
            })
        };
        found.ok_or(AppError::Core(guft_core::Error::Invalid("not a file")))
    }

    /// Write a received file into `dir` under its (validated) name; never overwrites.
    pub fn save_file(&self, msg_id: i64, dir: &Path) -> Result<PathBuf> {
        let (name, data) = self.file_bytes(msg_id)?;
        std::fs::create_dir_all(dir)?;
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) => (s.to_owned(), format!(".{e}")),
            None => (name.clone(), String::new()),
        };
        for n in 0..1000 {
            let candidate = if n == 0 { name.clone() } else { format!("{stem} ({n}){ext}") };
            match private_new_file(&dir.join(&candidate)) {
                Ok(mut f) => {
                    f.write_all(&data)?;
                    f.sync_all()?;
                    return Ok(dir.join(candidate));
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Err(AppError::Io("too many files with that name".into()))
    }
}
