//! The real network: an Arti Tor client plus our own onion service.
//!
//! * The onion identity is derived from a seed that lives in the encrypted
//!   vault. Arti's keystore is configured in-memory, so the service key never
//!   touches disk in plaintext.
//! * Vanguards are on in full mode (extra layered guards against guard discovery).
//! * Each (contact, slot) pair gets its own isolation token, so the parallel
//!   streams to one contact, and streams to different contacts, never share a circuit.
//! * Path length is left at Tor's default on purpose: a custom length would make
//!   our circuits stand out from everyone else's.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use arti_client::config::TorClientConfigBuilder;
use arti_client::{DataStream, HsId, IsolationToken, KeystoreSelector, StreamPrefs, TorClient};
use futures::StreamExt;
use safelog::DisplayRedacted;
use rand::TryRngCore;
use tokio::sync::mpsc;
use zeroize::Zeroizing;
use tokio::task::JoinHandle;
use tor_cell::relaycell::msg::{Connected, End};
use tor_proto::stream::IncomingStreamRequest;
use tor_config::ExplicitOrAuto;
use tor_guardmgr::VanguardMode;
use tor_hscrypto::pk::{HsClientDescEncKey, HsClientDescEncSecretKey, HsIdKey, HsIdKeypair};
use tor_config::Reconfigure;
use tor_hsservice::config::{OnionServiceConfig, OnionServiceConfigBuilder};
use tor_hsservice::{handle_rend_requests, HsNickname, RunningOnionService};
use tor_keymgr::config::ArtiKeystoreKind;
use tor_llcrypto::pk::curve25519::StaticSecret;
use tor_llcrypto::pk::ed25519::{ExpandedKeypair, Keypair};
use tor_rtcompat::PreferredRuntime;

use crate::access::{Access, AccessControl, Key};
use crate::link::{Connector, Inbound, Net, NetConfig, Receiver};
use crate::{Error, Result};

/// The virtual port our onion service listens on.
pub const SERVICE_PORT: u16 = 7777;
const NICKNAME: &str = "guft";
/// Streams a single client circuit may open to us; we only ever use one.
const MAX_STREAMS_PER_CIRCUIT: u32 = 4;

type Client = TorClient<PreferredRuntime>;

fn tor_err(e: impl std::fmt::Display) -> Error {
    Error::Tor(e.to_string())
}

/// How strongly to protect against guard discovery (see Tor's "vanguards").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Vanguards {
    Disabled,
    Lite,
    #[default]
    Full,
}

pub struct TorDirs {
    /// Persistent but non-secret state (guards, service replay logs).
    pub state: PathBuf,
    /// Downloaded directory documents.
    pub cache: PathBuf,
    pub vanguards: Vanguards,
}

fn keypair_from_seed(seed: &[u8; 32]) -> HsIdKeypair {
    let kp = Keypair::from_bytes(seed);
    HsIdKeypair::from(ExpandedKeypair::from(&kp))
}

/// The `.onion` address a seed maps to. Pure and instant: no network needed.
pub fn onion_address(seed: &[u8; 32]) -> String {
    let hsid = HsId::from(HsIdKey::from(&keypair_from_seed(seed)));
    let address = hsid.display_unredacted().to_string();
    address
}

fn client_secret(k: &[u8; 32]) -> HsClientDescEncSecretKey {
    HsClientDescEncSecretKey::from(StaticSecret::from(*k))
}

fn client_public(k: &[u8; 32]) -> HsClientDescEncKey {
    HsClientDescEncKey::from(&client_secret(k))
}

/// The service config with restricted discovery on: only holders of an authorized key can
/// find our introduction points. A throwaway key keeps the config valid with no contacts.
fn service_config(authorized: &[(String, Key)], placeholder: &[u8; 32]) -> Result<OnionServiceConfig> {
    let mut b = OnionServiceConfigBuilder::default();
    b.nickname(NICKNAME.parse::<HsNickname>().map_err(tor_err)?).max_concurrent_streams_per_circuit(MAX_STREAMS_PER_CIRCUIT);
    let rd = b.restricted_discovery();
    rd.enabled(true);
    let keys = rd.static_keys();
    // `HsClientNickname` is not re-exported by Arti; the parse target is inferred from `push`.
    keys.access().push(("placeholder".parse().map_err(tor_err)?, client_public(placeholder)));
    for (label, secret) in authorized {
        keys.access().push((label.parse().map_err(tor_err)?, client_public(secret)));
    }
    b.build().map_err(tor_err)
}

pub struct TorConnector {
    client: Arc<Client>,
    tokens: Mutex<HashMap<(String, usize), IsolationToken>>,
    /// Keys we should present, and the ones already installed in Tor's keystore.
    wanted: Mutex<HashMap<String, Key>>,
    installed: Mutex<HashMap<String, [u8; 32]>>,
}

impl TorConnector {
    fn set_keys(&self, keys: Vec<(String, Key)>) {
        *self.wanted.lock().expect("poisoned") = keys.into_iter().collect();
    }

    /// Make sure Tor holds the right client key for this service before connecting.
    fn install_key(&self, onion: &str) -> Result<()> {
        let Some(want) = self.wanted.lock().expect("poisoned").get(onion).cloned() else { return Ok(()) };
        if self.installed.lock().expect("poisoned").get(onion) == Some(&*want) {
            return Ok(());
        }
        let hsid: HsId = onion.parse().map_err(tor_err)?;
        // Replace any older key for this service (a contact's key changes once, after the invite).
        let _ = self.client.remove_service_discovery_key(KeystoreSelector::Primary, hsid);
        self.client.insert_service_discovery_key(KeystoreSelector::Primary, hsid, client_secret(&want)).map_err(tor_err)?;
        self.installed.lock().expect("poisoned").insert(onion.to_owned(), *want);
        Ok(())
    }
}

