//! XChaCha20-Poly1305 envelope encryption.
//!
//! XChaCha20 (24-byte nonce) rather than AES-GCM: the nonce is large enough to
//! be generated randomly without any counter state, and it does not depend on
//! AES-NI being present. Wire format is `nonce(24) || ciphertext || tag(16)`.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::XChaCha20Poly1305;
use zeroize::Zeroizing;

use crate::aad::Aad;
use crate::error::CryptoError;

pub const KEY_LEN: usize = 32;
pub const NONCE_LEN: usize = 24;
pub const TAG_LEN: usize = 16;

fn cipher(key: &[u8]) -> Result<XChaCha20Poly1305, CryptoError> {
    if key.len() != KEY_LEN {
        return Err(CryptoError::BadKeyLen {
            expected: KEY_LEN,
            got: key.len(),
        });
    }
    Ok(XChaCha20Poly1305::new(key.into()))
}

/// Encrypt `plaintext`, prefixing a fresh random nonce. Callers never manage nonces.
pub fn seal(key: &[u8], plaintext: &[u8], aad: Aad<'_>) -> Result<Vec<u8>, CryptoError> {
    let c = cipher(key)?;
    let mut nonce = [0u8; NONCE_LEN];
    getrandom::getrandom(&mut nonce).map_err(|_| CryptoError::NoEntropy)?;

    let ct = c
        .encrypt(
            &nonce.into(),
            Payload {
                msg: plaintext,
                aad: &aad.encode(),
            },
        )
        .map_err(|_| CryptoError::Seal)?;

    let mut out = Vec::with_capacity(NONCE_LEN + ct.len());
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&ct);
    Ok(out)
}

/// Decrypt the output of [`seal`].
///
/// The error deliberately does not distinguish "wrong key" from "AAD mismatch"
/// from "tampered" — that distinction is an oracle.
pub fn open(key: &[u8], sealed: &[u8], aad: Aad<'_>) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    let c = cipher(key)?;
    if sealed.len() < NONCE_LEN + TAG_LEN {
        return Err(CryptoError::CiphertextShort);
    }
    let (nonce, ct) = sealed.split_at(NONCE_LEN);
    c.decrypt(
        nonce.into(),
        Payload {
            msg: ct,
            aad: &aad.encode(),
        },
    )
    .map(Zeroizing::new)
    .map_err(|_| CryptoError::Decrypt)
}
