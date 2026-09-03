//! The neko-wallet key hierarchy.
//!
//! ```text
//! email (NFKC, lowercased) + password (NFKC, whitespace-folded, case-sensitive)
//!    │
//!    │  header16 = [fmt_ver][kdf_profile][14 random]   ← plaintext, bytes 0..16 of the .db
//!    ▼  argon_salt = SHA-512("neko/argon2-salt-v1" ‖ u32be(len(email)) ‖ email ‖ header16)[..32]
//!    ▼  Argon2id(m,t,p from PROFILES[kdf_profile])
//! stretched (32 B)
//!    ├─ HKDF "neko/file-key-v1" ─► k_file ─► SQLCipher raw key   (whole-file layer)
//!    └─ HKDF "neko/kek-v1"      ─► KEK    ─► unwrap vault.wrapped_mk
//!                                              │
//!                                              ▼
//!                                          MK (32 B, random — NOT a mnemonic)
//!    ┌──────────────────┬──────────────────┬───────────────────┐
//!    ▼                  ▼                  ▼                   ▼
//! k_data            k_blind            verifier         per-wallet material
//! ```
//!
//! `k_file` comes from `stretched`, not from MK, because MK lives *inside* the
//! encrypted database — anything needed to open the file must be derivable from
//! the password alone. That split is also what makes the two layers genuinely
//! independent: an attacker who obtains `k_file` (memory dump, an unlocked and
//! abandoned laptop) still cannot read `wallets.entropy_ct`, which needs MK.

use neko_crypto::{kdf, kdf_hkdf, Aad, Argon2idParams};
use sha2::{Digest, Sha512};
use zeroize::{Zeroize, Zeroizing};

use crate::error::VaultError;
use crate::header::FileHeader;
use crate::profile::Profile;

pub const ARGON_SALT_LABEL: &[u8] = b"neko/argon2-salt-v1";

macro_rules! secret32 {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Zeroize)]
        #[zeroize(drop)]
        pub struct $name(pub(crate) [u8; 32]);

        impl $name {
            pub fn as_bytes(&self) -> &[u8; 32] {
                &self.0
            }
            pub fn to_hex(&self) -> Zeroizing<String> {
                Zeroizing::new(hex::encode(self.0))
            }
            pub(crate) fn from_slice(s: &[u8]) -> Self {
                let mut b = [0u8; 32];
                b.copy_from_slice(s);
                Self(b)
            }
        }

        // Redacted at the type level: no `dbg!` or `{:?}` can ever leak this.
        impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(concat!(stringify!($name), "([redacted])"))
            }
        }
    };
}

secret32!(
    Stretched,
    "Argon2id output. The root of everything derivable from the password."
);
secret32!(FileKey, "SQLCipher raw key for the whole-file layer.");
secret32!(Kek, "Key-encryption key that wraps MK.");
secret32!(
    Mk,
    "Master key. 32 random bytes; deliberately NOT a BIP39 mnemonic."
);
secret32!(DataKey, "Field-level AEAD key.");
secret32!(BlindKey, "Blind-index HMAC key.");

/// `SHA-512(label ‖ u32be(len(email)) ‖ email ‖ header16)[..32]`
///
/// The email is length-prefixed for the same reason the AAD fields are: without
/// it, distinct (email, header) pairs could collide.
pub fn argon_salt(email_norm: &str, header: &FileHeader) -> [u8; 32] {
    let mut h = Sha512::new();
    h.update(ARGON_SALT_LABEL);
    h.update((email_norm.len() as u32).to_be_bytes());
    h.update(email_norm.as_bytes());
    h.update(header.as_bytes());
    let d = h.finalize();
    let mut out = [0u8; 32];
    out.copy_from_slice(&d[..32]);
    out
}

/// Run the (expensive) password stretch. CPU-bound: callers on an async runtime
/// must dispatch this via `spawn_blocking`.
pub fn stretch(
    email_norm: &str,
    password_norm: &str,
    header: &FileHeader,
) -> Result<Stretched, VaultError> {
    let profile = header.profile()?;
    let salt = argon_salt(email_norm, header);
    let out = kdf::derive_key(password_norm.as_bytes(), &salt, profile.params)?;
    Ok(Stretched::from_slice(&out))
}

