//! Bitcoin.
//!
//! The one chain here that is not account-based, and every difference follows
//! from that. There is no balance to read and no nonce to increment: a wallet
//! holds unspent outputs, spending means naming particular ones, and the fee is
//! whatever is left over rather than a field anybody sets. A slip in that
//! arithmetic does not fail - it pays a miner the difference.
//!
//! Addresses are derived at `m/84'/0'/0'/0/{i}` and are always P2WPKH, which is
//! what `bc1q...` means. Payments go to any of five script types, because
//! refusing to pay somebody's older address would make the wallet useless for
//! paying them.

pub mod chain_consts;
pub mod client;
pub mod coins;
pub mod error;
pub mod history;
pub mod tx;
pub mod varint;

pub use chain_consts::*;
pub use error::BtcError;
