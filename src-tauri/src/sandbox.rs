//! Where our data lives and how tightly the process is confined.

use std::path::PathBuf;

pub struct Dirs {
    pub profile: PathBuf,
    pub downloads: PathBuf,
    write: Vec<PathBuf>,
}

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

impl Dirs {
    pub fn resolve() -> Self {
        let home = env_path("HOME").unwrap_or_else(|| PathBuf::from("/tmp"));
        let data = env_path("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
        let cache = env_path("XDG_CACHE_HOME").unwrap_or_else(|| home.join(".cache"));
        let profile = env_path("GUFT_PROFILE_DIR").unwrap_or_else(|| data.join("guft"));
        let downloads = env_path("XDG_DOWNLOAD_DIR").unwrap_or_else(|| home.join("Downloads")).join("guft");

        // Everything the process may write: our data, the toolkit's caches, temp and device nodes.
        let mut write = vec![profile.clone(), downloads.clone(), cache, data.join("dev.guft.app"), PathBuf::from("/tmp"), PathBuf::from("/dev")];
        if let Some(rt) = env_path("XDG_RUNTIME_DIR") {
            write.push(rt);
        }
        Self { profile, downloads, write }
    }
}

/// Lock the process down. Debug builds can opt out to ease development.
pub fn apply(dirs: &Dirs) {
    if cfg!(debug_assertions) && std::env::var_os("GUFT_NO_SANDBOX").is_some() {
        eprintln!("guft: sandbox disabled by GUFT_NO_SANDBOX (debug build)");
        return;
    }
    let policy = guft_harden::Policy { write_dirs: dirs.write.clone(), skip_filesystem: false };
    match guft_harden::harden(&policy) {
        Ok(report) => {
            if cfg!(debug_assertions) {
                eprintln!("guft: sandbox {report:?}");
            }
        }
        // A security product must not silently run unprotected.
        Err(e) => {
            eprintln!("guft: could not apply the sandbox: {e}");
            std::process::exit(2);
        }
    }
}
