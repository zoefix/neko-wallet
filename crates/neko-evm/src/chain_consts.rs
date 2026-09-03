//! BNB Chain constants, each verified against the chain itself.
//!
//! A wrong contract address here sends funds somewhere unrecoverable, so the
//! wallet also asks the chain for `symbol()` and `decimals()` before any
//! transfer rather than trusting these alone.

/// EIP-155 chain id. Signing without it would produce a transaction replayable
/// on every other EVM chain.
pub const CHAIN_ID: u64 = 56;

/// BNB, the native coin.
pub const BNB_DECIMALS: u8 = 18;

/// USDT (BEP-20).
///
/// **18 decimals, not 6.** The same token on TRON has 6, and treating one like
/// the other is a factor of a million million. This is why decimals travel
/// with the asset everywhere in this program instead of being a constant.
pub const USDT_CONTRACT: &str = "0x55d398326f99059fF775485246999027B3197955";
pub const USDT_DECIMALS: u8 = 18;

/// Default public endpoint. Configurable, like TronGrid's.
pub const DEFAULT_RPC: &str = "https://bsc-dataseed.bnbchain.org";

/// Plain BNB transfer. Fixed by the protocol.
pub const GAS_TRANSFER: u64 = 21_000;

pub fn usdt_address() -> neko_hd::EvmAddress {
    // Parsed rather than stored as bytes so the constant above is the single
    // source of truth, and so a typo fails the test below rather than being
    // silently re-encoded into a different address.
    neko_hd::EvmAddress::parse(USDT_CONTRACT).expect("USDT contract address is malformed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_usdt_address_is_well_formed_and_checksummed() {
        let a = usdt_address();
        // Round-tripping through EIP-55 must reproduce the constant exactly,
        // which also proves the constant carries a valid checksum.
        assert_eq!(a.to_string(), USDT_CONTRACT);
    }

    /// Chain id 56 is what makes a signature valid on BNB Chain and useless
    /// anywhere else. Pinned because a wrong value here is a replay hazard,
    /// not merely a rejected transaction.
    #[test]
    fn chain_id_is_bnb_chain() {
        assert_eq!(CHAIN_ID, 56);
    }
}
