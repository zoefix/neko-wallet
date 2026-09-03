//! The 16-byte plaintext file header.
//!
//! These are the only bytes of the database we control that are readable
//! *without* a key: SQLCipher stores its per-file salt in bytes 0..16 and reads
//! it back with a raw `sqlite3OsRead(fd, salt, 16, 0)`. We supply those bytes
//! ourselves via the 92-hex keyspec at creation time, which solves the
//! chicken-and-egg problem (you need the salt before you can decrypt anything).
//!
//! Layout:
//! ```text
//! byte 0      fmt_ver     format version
//! byte 1      kdf_profile index into the frozen profile table
//! byte 2..16  file_rand   14 random bytes (112 bits, unique per database)
//! ```
//!
//! The profile byte is deliberately *unauthenticated*. Flipping it makes Argon2
//! produce a different `stretched`, hence a different file key, hence the
//! database simply fails to open. A downgrade attack is self-defeating: it can
//! deny service but never weaken the KDF. The authenticated copy of the full
//! parameters lives inside the encrypted `vault` row and is cross-checked there.

use crate::error::VaultError;
use crate::profile::{self, Profile};

pub const HEADER_LEN: usize = 16;
pub const FMT_V1: u8 = 0x01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileHeader {
    pub fmt_ver: u8,
    pub kdf_profile: u8,
    pub file_rand: [u8; 14],
}

impl FileHeader {
    pub fn new(profile: Profile) -> Result<Self, VaultError> {
        let r = neko_crypto::random(14)?;
        let mut file_rand = [0u8; 14];
        file_rand.copy_from_slice(&r);
        Ok(Self {
            fmt_ver: FMT_V1,
            kdf_profile: profile.id,
            file_rand,
        })
    }

    pub fn as_bytes(&self) -> [u8; HEADER_LEN] {
        let mut b = [0u8; HEADER_LEN];
        b[0] = self.fmt_ver;
        b[1] = self.kdf_profile;
        b[2..].copy_from_slice(&self.file_rand);
        b
    }

    pub fn parse(b: &[u8]) -> Result<Self, VaultError> {
        if b.len() < HEADER_LEN {
            return Err(VaultError::NotANekoVault);
        }
        if b[0] != FMT_V1 {
            return Err(VaultError::FutureFormat(b[0]));
        }
        if profile::by_id(b[1]).is_none() {
            return Err(VaultError::UnknownKdfProfile(b[1]));
        }
        let mut file_rand = [0u8; 14];
        file_rand.copy_from_slice(&b[2..HEADER_LEN]);
        Ok(Self {
            fmt_ver: b[0],
            kdf_profile: b[1],
            file_rand,
        })
    }

    pub fn profile(&self) -> Result<Profile, VaultError> {
        profile::by_id(self.kdf_profile).ok_or(VaultError::UnknownKdfProfile(self.kdf_profile))
    }

    pub fn to_hex(self) -> String {
        hex::encode(self.as_bytes())
    }
}
