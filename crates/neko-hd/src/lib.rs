//! BIP39 / BIP32 / BIP44 derivation and TRON address encoding.
//!
//! Chain-agnostic enough that adding Bitcoin later means a new coin type and a
//! new address encoder, not a rewrite. Verified byte-for-byte against
//! `vectors/hd.json`, which includes the official Ledger test vector.

pub mod address;
pub mod derive;
pub mod error;

pub use address::{Address, ADDRESS_LEN};
pub use derive::{
    address_at, address_from_private_key, entropy_from_mnemonic, generate_mnemonic,
    mnemonic_from_entropy, private_key_at, seed_from_mnemonic, validate_mnemonic, COIN_TYPE,
};
pub use error::HdError;