pub fn file_key(s: &Stretched) -> Result<FileKey, VaultError> {
    Ok(FileKey::from_slice(&kdf_hkdf::derive_key32(
        s.as_bytes(),
        kdf_hkdf::info::FILE_KEY,
    )?))
}

/// KEK is salted with the per-database random salt from the vault row. HKDF is
/// essentially free, so this hardens the MK wrapping without doubling login time.
pub fn kek(s: &Stretched, vault_salt: &[u8]) -> Result<Kek, VaultError> {
    Ok(Kek::from_slice(&kdf_hkdf::derive_with_salt(
        s.as_bytes(),
        vault_salt,
        kdf_hkdf::info::KEK,
        32,
    )?))
}

pub fn data_key(mk: &Mk) -> Result<DataKey, VaultError> {
    Ok(DataKey::from_slice(&kdf_hkdf::derive_key32(
        mk.as_bytes(),
        kdf_hkdf::info::DATA_KEY,
    )?))
}

pub fn blind_key(mk: &Mk) -> Result<BlindKey, VaultError> {
    Ok(BlindKey::from_slice(&kdf_hkdf::derive_key32(
        mk.as_bytes(),
        kdf_hkdf::info::BLIND_INDEX,
    )?))
}

pub fn verifier(mk: &Mk) -> Result<[u8; 32], VaultError> {
    let v = kdf_hkdf::derive_key32(mk.as_bytes(), kdf_hkdf::info::VERIFIER)?;
    let mut out = [0u8; 32];
    out.copy_from_slice(&v);
    Ok(out)
}

/// Per-wallet key material, namespaced by a monotonic sequence number.
pub fn wallet_material(mk: &Mk, seq: u32, len: usize) -> Result<Zeroizing<Vec<u8>>, VaultError> {
    let mut info = kdf_hkdf::info::WALLET_SEED.as_bytes().to_vec();
    info.extend_from_slice(&seq.to_be_bytes());
    let s = String::from_utf8_lossy(&info).into_owned();
    Ok(kdf_hkdf::derive_from_ikm(mk.as_bytes(), &s, len)?)
}

pub fn new_mk() -> Result<Mk, VaultError> {
    Ok(Mk::from_slice(&neko_crypto::random(32)?))
}

/// AAD binding for the wrapped master key. `extra` folds in the KDF parameter
/// encoding and the vault salt, so tampering with either breaks decryption
/// outright instead of quietly weakening it.
pub fn vault_aad_extra(params: Argon2idParams, vault_salt: &[u8]) -> Vec<u8> {
    let mut extra = params.encode().to_vec();
    extra.extend_from_slice(vault_salt);
    extra
}

pub fn wrap_mk(kek: &Kek, mk: &Mk, key_ver: u32, extra: &[u8]) -> Result<Vec<u8>, VaultError> {
    let aad = Aad {
        table: "vault",
        column: "wrapped_mk",
        row_id: 1,
        key_ver,
        extra,
    };
    Ok(neko_crypto::seal(kek.as_bytes(), mk.as_bytes(), aad)?)
}

pub fn unwrap_mk(kek: &Kek, wrapped: &[u8], key_ver: u32, extra: &[u8]) -> Result<Mk, VaultError> {
    let aad = Aad {
        table: "vault",
        column: "wrapped_mk",
        row_id: 1,
        key_ver,
        extra,
    };
    let pt = neko_crypto::open(kek.as_bytes(), wrapped, aad)
        .map_err(|_| VaultError::VaultBlobCorrupt)?;
    if pt.len() != 32 {
        return Err(VaultError::VaultBlobCorrupt);
    }
    Ok(Mk::from_slice(&pt))
}

/// Profile chosen at creation; also re-validated against the encrypted record.
pub fn assert_profile_matches(
    header: &FileHeader,
    recorded: Argon2idParams,
) -> Result<Profile, VaultError> {
    let p = header.profile()?;
    if p.params != recorded {
        return Err(VaultError::KdfProfileMismatch);
    }
    Ok(p)
}
