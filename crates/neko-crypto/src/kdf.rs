//! Argon2id password stretching.

use zeroize::Zeroizing;

use crate::error::CryptoError;

/// Production floor. Parameters below this are rejected at load time so a
/// tampered database cannot silently weaken the KDF.
pub const MIN_MEM_KIB: u32 = 65_536; // 64 MiB
pub const MIN_ITERS: u32 = 2;
pub const MIN_SALT_LEN: usize = 16;
pub const MIN_KEY_LEN: u32 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Argon2idParams {
    pub mem_kib: u32,
    pub iters: u32,
    pub par: u8,
    pub key_len: u32,
}

impl Argon2idParams {
    /// Fixed 13-byte encoding, folded into the vault AAD so that tampering with
    /// the stored cost parameters breaks decryption outright instead of quietly
    /// lowering the work factor.
    ///
    /// Layout: `mem_kib(4) || iters(4) || par(1) || key_len(4)`, big-endian.
    pub fn encode(&self) -> [u8; 13] {
        let mut b = [0u8; 13];
        b[0..4].copy_from_slice(&self.mem_kib.to_be_bytes());
        b[4..8].copy_from_slice(&self.iters.to_be_bytes());
        b[8] = self.par;
        b[9..13].copy_from_slice(&self.key_len.to_be_bytes());
        b
    }

    pub fn validate(&self) -> Result<(), CryptoError> {
        if self.mem_kib < MIN_MEM_KIB {
            return Err(CryptoError::WeakKdfParams("mem_kib below 64 MiB"));
        }
        if self.iters < MIN_ITERS {
            return Err(CryptoError::WeakKdfParams("iters below 2"));
        }
        if self.par == 0 {
            return Err(CryptoError::WeakKdfParams("par must be >= 1"));
        }
        if self.key_len < MIN_KEY_LEN {
            return Err(CryptoError::WeakKdfParams("key_len below 32"));
        }
        Ok(())
    }
}

/// Argon2id(password, salt) -> key_len bytes.
pub fn derive_key(
    password: &[u8],
    salt: &[u8],
    p: Argon2idParams,
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    if password.is_empty() {
        return Err(CryptoError::EmptyPassword);
    }
    if salt.len() < MIN_SALT_LEN {
        return Err(CryptoError::BadSaltLen {
            min: MIN_SALT_LEN,
            got: salt.len(),
        });
    }
    let params = argon2::Params::new(p.mem_kib, p.iters, p.par as u32, Some(p.key_len as usize))
        .map_err(|_| CryptoError::WeakKdfParams("argon2 rejected the parameter set"))?;
    let a = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);

    let mut out = Zeroizing::new(vec![0u8; p.key_len as usize]);
    a.hash_password_into(password, salt, &mut out)
        .map_err(|_| CryptoError::KdfFailed { m_kib: p.mem_kib })?;
    Ok(out)
}
