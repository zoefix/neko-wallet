//! Sui: a chain where a balance is a set of objects.
//!
//! Nothing here has an account with a number in it. Coins are *objects*, each
//! with an id, a version and a digest, each owned by an address; a balance is
//! their sum and a transfer spends particular ones. That makes this the second
//! chain in this wallet - after Bitcoin - where sending needs a selection
//! rather than a subtraction, and it makes a stale version a rejection rather
//! than a smaller payment.
//!
//! Transactions are *programmable blocks*: a list of inputs and a list of
//! commands that refer to them and to each other's results. There is no
//! "transfer" instruction; a payment is a split followed by a hand-over.

pub mod address;
pub mod bcs;
pub mod client;
pub mod error;
pub mod history;
pub mod tx;

pub use address::{SuiAddress, ADDRESS_BYTES};
pub use error::SuiError;

/// SUI's precision. Nine, not eighteen and not six.
pub const SUI_DECIMALS: u8 = 9;

/// Circle's USDC on Sui, and its precision - both read from the chain's own
/// `suix_getCoinMetadata`.
///
/// A *coin type*, not an address: on this chain a token is named by the Move
/// type its objects hold.
pub const USDC_TYPE: &str =
    "0xdba34672e30cb065b1f93e3ab55318768fd6fef66c15942c9f7cb846e2f900e7::usdc::USDC";
pub const USDC_DECIMALS: u8 = 6;
pub const USDC_SYMBOL: &str = "USDC";

/// SUI's own coin type, which is how the node is asked for gas coins.
pub const SUI_TYPE: &str = "0x2::sui::SUI";

pub const DEFAULT_API: &str = "https://sui-rpc.publicnode.com";
pub const EXPLORER_TX: &str = "https://suiscan.xyz/mainnet/tx/";

/// The gas ceiling for a transfer, in MIST.
///
/// Sui charges computation plus storage and refunds the unused part, but the
/// whole budget has to be available in the gas coins before it will run. A
/// transfer's real cost is a small fraction of this.
pub const GAS_BUDGET_TRANSFER: u64 = 10_000_000;

/// How many coin objects a transfer will fold together.
///
/// A balance spread across more objects than this cannot be sent in one
/// transaction, which is said plainly rather than discovered at the node.
pub const MAX_COINS_PER_TRANSFER: usize = 32;

/// How long a serialized signature is: a flag byte, 64 bytes of Ed25519, and
/// the 32-byte public key.
///
/// Sui wants the signature *beside* the transaction rather than inside it, so
/// this wallet carries the two concatenated through the one `raw` field every
/// chain shares, and splits them again at broadcast. The length is fixed,
/// which is what makes that safe.
pub const SIGNATURE_BYTES: usize = 1 + 64 + 32;

/// BLAKE2b-256, which is Sui's hash everywhere Aptos uses SHA3.
pub fn blake2b256(data: &[u8]) -> [u8; 32] {
    use blake2::digest::consts::U32;
    use blake2::{Blake2b, Digest};
    let mut h = Blake2b::<U32>::new();
    h.update(data);
    let mut out = [0u8; 32];
    out.copy_from_slice(&h.finalize());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BLAKE2b-256 against a published vector, because every address and every
    /// signature on this chain depends on it.
    #[test]
    fn blake2b256_matches_the_reference_vector() {
        assert_eq!(
            hex::encode(blake2b256(b"")),
            "0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8"
        );
        assert_eq!(
            hex::encode(blake2b256(b"abc")),
            "bddd813c634239723171ef3fee98579b94964e3bb1cb3e427262c8c068d52319"
        );
    }

    #[test]
    fn the_precisions_are_the_ones_the_chain_reports() {
        assert_eq!(SUI_DECIMALS, 9);
        assert_eq!(USDC_DECIMALS, 6, "suix_getCoinMetadata answers 6");
    }
}
