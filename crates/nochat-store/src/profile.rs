//! One user profile on disk: `state.nc` (sealed engine state) + `history.db`.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use nochat_core::vault::Vault;
use nochat_core::Engine;

use crate::history::History;
use crate::{Error, Result};

const STATE_FILE: &str = "state.nc";
const HISTORY_FILE: &str = "history.db";

/// An unlocked profile. Dropping or [`lock`](Profile::lock)ing it wipes the
/// master key and the decrypted state from memory.
pub struct Profile {
    dir: PathBuf,
    vault: Vault,
    pub engine: Engine,
    pub history: History,
}

fn private_dir(dir: &Path) -> Result<()> {
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

impl Profile {
    pub fn exists(dir: &Path) -> bool {
        dir.join(STATE_FILE).is_file()
    }

    pub fn create(dir: &Path, passphrase: &str, display_name: &str) -> Result<Self> {
        if Self::exists(dir) {
            return Err(Error::Exists);
        }
        private_dir(dir)?;
        let vault = Vault::create(passphrase)?;
        let engine = Engine::create(display_name)?;
        let history = History::open(&dir.join(HISTORY_FILE), &vault.subkey("history"))?;
        let p = Self { dir: dir.to_owned(), vault, engine, history };
        p.save()?;
        Ok(p)
    }

    pub fn unlock(dir: &Path, passphrase: &str) -> Result<Self> {
        let blob = fs::read(dir.join(STATE_FILE)).map_err(|_| Error::Missing)?;
        let (vault, plain) = Vault::unlock(passphrase, &blob)?;
        let engine = Engine::from_bytes(&plain)?;
        let history = History::open(&dir.join(HISTORY_FILE), &vault.subkey("history"))?;
        Ok(Self { dir: dir.to_owned(), vault, engine, history })
    }

    /// Atomic write: temp file (0600), fsync, rename, fsync the directory.
    pub fn save(&self) -> Result<()> {
        let sealed = self.vault.seal(&self.engine.to_bytes()?)?;
        let tmp = self.dir.join(format!("{STATE_FILE}.tmp"));
        let mut f = OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
        f.write_all(&sealed)?;
        f.sync_all()?;
        fs::rename(&tmp, self.dir.join(STATE_FILE))?;
        File::open(&self.dir)?.sync_all()?;
        Ok(())
    }

    /// Save, then drop everything secret.
    pub fn lock(self) -> Result<()> {
        let saved = self.save();
        let closed = self.history.close();
        // `vault` and `engine` are wiped as they drop here.
        saved.and(closed)
    }
}
