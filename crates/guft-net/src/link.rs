//! Sending and receiving frames over several independent circuits.
//!
//! The sender keeps at least [`MIN_CIRCUITS`] streams to each contact, each on its
//! own Tor circuit, and spreads a frame's cells across them with small random
//! delays. Cover cells keep the links busy while a conversation is active. The
//! receiver rebuilds frames from any circuit, hands them to the app, and sends an
//! acknowledgement only once the app has accepted the frame.
//!
//! Everything here is generic over `AsyncRead + AsyncWrite` streams, so it is
//! tested without Tor and run on Tor by the `tor` module.

use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::{Duration, Instant};

use futures::future::join_all;
use futures::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadHalf, WriteHalf};
use rand::Rng;
use tokio::sync::{mpsc, oneshot, Mutex, Notify, Semaphore};
use tokio::time::{sleep, timeout};

use crate::cell::{ack_cell, cover_cell, fragment, frame_id, parse, FrameId, Kind, RawCell, CELL_SIZE};
use crate::reassembly::Reassembler;
use crate::{Error, Result};

/// Fewest circuits kept to one contact. Fixed by design; not configurable below this.
pub const MIN_CIRCUITS: usize = 5;
/// Concurrent inbound streams we serve.
pub const MAX_STREAMS: usize = 128;
const RATE_WINDOW: Duration = Duration::from_secs(10);
const RATE_MAX_CELLS: u32 = 300;
const RECEIVER_IDLE: Duration = Duration::from_secs(20 * 60);
const HANDLER_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug)]
pub struct NetConfig {
    /// Streams opened per contact (each on its own circuit).
    pub circuits: usize,
    /// A send needs at least this many live streams.
    pub min_circuits: usize,
    /// Mean gap between cover cells on a link; `None` turns cover traffic off.
    pub cover_mean: Option<Duration>,
    /// Each data cell is delayed by a random amount up to this.
    pub max_jitter: Duration,
    pub ack_timeout: Duration,
    pub connect_timeout: Duration,
    /// Links with no real traffic for this long are closed.
    pub idle_close: Duration,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            circuits: 6,
            min_circuits: MIN_CIRCUITS,
            cover_mean: Some(Duration::from_secs(4)),
            max_jitter: Duration::from_millis(150),
            ack_timeout: Duration::from_secs(90),
            connect_timeout: Duration::from_secs(60),
            idle_close: Duration::from_secs(10 * 60),
        }
    }
}

impl NetConfig {
    pub fn validate(&self) -> Result<()> {
        if self.min_circuits < MIN_CIRCUITS || self.circuits < self.min_circuits {
            return Err(Error::Protocol("at least 5 circuits per contact are required"));
        }
        Ok(())
    }
}

/// A frame the app must accept or refuse.
pub struct Inbound {
    pub frame: Vec<u8>,
    /// `true` = accepted (we acknowledge); `false` = not accepted (sender retries later).
    pub ack: oneshot::Sender<bool>,
}

pub trait Transport: Send + Sync + 'static {
    /// Resolves once the peer has acknowledged the frame.
    fn send(&self, onion: &str, frame: Vec<u8>) -> impl Future<Output = Result<()>> + Send;

    /// Close every open link (on lock). Default: nothing to close.
    fn close_links(&self) -> impl Future<Output = ()> + Send {
        async {}
    }
}

pub trait Connector: Send + Sync + 'static {
    type Stream: AsyncRead + AsyncWrite + Unpin + Send + 'static;
    /// Open a stream to `onion`; `slot` selects which isolated circuit to use.
    fn connect(&self, onion: &str, slot: usize) -> impl Future<Output = Result<Self::Stream>> + Send;
}

