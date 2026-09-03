//! Settings that have to be readable *before* a vault is opened.
//!
//! There is exactly one so far: which vault file to use. It cannot live inside
//! the vault, because you need it to find the vault - so this is a small
//! plaintext TOML file in the user's config directory.
//!
//! Plaintext is not a weakness here. The file holds a path, not a secret, and
//! anyone who can write it could equally well replace the executable that reads
//! it. What it *must* do is fail safe: a corrupt or unreadable config falls back
//! to the normal search order rather than stopping the program, because being
//! locked out of a wallet is a far worse outcome than losing a preference.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "config.toml";

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// Absolute path to the vault. Always stored absolute: a relative path
    /// would resolve against whatever directory the user happened to be in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub db: Option<PathBuf>,
}

/// Where the config file lives, or `None` if the OS will not tell us.
pub fn path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("NEKO_WALLET_CONFIG") {
        return Some(PathBuf::from(p));
    }
    directories::ProjectDirs::from("dev", "neko", "neko-wallet")
        .map(|d| d.config_dir().join(FILE_NAME))
}

pub fn load() -> Config {
    load_from(path().as_deref())
}

pub fn load_from(p: Option<&Path>) -> Config {
    p.and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(cfg: &Config) -> std::io::Result<PathBuf> {
    let p = path().ok_or_else(|| {
        std::io::Error::other("could not work out where to put the configuration file")
    })?;
    save_to(&p, cfg)?;
    Ok(p)
}

pub fn save_to(p: &Path, cfg: &Config) -> std::io::Result<()> {
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let body = toml::to_string_pretty(cfg).map_err(std::io::Error::other)?;
    // Write-then-rename, so an interrupted save cannot leave a half-written
    // file that reads as "no vault configured" on the next launch.
    let tmp = p.with_extension("toml.tmp");
    std::fs::write(&tmp, body)?;
    std::fs::rename(&tmp, p)?;
    Ok(())
}

/// Why a path is not usable as a vault.
#[derive(Debug)]
pub enum VaultCheck {
    /// An existing file with a valid neko-wallet header.
    Ok {
        profile: &'static str,
    },
    Missing,
    /// Exists, but is not one of ours. Pointing the wallet at it would show the
    /// first-run screen and invite the user to overwrite something.
    NotAVault(String),
}

/// Does this path hold a neko-wallet vault?
///
/// Worth doing before saving. The failure this prevents is quiet and nasty: a
/// mistyped path makes the next launch show first-run setup, and a user who
/// takes that at face value creates a second empty vault and believes the
/// original is gone.
pub fn check_vault(p: &Path) -> VaultCheck {
    let Ok(bytes) = std::fs::read(p) else {
        return VaultCheck::Missing;
    };
    if bytes.len() < neko_vault::HEADER_LEN {
        return VaultCheck::NotAVault(format!("only {} bytes long", bytes.len()));
    }
    // By far the most likely wrong file, and the generic header error for it -
    // "format v83", because 'S' is 83 - explains nothing. An unencrypted
    // database where a vault should be is also worth naming loudly: if it
    // really did come from this program, something is very wrong.
    if bytes.starts_with(b"SQLite format 3\0") {
        return VaultCheck::NotAVault(
            "it is an unencrypted SQLite database, not an encrypted vault".into(),
        );
    }
    match neko_vault::FileHeader::parse(&bytes[..neko_vault::HEADER_LEN]) {
        Ok(h) => match h.profile() {
            Ok(profile) => VaultCheck::Ok {
                profile: profile.name,
            },
            Err(e) => VaultCheck::NotAVault(e.to_string()),
        },
        Err(e) => VaultCheck::NotAVault(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_disk() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nested").join(FILE_NAME);
        let cfg = Config {
            db: Some(PathBuf::from("/Users/zoe/wallets/main.db")),
        };
        save_to(&p, &cfg).unwrap();
        assert_eq!(load_from(Some(&p)), cfg);
    }

    /// A damaged config must never be the reason the wallet will not start.
    #[test]
    fn a_broken_config_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        std::fs::write(&p, b"this is not toml {{{").unwrap();
        assert_eq!(load_from(Some(&p)), Config::default());
        assert_eq!(
            load_from(Some(&dir.path().join("absent"))),
            Config::default()
        );
        assert_eq!(load_from(None), Config::default());
    }

    #[test]
    fn saving_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE_NAME);
        save_to(&p, &Config::default()).unwrap();
        let extra: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n != FILE_NAME)
            .collect();
        assert!(extra.is_empty(), "left behind {extra:?}");
    }

    #[test]
    fn a_real_vault_is_recognised() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("v.db");
        let h = neko_vault::FileHeader::new(neko_vault::profile::LIGHT).unwrap();
        let mut bytes = h.as_bytes().to_vec();
        bytes.extend_from_slice(&[0u8; 4096]);
        std::fs::write(&p, bytes).unwrap();
        assert!(matches!(check_vault(&p), VaultCheck::Ok { profile } if profile == "light"));
    }

    /// The check exists to stop a typo from becoming a second empty wallet.
    #[test]
    fn other_files_are_refused() {
        let dir = tempfile::tempdir().unwrap();

        assert!(matches!(
            check_vault(&dir.path().join("nope")),
            VaultCheck::Missing
        ));

        let short = dir.path().join("short");
        std::fs::write(&short, b"tiny").unwrap();
        assert!(matches!(check_vault(&short), VaultCheck::NotAVault(_)));

        // A plain, unencrypted SQLite database: the single most likely wrong
        // file for somebody to point this at.
        let sqlite = dir.path().join("other.db");
        let mut b = b"SQLite format 3\0".to_vec();
        b.extend_from_slice(&[0u8; 4096]);
        std::fs::write(&sqlite, b).unwrap();
        assert!(matches!(check_vault(&sqlite), VaultCheck::NotAVault(_)));

        // Right shape, unknown KDF profile: a vault from a future version.
        let future = dir.path().join("future.db");
        let mut b = vec![0x01, 0x7f];
        b.extend_from_slice(&[0xAB; 14]);
        b.extend_from_slice(&[0u8; 4096]);
        std::fs::write(&future, b).unwrap();
        assert!(matches!(check_vault(&future), VaultCheck::NotAVault(_)));
    }
}
