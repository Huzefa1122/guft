//! In-memory network for tests and demos: streams are local pipes, "onion
//! addresses" are just names. Lets the whole stack run without Tor.

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};
use tokio::sync::mpsc;
use tokio_util::compat::{Compat, TokioAsyncReadCompatExt};

use sha2::{Digest, Sha256};

use crate::access::{Access, AccessControl, Key};
use crate::link::{Connector, Inbound, Receiver};
use crate::{Error, Result};

/// Stand-in for the public key a secret maps to (the real one is X25519 in `tor`).
fn mem_public(secret: &[u8; 32]) -> [u8; 32] {
    let mut out = [0u8; 32];
    out.copy_from_slice(&Sha256::digest([b"mem client auth".as_slice(), secret].concat()));
    out
}

#[derive(Default)]
pub struct MemNetwork {
    nodes: Mutex<HashMap<String, Arc<Receiver>>>,
    offline: Mutex<HashSet<String>>,
    serving: Mutex<HashMap<String, Vec<tokio::task::AbortHandle>>>,
    /// Nodes that enforce authorization, with the public keys they accept.
    authorized: Mutex<HashMap<String, HashSet<[u8; 32]>>>,
    /// Connection attempts refused for lack of a valid key.
    pub denied: AtomicU64,
    /// Streams opened to each address, and bytes read per (address, slot).
    pub opened: Mutex<HashMap<String, u64>>,
    pub bytes: Mutex<HashMap<(String, usize), Arc<AtomicU64>>>,
}

impl MemNetwork {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register a node; frames sent to `onion` appear on the returned channel.
    pub fn register(&self, onion: &str) -> mpsc::Receiver<Inbound> {
        let (tx, rx) = mpsc::channel(64);
        self.nodes.lock().unwrap().insert(onion.to_owned(), Receiver::new(tx));
        rx
    }

    pub fn unregister(&self, onion: &str) {
        self.nodes.lock().unwrap().remove(onion);
        self.sever(onion);
    }

    /// Drop every open stream to `onion`, like circuits dying with the peer.
    fn sever(&self, onion: &str) {
        if let Some(handles) = self.serving.lock().unwrap().remove(onion) {
            handles.into_iter().for_each(|h| h.abort());
        }
    }

    /// Whether a node with this name is currently registered.
    pub fn is_registered(&self, onion: &str) -> bool {
        self.nodes.lock().unwrap().contains_key(onion)
    }

    pub fn set_online(&self, onion: &str, online: bool) {
        let mut off = self.offline.lock().unwrap();
        if online {
            off.remove(onion);
        } else {
            off.insert(onion.to_owned());
            drop(off);
            self.sever(onion);
        }
    }

    /// A connector that presents no keys (reaches only nodes without authorization).
    pub fn connector(self: &Arc<Self>) -> MemConnector {
        MemConnector { net: self.clone(), keys: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Make `onion` require a valid client key; initially nobody has one.
    pub fn require_auth(&self, onion: &str) {
        self.authorized.lock().unwrap().entry(onion.to_owned()).or_default();
    }

    /// A node's control handle plus a connector that presents that node's keys.
    pub fn node_access(self: &Arc<Self>, onion: &str) -> (MemAccess, MemConnector) {
        let keys = Arc::new(Mutex::new(HashMap::new()));
        (
            MemAccess { net: self.clone(), onion: onion.to_owned(), keys: keys.clone() },
            MemConnector { net: self.clone(), keys },
        )
    }
}

pub struct MemConnector {
    net: Arc<MemNetwork>,
    keys: Arc<Mutex<HashMap<String, Key>>>,
}

pub struct MemAccess {
    net: Arc<MemNetwork>,
    onion: String,
    keys: Arc<Mutex<HashMap<String, Key>>>,
}

impl AccessControl for MemAccess {
    fn update(&self, access: Access) -> Result<()> {
        let accepted = access.authorized.iter().map(|(_, k)| mem_public(k)).collect();
        self.net.authorized.lock().unwrap().insert(self.onion.clone(), accepted);
        *self.keys.lock().unwrap() = access.connect.into_iter().collect();
        Ok(())
    }
}

/// Counts bytes read, so tests can see that every circuit carried traffic.
pub struct Counting<T> {
    inner: T,
    count: Arc<AtomicU64>,
}

impl<T: AsyncRead + Unpin> AsyncRead for Counting<T> {
    fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<std::io::Result<()>> {
        let before = buf.filled().len();
        let r = Pin::new(&mut self.inner).poll_read(cx, buf);
        self.count.fetch_add((buf.filled().len() - before) as u64, Ordering::Relaxed);
        r
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for Counting<T> {
    fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, b: &[u8]) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, b)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

impl Connector for MemConnector {
    type Stream = Compat<DuplexStream>;

    async fn connect(&self, onion: &str, slot: usize) -> Result<Self::Stream> {
        if self.net.offline.lock().unwrap().contains(onion) {
            return Err(Error::Io("host unreachable".into()));
        }
        let Some(receiver) = self.net.nodes.lock().unwrap().get(onion).cloned() else {
            return Err(Error::Io("no such onion service".into()));
        };
        if let Some(accepted) = self.net.authorized.lock().unwrap().get(onion) {
            let ok = self.keys.lock().unwrap().get(onion).is_some_and(|k| accepted.contains(&mem_public(k)));
            if !ok {
                self.net.denied.fetch_add(1, Ordering::Relaxed);
                return Err(Error::Io("not authorized".into()));
            }
        }
        *self.net.opened.lock().unwrap().entry(onion.to_owned()).or_default() += 1;
        let count = self.net.bytes.lock().unwrap().entry((onion.to_owned(), slot)).or_default().clone();
        let (a, b) = tokio::io::duplex(256 * 1024);
        let handle = tokio::spawn(receiver.serve(Counting { inner: b, count }.compat()));
        self.net.serving.lock().unwrap().entry(onion.to_owned()).or_default().push(handle.abort_handle());
        Ok(a.compat())
    }
}