async fn read_cell<R: AsyncRead + Unpin>(r: &mut R) -> std::io::Result<RawCell> {
    let mut buf = [0u8; CELL_SIZE];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_cell<W: AsyncWrite + Unpin>(w: &mut W, cell: &RawCell) -> std::io::Result<()> {
    w.write_all(cell).await?;
    w.flush().await
}

// ───────────────────────────── receiving ─────────────────────────────

pub struct Receiver {
    reasm: StdMutex<Reassembler>,
    tx: mpsc::Sender<Inbound>,
    streams: Arc<Semaphore>,
}

impl Receiver {
    pub fn new(tx: mpsc::Sender<Inbound>) -> Arc<Self> {
        Arc::new(Self { reasm: StdMutex::new(Reassembler::new()), tx, streams: Arc::new(Semaphore::new(MAX_STREAMS)) })
    }

    /// Serve one inbound stream until it ends, misbehaves, or goes idle.
    pub async fn serve<S>(self: Arc<Self>, stream: S)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send,
    {
        let Ok(_permit) = self.streams.clone().try_acquire_owned() else { return };
        let (mut rd, mut wr) = stream.split();
        let (mut window_start, mut in_window) = (Instant::now(), 0u32);
        loop {
            let Ok(Ok(raw)) = timeout(RECEIVER_IDLE, read_cell(&mut rd)).await else { return };
            if window_start.elapsed() >= RATE_WINDOW {
                (window_start, in_window) = (Instant::now(), 0);
            }
            in_window += 1;
            if in_window > RATE_MAX_CELLS {
                return;
            }
            let Ok(cell) = parse(&raw) else { return };
            match cell.kind {
                Kind::Cover | Kind::Ack => continue,
                Kind::Data => {}
            }
            let pushed = self.reasm.lock().expect("poisoned").push(cell, Instant::now());
            let frame = match pushed {
                Ok(Some(frame)) => frame,
                Ok(None) => continue,
                Err(_) => return,
            };
            let id = frame_id(&frame);
            let (ack_tx, ack_rx) = oneshot::channel();
            if self.tx.send(Inbound { frame, ack: ack_tx }).await.is_err() {
                return;
            }
            let accepted = matches!(timeout(HANDLER_TIMEOUT, ack_rx).await, Ok(Ok(true)));
            if accepted {
                let cell = ack_cell(&id, &mut rand::rng());
                if write_cell(&mut wr, &cell).await.is_err() {
                    return;
                }
            }
        }
    }
}

// ───────────────────────────── sending ─────────────────────────────

type Pending = Arc<StdMutex<HashMap<FrameId, oneshot::Sender<()>>>>;

struct Slot<S> {
    writer: Mutex<Option<WriteHalf<S>>>,
    alive: AtomicBool,
    /// Bumped on every (re)connect so a late-exiting old reader cannot close a newer stream.
    generation: AtomicU64,
    /// Wakes the current reader; replaced on every (re)connect so no stale wake-up survives.
    stop: StdMutex<Arc<Notify>>,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send + 'static> Slot<S> {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            writer: Mutex::new(None),
            alive: AtomicBool::new(false),
            generation: AtomicU64::new(0),
            stop: StdMutex::new(Arc::new(Notify::new())),
        })
    }

    fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    fn stop_reader(&self) {
        self.stop.lock().expect("poisoned").notify_one();
    }

    async fn install(self: &Arc<Self>, stream: S, pending: Pending) {
        let (rd, wr) = stream.split();
        let stop = Arc::new(Notify::new());
        let mut writer = self.writer.lock().await;
        *self.stop.lock().expect("poisoned") = stop.clone();
        let generation = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        *writer = Some(wr);
        self.alive.store(true, Ordering::Release);
        drop(writer);
        tokio::spawn(Self::reader(self.clone(), rd, pending, generation, stop));
    }

    /// Reads acknowledgements (and ignores cover) until the stream ends or is closed.
    async fn reader(self: Arc<Self>, mut rd: ReadHalf<S>, pending: Pending, generation: u64, stop: Arc<Notify>) {
        loop {
            tokio::select! {
                _ = stop.notified() => break,
                res = read_cell(&mut rd) => {
                    let Ok(raw) = res else { break };
                    let Ok(cell) = parse(&raw) else { break };
                    if cell.kind == Kind::Ack {
                        if let Some(tx) = pending.lock().expect("poisoned").remove(&cell.frame_id) {
                            let _ = tx.send(());
                        }
                    }
                }
            }
        }
        let mut writer = self.writer.lock().await;
        if self.generation.load(Ordering::Acquire) == generation {
            writer.take();
            self.alive.store(false, Ordering::Release);
        }
    }

    async fn write(&self, cell: &RawCell) -> std::io::Result<()> {
        let mut guard = self.writer.lock().await;
        let Some(w) = guard.as_mut() else {
            return Err(std::io::ErrorKind::NotConnected.into());
        };
        let res = write_cell(w, cell).await;
        if res.is_err() {
            *guard = None;
            self.alive.store(false, Ordering::Release);
            self.stop_reader();
        }
        res
    }

    async fn close(&self) {
        let mut writer = self.writer.lock().await;
        writer.take();
        self.alive.store(false, Ordering::Release);
        self.stop_reader();
    }
}

struct Link<S> {
    slots: Vec<Arc<Slot<S>>>,
    pending: Pending,
    last_activity: StdMutex<Instant>,
    connecting: Mutex<()>,
    cover_running: AtomicBool,
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send + 'static> Link<S> {
    fn new(circuits: usize) -> Arc<Self> {
        Arc::new(Self {
            slots: (0..circuits).map(|_| Slot::new()).collect(),
            pending: Arc::new(StdMutex::new(HashMap::new())),
            last_activity: StdMutex::new(Instant::now()),
            connecting: Mutex::new(()),
            cover_running: AtomicBool::new(false),
        })
    }

    fn touch(&self) {
        *self.last_activity.lock().expect("poisoned") = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        self.last_activity.lock().expect("poisoned").elapsed()
    }

    fn alive_slots(&self) -> Vec<Arc<Slot<S>>> {
        self.slots.iter().filter(|s| s.is_alive()).cloned().collect()
    }

