//! HKDF-SHA512 subkey derivation and HMAC-SHA256.

use hkdf::Hkdf;
use hmac::{Mac, SimpleHmac};
use sha2::{Sha256, Sha512};
use zeroize::Zeroizing;

use crate::aead::KEY_LEN;
use crate::error::CryptoError;

/// Subkey labels. Every label must be globally unique and never reused —
/// identical `info` derives an identical key, re-coupling purposes that are
/// meant to be isolated. The `-v1` suffix reserves the ability to rotate a
/// single subkey later without changing MK.
pub mod info {
    /// Whole-file SQLCipher key. Derived from `stretched`, NOT from MK — anything
    /// needed to open the database must come from the password alone.
    pub const FILE_KEY: &str = "neko/file-key-v1";
    /// Wraps MK. Also derived from `stretched`.
    pub const KEK: &str = "neko/kek-v1";
    /// Field-level AEAD key. Derived from MK.
    pub const DATA_KEY: &str = "neko/data-key-v1";
    /// Blind-index HMAC key, for equality lookup over encrypted columns.
    pub const BLIND_INDEX: &str = "neko/blind-index-v1";
    /// MK correctness self-check.
    pub const VERIFIER: &str = "neko/verifier-v1";
    /// Per-wallet material namespace (suffixed with a big-endian u32 sequence).
    pub const WALLET_SEED: &str = "neko/wallet-seed-v1";
}

/// Derive a purpose-isolated subkey from a 32-byte master key.
pub fn derive(mk: &[u8], info: &str, length: usize) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    if mk.len() != KEY_LEN {
        return Err(CryptoError::BadKeyLen {
            expected: KEY_LEN,
            got: mk.len(),
        });
    }
    derive_from_ikm(mk, info, length)
}

/// 32-byte convenience wrapper around [`derive`].
pub fn derive_key32(mk: &[u8], info: &str) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    derive(mk, info, KEY_LEN)
}

/// Like [`derive`] but accepts input keying material of any length, for
/// combining multiple sources.
pub fn derive_from_ikm(
    ikm: &[u8],
    info: &str,
    length: usize,
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    if ikm.is_empty() {
        return Err(CryptoError::BadKeyLen {
            expected: 1,
            got: 0,
        });
    }
    if info.is_empty() {
        return Err(CryptoError::EmptyInfoLabel);
    }
    let hk = Hkdf::<Sha512>::new(None, ikm);
    let mut out = Zeroizing::new(vec![0u8; length]);
    hk.expand(info.as_bytes(), &mut out)
        .map_err(|_| CryptoError::HkdfExpand)?;
    Ok(out)
}

/// HKDF with an explicit salt, used where a per-database random salt should
/// harden a derivation without paying for a second Argon2 run.
pub fn derive_with_salt(
    ikm: &[u8],
    salt: &[u8],
    info: &str,
    length: usize,
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    if info.is_empty() {
        return Err(CryptoError::EmptyInfoLabel);
    }
    let hk = Hkdf::<Sha512>::new(Some(salt), ikm);
    let mut out = Zeroizing::new(vec![0u8; length]);
    hk.expand(info.as_bytes(), &mut out)
        .map_err(|_| CryptoError::HkdfExpand)?;
    Ok(out)
}

/// HMAC-SHA256 over the concatenation of `parts`. Used for blind indexes.
pub fn hmac_sha256(key: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut m = <SimpleHmac<Sha256> as Mac>::new_from_slice(key)
        .expect("SimpleHmac accepts any key length");
    for p in parts {
        m.update(p);
    }
    m.finalize().into_bytes().into()
}
