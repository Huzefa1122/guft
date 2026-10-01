//! Who may reach us, and which keys we present to others.
//!
//! These are Tor "client authorization" keys. Our onion service only reveals how
//! to reach it to holders of an authorized key, so a stranger who learns the
//! address still cannot connect. Secrets come from the vault; the network layer
//! turns them into the public keys Tor needs.

use zeroize::Zeroizing;

use crate::Result;

pub type Key = Zeroizing<[u8; 32]>;

#[derive(Clone, Default)]
pub struct Access {
    /// Keys allowed to discover our service: `(nickname, secret)`.
    pub authorized: Vec<(String, Key)>,
    /// Keys to present when connecting out: `(their onion, secret)`.
    pub connect: Vec<(String, Key)>,
}

impl PartialEq for Access {
    fn eq(&self, other: &Self) -> bool {
        fn same(a: &[(String, Key)], b: &[(String, Key)]) -> bool {
            a.len() == b.len() && a.iter().zip(b).all(|((n1, k1), (n2, k2))| n1 == n2 && **k1 == **k2)
        }
        same(&self.authorized, &other.authorized) && same(&self.connect, &other.connect)
    }
}

pub trait AccessControl: Send + Sync + 'static {
    /// Replace the full set. Idempotent.
    fn update(&self, access: Access) -> Result<()>;
}
