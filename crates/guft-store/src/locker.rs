//! Session lock: holds the unlocked profile, relocks on idle or on demand, and
//! slows down repeated wrong passwords.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::profile::Profile;
use crate::{Error, Result};

pub struct Locker {
    dir: PathBuf,
    profile: Option<Profile>,
    idle_timeout: Duration,
    last_activity: Instant,
    failures: u32,
    next_try: Instant,
}

impl Locker {
    pub fn new(dir: PathBuf, idle_timeout: Duration) -> Self {
        let now = Instant::now();
        Self { dir, profile: None, idle_timeout, last_activity: now, failures: 0, next_try: now }
    }

    pub fn is_unlocked(&self) -> bool {
        self.profile.is_some()
    }

    pub fn set_idle_timeout(&mut self, t: Duration) {
        self.idle_timeout = t;
    }

    pub fn unlock(&mut self, passphrase: &str) -> Result<()> {
        let now = Instant::now();
        if now < self.next_try {
            return Err(Error::Backoff((self.next_try - now).as_secs().max(1)));
        }
        match Profile::unlock(&self.dir, passphrase) {
            Ok(p) => {
                self.profile = Some(p);
                self.failures = 0;
                self.touch();
                Ok(())
            }
            Err(e) => {
                if matches!(e, Error::Core(guft_core::Error::Vault)) {
                    self.failures += 1;
                    // 0, 0, 0, 2s, 4s, 8s ... capped at 5 minutes.
                    let wait = if self.failures > 3 { (1u64 << (self.failures - 2).min(9)).min(300) } else { 0 };
                    self.next_try = now + Duration::from_secs(wait);
                }
                Err(e)
            }
        }
    }

    /// Run `f` against the unlocked profile; counts as activity.
    pub fn with<T>(&mut self, f: impl FnOnce(&mut Profile) -> Result<T>) -> Result<T> {
        self.tick()?;
        self.touch();
        f(self.profile.as_mut().ok_or(Error::Locked)?)
    }

    /// Like [`with`](Self::with) but does NOT count as user activity, so background
    /// work (retries, incoming messages) can never keep the session open.
    pub fn peek<T>(&mut self, f: impl FnOnce(&mut Profile) -> Result<T>) -> Result<T> {
        self.tick()?;
        f(self.profile.as_mut().ok_or(Error::Locked)?)
    }

    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Call periodically. Locks if idle too long; returns true if it just locked.
    pub fn tick(&mut self) -> Result<bool> {
        if self.profile.is_some() && self.last_activity.elapsed() >= self.idle_timeout {
            self.lock()?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Save and wipe. Safe to call when already locked.
    pub fn lock(&mut self) -> Result<()> {
        match self.profile.take() {
            Some(p) => p.lock(),
            None => Ok(()),
        }
    }
}

impl Drop for Locker {
    fn drop(&mut self) {
        let _ = self.lock();
    }
}