    /// Reconnect any dead slots, then insist on the minimum.
    async fn ensure<C: Connector<Stream = S>>(self: &Arc<Self>, conn: &C, onion: &str, cfg: &NetConfig) -> Result<()> {
        let _guard = self.connecting.lock().await;
        let dead: Vec<usize> = (0..self.slots.len()).filter(|i| !self.slots[*i].is_alive()).collect();
        let attempts = dead.iter().map(|&i| async move { (i, timeout(cfg.connect_timeout, conn.connect(onion, i)).await) });
        for (i, res) in join_all(attempts).await {
            if let Ok(Ok(stream)) = res {
                self.slots[i].install(stream, self.pending.clone()).await;
            }
        }
        let have = self.alive_slots().len();
        if have < cfg.min_circuits {
            return Err(Error::NotEnoughCircuits { have, need: cfg.min_circuits });
        }
        if let Some(mean) = cfg.cover_mean {
            if !self.cover_running.swap(true, Ordering::AcqRel) {
                tokio::spawn(cover_task(Arc::downgrade(self), mean, cfg.idle_close));
            }
        }
        Ok(())
    }

    async fn send_frame(&self, frame: &[u8], cfg: &NetConfig) -> Result<()> {
        let id = frame_id(frame);
        let cells = fragment(frame, &mut rand::rng())?;
        let alive = self.alive_slots();
        if alive.len() < cfg.min_circuits {
            return Err(Error::NotEnoughCircuits { have: alive.len(), need: cfg.min_circuits });
        }
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("poisoned").insert(id, tx);

        // Round-robin the cells over the circuits, starting at a random one.
        let start = rand::rng().random_range(0..alive.len());
        let mut per_slot: Vec<Vec<RawCell>> = vec![Vec::new(); alive.len()];
        for (i, c) in cells.into_iter().enumerate() {
            per_slot[(start + i) % alive.len()].push(c);
        }
        let max_ms = cfg.max_jitter.as_millis() as u64;
        let writers = alive.iter().zip(per_slot).map(|(slot, cells)| async move {
            for c in cells {
                let ms = if max_ms == 0 { 0 } else { rand::rng().random_range(0..=max_ms) };
                if ms > 0 {
                    sleep(Duration::from_millis(ms)).await;
                }
                if slot.write(&c).await.is_err() {
                    break;
                }
            }
        });
        join_all(writers).await;
        self.touch();

        match timeout(cfg.ack_timeout, rx).await {
            Ok(Ok(())) => Ok(()),
            _ => {
                self.pending.lock().expect("poisoned").remove(&id);
                Err(Error::AckTimeout)
            }
        }
    }

    async fn close_all(&self) {
        for s in &self.slots {
            s.close().await;
        }
    }
}

/// Sends cover cells while a link is active, then closes the idle link.
async fn cover_task<S: AsyncRead + AsyncWrite + Unpin + Send + 'static>(link: Weak<Link<S>>, mean: Duration, idle_close: Duration) {
    loop {
        // Exponential gaps, clamped, so the rhythm is not a fixed beat.
        let u: f64 = rand::rng().random();
        let gap = mean.mul_f64((-(1.0 - u).ln()).clamp(0.2, 5.0));
        sleep(gap).await;
        let Some(link) = link.upgrade() else { return };
        if link.idle_for() >= idle_close {
            link.close_all().await;
            link.cover_running.store(false, Ordering::Release);
            return;
        }
        let alive = link.alive_slots();
        if alive.is_empty() {
            link.cover_running.store(false, Ordering::Release);
            return;
        }
        let pick = rand::rng().random_range(0..alive.len());
        let cell = cover_cell(&mut rand::rng());
        let _ = alive[pick].write(&cell).await;
    }
}

/// Sends frames to onion addresses over per-contact multi-circuit links.
pub struct Net<C: Connector> {
    conn: Arc<C>,
    cfg: NetConfig,
    links: Mutex<HashMap<String, Arc<Link<C::Stream>>>>,
}

impl<C: Connector> Net<C> {
    pub fn new(conn: C, cfg: NetConfig) -> Result<Arc<Self>> {
        cfg.validate()?;
        Ok(Arc::new(Self { conn: Arc::new(conn), cfg, links: Mutex::new(HashMap::new()) }))
    }

    pub fn connector(&self) -> &Arc<C> {
        &self.conn
    }

    /// Close every link (used on lock and shutdown).
    pub async fn close_all(&self) {
        let links: Vec<_> = self.links.lock().await.drain().map(|(_, l)| l).collect();
        for l in links {
            l.close_all().await;
        }
    }
}

impl<C: Connector> Transport for Net<C> {
    async fn send(&self, onion: &str, frame: Vec<u8>) -> Result<()> {
        let link = {
            let mut links = self.links.lock().await;
            links.entry(onion.to_owned()).or_insert_with(|| Link::new(self.cfg.circuits)).clone()
        };
        link.ensure(&*self.conn, onion, &self.cfg).await?;
        link.send_frame(&frame, &self.cfg).await
    }

    async fn close_links(&self) {
        self.close_all().await;
    }
}
