use thiserror::Error;

#[derive(Debug, Error)]
pub enum VaultError {
    /// The ONLY error a failed login may produce. It must never reveal which
    /// input was wrong — that would be an oracle.
    #[error("email or password is incorrect")]
    WrongCredentials,
    #[error("vault is locked")]
    Locked,
    #[error("password too weak: {0}")]
    WeakPassword(String),
    #[error("this file is not a neko-wallet vault")]
    NotANekoVault,
    #[error("vault file format v{0} is newer than this build supports")]
    FutureFormat(u8),
    #[error("unknown KDF profile id {0}")]
    UnknownKdfProfile(u8),
    /// The plaintext header disagrees with the authenticated in-database record.
    #[error("KDF profile mismatch between file header and vault record")]
    KdfProfileMismatch,
    #[error("vault record is corrupt or has been tampered with")]
    VaultBlobCorrupt,
    #[error("a password change was interrupted; run `neko-wallet vault repair`")]
    RewrapIncomplete,
    #[error(transparent)]
    Crypto(#[from] neko_crypto::CryptoError),
}
