use thiserror::Error;

#[derive(Debug, Error)]
pub enum CryptoError {
    #[error("ciphertext too short")]
    CiphertextShort,
    /// Deliberately merges wrong-key / AAD-mismatch / tampering: distinguishing
    /// them would hand an attacker an oracle. Port of Go's ErrDecrypt.
    #[error("decryption failed (wrong key, corrupted data, or AAD mismatch)")]
    Decrypt,
    #[error("encryption failed")]
    Seal,
    #[error("key must be {expected} bytes, got {got}")]
    BadKeyLen { expected: usize, got: usize },
    #[error("salt must be at least {min} bytes, got {got}")]
    BadSaltLen { min: usize, got: usize },
    #[error("password must not be empty")]
    EmptyPassword,
    #[error("HKDF info label must not be empty")]
    EmptyInfoLabel,
    #[error("HKDF expand failed")]
    HkdfExpand,
    #[error("KDF parameters below the production floor: {0}")]
    WeakKdfParams(&'static str),
    #[error("Argon2 failed (likely out of memory at {m_kib} KiB)")]
    KdfFailed { m_kib: u32 },
    #[error("system entropy source unavailable")]
    NoEntropy,
}
