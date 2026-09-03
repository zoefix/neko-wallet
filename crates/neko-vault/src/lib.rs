//! Key hierarchy, KDF profiles, credential policy, and lock state.
//!
//! This crate owns every decision about *how* keys come into existence.
//! `neko-store` never derives a key; it takes them as parameters.

pub mod calibrate;
pub mod error;
pub mod header;
pub mod keys;
pub mod normalize;
pub mod password;
pub mod profile;

pub use calibrate::Calibration;
pub use error::VaultError;
pub use header::{FileHeader, HEADER_LEN};
pub use keys::{DataKey, FileKey, Kek, Mk, Stretched};
pub use profile::Profile;
