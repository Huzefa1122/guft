//! How the app gets a network. Production uses Tor (`TorBackend`); tests use an
//! in-memory network.

use std::future::Future;
use std::sync::Arc;

use futures::future::BoxFuture;
use guft_net::access::{Access, AccessControl};
use guft_net::{Inbound, Transport};
use tokio::sync::mpsc;

use crate::Result;

/// A started network: send through `transport`, receive from `inbound`.
pub struct Running<T> {
    pub transport: Arc<T>,
    pub inbound: mpsc::Receiver<Inbound>,
    pub onion: String,
    /// Push new authorization keys when contacts or invites change.
    pub access: Arc<dyn AccessControl>,
    /// Tears everything down (onion service, circuits). Called on lock.
    pub stop: Box<dyn FnOnce() -> BoxFuture<'static, ()> + Send>,
}

pub trait NetworkBackend: Send + Sync + 'static {
    type T: Transport;

    /// The onion address a seed maps to. Pure, so invites work before Tor is up.
    fn onion_for_seed(&self, seed: &[u8; 32]) -> Result<String>;

    /// `access` is the initial authorization state (who may reach us, keys we present).
    fn start(&self, seed: &[u8; 32], access: Access) -> impl Future<Output = Result<Running<Self::T>>> + Send;
}

#[cfg(feature = "mem")]
pub use mem::MemBackend;

#[cfg(feature = "mem")]
mod mem {
    use super::*;
    use guft_net::mem::{MemConnector, MemNetwork};
    use guft_net::{Net, NetConfig};
    use sha2::{Digest, Sha512};

    pub struct MemBackend {
        pub net: Arc<MemNetwork>,
        pub cfg: NetConfig,
    }

    impl MemBackend {
        pub fn new(net: Arc<MemNetwork>, cfg: NetConfig) -> Self {
            Self { net, cfg }
        }
    }

    impl NetworkBackend for MemBackend {
        type T = Net<MemConnector>;

        fn onion_for_seed(&self, seed: &[u8; 32]) -> Result<String> {
            // 35 bytes -> exactly 56 base32 chars, so it passes the onion-address check.
            let digest = Sha512::digest(seed);
            let host = data_encoding::BASE32_NOPAD.encode(&digest[..35]).to_lowercase();
            Ok(format!("{host}.onion"))
        }

        async fn start(&self, seed: &[u8; 32], access: Access) -> Result<Running<Self::T>> {
            let onion = self.onion_for_seed(seed)?;
            let inbound = self.net.register(&onion);
            // Like the real service, a node is closed to strangers until keys are authorized.
            self.net.require_auth(&onion);
            let (control, connector) = self.net.node_access(&onion);
            control.update(access)?;
            let transport = Net::new(connector, self.cfg.clone())?;
            let (net, name, t2) = (self.net.clone(), onion.clone(), transport.clone());
            Ok(Running {
                transport,
                inbound,
                onion,
                access: Arc::new(control),
                stop: Box::new(move || {
                    Box::pin(async move {
                        net.unregister(&name);
                        t2.close_all().await;
                    })
                }),
            })
        }
    }
}
