use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("SQLCipher rejected the key")]
    KeyRejected,
    #[error("database file not found: {0}")]
    NotFound(PathBuf),
    #[error("file is too small to be a neko-wallet vault")]
    TooSmall,
    #[error("a vault already exists at {0}")]
    AlreadyExists(PathBuf),
    #[error("schema version {found} is newer than this build supports ({max})")]
    FutureSchema { found: i32, max: i32 },
    /// The worst failure mode in the system: base58 and raw address drifting
    /// apart means deposits are silently missed with no error anywhere.
    #[error("address/address_raw mismatch on row {0}; refusing to start")]
    AddressDrift(i64),
    #[error("stale WAL sidecar files present; the vault was not closed cleanly")]
    StaleWal,
    #[error("no wallet with id {0}")]
    NoSuchWallet(i64),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Crypto(#[from] neko_crypto::CryptoError),
    #[error(transparent)]
    Vault(#[from] neko_vault::VaultError),
}
