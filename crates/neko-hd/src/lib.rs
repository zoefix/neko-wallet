//! BIP39 / BIP32 / BIP44 derivation, and the address encodings that sit on it.
//!
//! One mnemonic, many chains: the coin type in the derivation path is what
//! separates them, so a wallet needs no second phrase and no second backup to
//! gain a chain. TRON is coin type 195 and encodes base58check with a 0x41
//! prefix; EVM chains are coin type 60 and print hex with an EIP-55 checksum.
//! The 20 bytes underneath are computed identically for both, which is why the
//! encoders live together here rather than in the chain crates.
//!
//! Solana is the exception to all of that: it signs with Ed25519, derives by
//! SLIP-0010 rather than BIP32, and its address is the bare public key. See
//! `solana` - none of the machinery above applies to it.
//!
//! Verified byte-for-byte against `vectors/hd.json`, which includes the
//! official Ledger test vector.

pub mod address;
pub mod bech32;
pub mod bitcoin;
pub mod derive;
pub mod error;
pub mod evm;
pub mod solana;

pub use address::{Address, ADDRESS_LEN};
pub use bitcoin::{BtcAddress, COIN_TYPE_BTC};
pub use derive::{
    address_at, address_from_private_key, entropy_from_mnemonic, evm_address_at,
    evm_address_from_private_key, evm_private_key_at, generate_mnemonic, mnemonic_from_entropy,
    private_key_at, seed_from_mnemonic, validate_mnemonic, COIN_TYPE, COIN_TYPE_EVM,
};
pub use error::HdError;
pub use evm::EvmAddress;
pub use solana::{SolanaAddress, COIN_TYPE_SOLANA};
