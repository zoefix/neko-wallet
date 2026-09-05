//! BNB Chain, and the EVM machinery it shares with every other EVM chain.
//!
//! The split from `neko-tron` is deliberate: nothing here knows about TRON's
//! protobuf, energy or bandwidth, and nothing there knows about RLP or gas.
//! What the two have in common - key derivation, the 20-byte address, storage,
//! encryption, the interface - lives in the crates below both.

pub mod abi;
pub mod blockscout;
pub mod chain_consts;
pub mod client;
pub mod error;
pub mod etherscan;
pub mod history;
pub mod price;
pub mod rlp;
pub mod tx;

pub use chain_consts::*;
pub use error::EvmError;