impl Connector for TorConnector {
    type Stream = DataStream;

    async fn connect(&self, onion: &str, slot: usize) -> Result<DataStream> {
        self.install_key(onion)?;
        let token = *self
            .tokens
            .lock()
            .expect("poisoned")
            .entry((onion.to_owned(), slot))
            .or_insert_with(IsolationToken::new);
        let mut prefs = StreamPrefs::new();
        prefs.set_isolation(token);
        self.client.connect_with_prefs((onion, SERVICE_PORT), &prefs).await.map_err(tor_err)
    }
}

pub struct TorNode {
    _client: Arc<Client>,
    service: Arc<RunningOnionService>,
    onion: String,
    net: Arc<Net<TorConnector>>,
    accept: JoinHandle<()>,
    placeholder: Key,
}

impl TorNode {
    /// Bootstrap Tor, publish our onion service, and start accepting frames.
    pub async fn start(dirs: &TorDirs, seed: &[u8; 32], cfg: NetConfig, access: Access) -> Result<(Arc<Self>, mpsc::Receiver<Inbound>)> {
        cfg.validate()?;
        let mut b = TorClientConfigBuilder::from_directories(&dirs.state, &dirs.cache);
        b.storage().keystore().primary().kind(ExplicitOrAuto::Explicit(ArtiKeystoreKind::Ephemeral));
        b.address_filter().allow_onion_addrs(true);
        let mode = match dirs.vanguards {
            Vanguards::Disabled => VanguardMode::Disabled,
            Vanguards::Lite => VanguardMode::Lite,
            Vanguards::Full => VanguardMode::Full,
        };
        b.vanguards().mode(ExplicitOrAuto::Explicit(mode));
        let config = b.build().map_err(tor_err)?;

        let client = TorClient::create_bootstrapped(config).await.map_err(tor_err)?;

        let mut placeholder = Zeroizing::new([0u8; 32]);
        rand::rngs::OsRng.try_fill_bytes(&mut *placeholder).map_err(tor_err)?;
        let service_cfg = service_config(&access.authorized, &placeholder)?;
        let (service, rend_requests) = client
            .launch_onion_service_with_hsid(service_cfg, keypair_from_seed(seed))
            .map_err(tor_err)?
            .ok_or_else(|| Error::Tor("onion service disabled".into()))?;

        let onion = onion_address(seed);
        let (tx, rx) = mpsc::channel(64);
        let receiver = Receiver::new(tx);
        let mut requests = Box::pin(handle_rend_requests(rend_requests));
        let accept = tokio::spawn(async move {
            while let Some(req) = requests.next().await {
                match req.request() {
                    IncomingStreamRequest::Begin(begin) if begin.port() == SERVICE_PORT => {
                        let receiver = receiver.clone();
                        tokio::spawn(async move {
                            if let Ok(stream) = req.accept(Connected::new_empty()).await {
                                receiver.serve(stream).await;
                            }
                        });
                    }
                    _ => {
                        let _ = req.reject(End::new_misc()).await;
                    }
                }
            }
        });

        let connector = TorConnector {
            client: client.clone(),
            tokens: Mutex::new(HashMap::new()),
            wanted: Mutex::new(HashMap::new()),
            installed: Mutex::new(HashMap::new()),
        };
        connector.set_keys(access.connect);
        let net = Net::new(connector, cfg)?;
        Ok((Arc::new(Self { _client: client, service, onion, net, accept, placeholder }), rx))
    }

    pub fn onion(&self) -> &str {
        &self.onion
    }

    /// The address Arti itself reports for the running service (must equal [`onion`](Self::onion)).
    pub fn service_onion(&self) -> Option<String> {
        self.service.onion_address().map(|h| h.display_unredacted().to_string())
    }

    pub fn net(&self) -> Arc<Net<TorConnector>> {
        self.net.clone()
    }

    /// Wait until the service's descriptor is published and it can be reached.
    /// On failure the error says what state the service was last in, and why.
    pub async fn wait_reachable(&self, max: Duration) -> Result<()> {
        let mut events = self.service.status_events();
        let mut last = String::from("no status received");
        let waited = tokio::time::timeout(max, async {
            while let Some(status) = events.next().await {
                last = format!("{:?}, problem: {:?}", status.state(), status.current_problem());
                if status.state().is_fully_reachable() {
                    return true;
                }
            }
            false
        })
        .await;
        match waited {
            Ok(true) => Ok(()),
            Ok(false) => Err(Error::Tor(format!("onion service stopped ({last})"))),
            Err(_) => Err(Error::Tor(format!("onion service not reachable in time ({last})"))),
        }
    }

    /// Close links and stop accepting. Dropping the node then shuts Tor down.
    pub async fn shutdown(&self) {
        self.accept.abort();
        self.net.close_all().await;
    }
}

impl AccessControl for TorNode {
    fn update(&self, access: Access) -> Result<()> {
        self.net.connector().set_keys(access.connect);
        let cfg = service_config(&access.authorized, &self.placeholder)?;
        self.service.reconfigure(cfg, Reconfigure::AllOrNothing).map_err(tor_err)
    }
}
