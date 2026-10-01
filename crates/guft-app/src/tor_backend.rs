//! Production backend: our own onion service plus a Tor client, via `guft-net`.

use std::path::Path;

use guft_net::access::Access;
use guft_net::tor::{onion_address, TorConnector, TorDirs, TorNode, Vanguards};
use guft_net::{Net, NetConfig};

use crate::backend::{NetworkBackend, Running};
use crate::Result;

pub struct TorBackend {
    dirs: TorDirs,
    cfg: NetConfig,
}

impl TorBackend {
    /// Tor's own state and cache live under the profile directory.
    pub fn new(profile_dir: &Path, cfg: NetConfig) -> Self {
        Self { dirs: TorDirs { state: profile_dir.join("tor/state"), cache: profile_dir.join("tor/cache"), vanguards: Vanguards::default() }, cfg }
    }
}

impl NetworkBackend for TorBackend {
    type T = Net<TorConnector>;

    fn onion_for_seed(&self, seed: &[u8; 32]) -> Result<String> {
        Ok(onion_address(seed))
    }

    async fn start(&self, seed: &[u8; 32], access: Access) -> Result<Running<Self::T>> {
        let (node, inbound) = TorNode::start(&self.dirs, seed, self.cfg.clone(), access).await?;
        let (transport, onion) = (node.net(), node.onion().to_owned());
        Ok(Running {
            transport,
            inbound,
            onion,
            access: node.clone(),
            stop: Box::new(move || {
                Box::pin(async move {
                    node.shutdown().await;
                    // Dropping the last handle shuts the Tor client and onion service down.
                    drop(node);
                })
            }),
        })
    }
}
