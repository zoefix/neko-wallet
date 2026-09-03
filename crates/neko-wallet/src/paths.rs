//! Where the vault file lives.
//!
//! The requirement is "drop the .db back into the program directory and
//! everything comes back", so the executable's own directory wins whenever it
//! is usable. The OS data directory is only a fallback for installs into
//! read-only locations (a macOS /Applications bundle, a system package).

use std::path::{Path, PathBuf};

pub const FILE_NAME: &str = "neko-wallet.db";
pub const ENV_VAR: &str = "NEKO_WALLET_DB";

/// Resolution order: `--db` > `$NEKO_WALLET_DB` > the saved setting >
/// next to the executable > OS data directory.
///
/// The saved setting sits below the flag and the variable on purpose, so a
/// one-off `--db other.db` can be used to look at another vault without
/// disturbing the configured one.
pub fn resolve(explicit: Option<PathBuf>) -> PathBuf {
    if let Some(p) = explicit {
        return p;
    }
    if let Some(p) = std::env::var_os(ENV_VAR) {
        return PathBuf::from(p);
    }
    if let Some(p) = crate::config::load().db {
        // Returned even when the file has gone missing. Falling through to the
        // next candidate would quietly open a *different* wallet, and a user
        // who believes they are looking at vault A while looking at vault B is
        // in a far worse position than one who gets a clear complaint.
        return p;
    }
    if let Some(dir) = exe_dir() {
        let candidate = dir.join(FILE_NAME);
        // An existing vault next to the binary always wins, even if the
        // directory is now read-only -- we still want to report it, not
        // silently start a second empty vault somewhere else.
        if candidate.exists() || is_writable(&dir) {
            return candidate;
        }
    }
    data_dir().join(FILE_NAME)
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(Path::to_path_buf)
}

fn is_writable(dir: &Path) -> bool {
    let probe = dir.join(".neko-write-probe");
    match std::fs::File::create(&probe) {
        Ok(_) => {
            let _ = std::fs::remove_file(&probe);
            true
        }
        Err(_) => false,
    }
}

fn data_dir() -> PathBuf {
    directories::ProjectDirs::from("dev", "neko", "neko-wallet")
        .map(|d| d.data_dir().to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."))
}
