//! SQLCipher-backed encrypted storage.
//!
//! This crate **never derives a key**. Every entry point takes key material as a
//! parameter. That breaks the otherwise-circular dependency with `neko-vault`
//! (which needs to read the vault row) and lets the storage layer be tested
//! with a fixed key, without paying for Argon2.

pub mod codec;
pub mod error;
pub mod migrate;
pub mod open;
pub mod repo;
pub mod vault_row;

pub use error::StoreError;
pub use open::{assert_no_wal_sidecars, create, read_header, rekey};
pub use vault_row::{VaultRow, BLOB_VERSION, CURRENT_SCHEMA};
