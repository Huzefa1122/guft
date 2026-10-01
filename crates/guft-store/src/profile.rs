//! One user profile on disk: `state.nc` (sealed engine state) + `history.db`.

use std::fs::{self, File, OpenOptions};
use std::io::Write;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use guft_core::vault::Vault;
use guft_core::Engine;

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
    // On Unix the directory is private to the user; on Windows the per-user profile ACL applies.
    #[cfg(unix)]
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

impl Profile {
    pub fn exists(dir: &Path) -> bool {
        dir.join(STATE_FILE).is_file()
    }

    pub fn create(dir: &Path, passphrase: &str, display_name: &str) -> Result<Self> {
        Self::create_with(dir, display_name, Vault::create(passphrase))
    }

    /// For a profile protected by a random 256-bit secret instead of a human passphrase
    /// (a separate identity kept inside another profile): opening it is cheap.
    pub fn create_for_random_secret(dir: &Path, secret: &str, display_name: &str) -> Result<Self> {
        Self::create_with(dir, display_name, Vault::create_for_random_secret(secret))
    }

    fn create_with(dir: &Path, display_name: &str, vault: guft_core::Result<Vault>) -> Result<Self> {
        if Self::exists(dir) {
            return Err(Error::Exists);
        }
        private_dir(dir)?;
        let vault = vault?;
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
        self.write_sealed(STATE_FILE, &self.engine.to_bytes()?)
    }

    fn write_sealed(&self, file: &str, plain: &[u8]) -> Result<()> {
        let sealed = self.vault.seal(plain)?;
        let tmp = self.dir.join(format!("{file}.tmp"));
        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        opts.mode(0o600);
        let mut f = opts.open(&tmp)?;
        f.write_all(&sealed)?;
        f.sync_all()?;
        fs::rename(&tmp, self.dir.join(file))?;
        File::open(&self.dir)?.sync_all()?;
        Ok(())
    }

    fn aux_file(name: &str) -> Result<String> {
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
            return Err(guft_core::Error::Invalid("bad file name").into());
        }
        Ok(format!("{name}.nc"))
    }

    /// Small extra data sealed with the same key as the profile (for example the list of
    /// separate identities). `None` if it was never written.
    pub fn read_aux(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let file = Self::aux_file(name)?;
        match fs::read(self.dir.join(&file)) {
            Ok(blob) => Ok(Some(self.vault.open(&blob)?.to_vec())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn write_aux(&self, name: &str, plain: &[u8]) -> Result<()> {
        self.write_sealed(&Self::aux_file(name)?, plain)
    }

    /// Save, then drop everything secret.
    pub fn lock(self) -> Result<()> {
        let saved = self.save();
        let closed = self.history.close();
        // `vault` and `engine` are wiped as they drop here.
        saved.and(closed)
    }
}
