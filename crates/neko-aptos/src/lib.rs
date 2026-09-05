//! Aptos: an account chain that keeps coins and fungible assets apart.
//!
//! Two things here are unlike the other account chains this wallet knows.
//!
//! The address is **not** the public key, and not a hash of it alone: it is
//! `sha3_256(public_key || scheme)`, so the same key under a different signing
//! scheme is a different account. That is checked before signing.
//!
//! And the chain has two token systems. `Coin` is the original one and APT
//! still answers to it; **fungible assets** are the newer one, and Tether's
//! USDT here is one of those. They have different entry functions and
//! different balance views, and using one against the other does not move a
//! wrong amount - the transaction aborts.

pub mod address;
pub mod bcs;
pub mod client;
pub mod error;
pub mod history;
pub mod tx;

pub use address::{AptosAddress, ADDRESS_BYTES};
pub use error::AptosError;

/// APT's precision. Read from the chain: `0x1::coin::decimals` answers 8.
pub const APT_DECIMALS: u8 = 8;

/// USDT's precision here. Six, like Ethereum's and unlike BNB Chain's.
pub const USDT_DECIMALS: u8 = 6;

/// Tether's fungible-asset metadata object on Aptos.
///
/// Not a coin type and not an ERC-20-style contract: the object whose address
/// identifies the asset. This is the address Binance itself lists for USDT
/// withdrawals to Aptos.
pub const USDT_METADATA: &str =
    "0x357b0b74bc833e95a115ad22604854d6b0fca151cecd94111770e5d6ffc9dc2b";

/// What that contract calls itself, checked against the chain before a
/// transfer is signed.
///
/// `USDt`, with a lowercase t - the same spelling Avalanche uses and not the
/// one on the tin.
pub const USDT_SYMBOL: &str = "USDt";

/// APT's own fungible-asset metadata, for the balance view.
pub const APT_METADATA: &str = "0xa";

pub const DEFAULT_API: &str = "https://fullnode.mainnet.aptoslabs.com/v1";
pub const EXPLORER_TX: &str = "https://explorer.aptoslabs.com/txn/";

/// Mainnet. Signed over, so this is what keeps a mainnet transaction from
/// being valid on a testnet and the other way round.
pub const CHAIN_ID: u8 = 1;

/// Gas units to allow for a transfer.
///
/// Aptos charges gas units times a price, and the *units* are what this
/// bounds. Both transfers here cost far less; the ceiling exists so a
/// transaction cannot run away, and unused units are not charged.
pub const MAX_GAS_TRANSFER: u64 = 2_000;

/// How long a transaction stays valid. Aptos expires by wall clock rather than
/// by block height, so this is seconds rather than blocks.
pub const EXPIRY_SECS: u64 = 600;

pub fn usdt_metadata() -> AptosAddress {
    AptosAddress::parse(USDT_METADATA).expect("a constant address in this file is malformed")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two numbers that decide what an amount means.
    #[test]
    fn the_precisions_are_the_ones_the_chain_reports() {
        assert_eq!(APT_DECIMALS, 8, "0x1::coin::decimals answers 8");
        assert_eq!(USDT_DECIMALS, 6);
    }

    /// The metadata address is well-formed and is not the framework.
    #[test]
    fn the_usdt_object_is_a_real_address() {
        let m = usdt_metadata();
        assert_eq!(m.to_string(), USDT_METADATA);
        assert_ne!(m, tx::framework());
    }
}
