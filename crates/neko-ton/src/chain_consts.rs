//! TON constants.

use crate::address::TonAddress;

/// Nanotons: 1 GRAM = 1e9. Nine, where BTC is 8 and SOL is 9 and BNB is 18 -
/// the number travels with the asset for this reason.
pub const GRAM_DECIMALS: u8 = 9;

/// Tether on TON. **Six decimals**, like Ethereum's and TRON's, unlike BNB
/// Chain's eighteen. Confirmed against the jetton master itself.
pub const USDT_MASTER: &str = "EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs";
pub const USDT_DECIMALS: u8 = 6;

/// Configurable, like every other chain's. TON's public endpoint rate-limits
/// hard, and whoever answers sees which addresses are being asked about.
pub const DEFAULT_API: &str = "https://toncenter.com/api/v2";

pub const EXPLORER_TX: &str = "https://tonviewer.com/transaction/";

/// What a plain transfer costs, in nanotons.
///
/// Not a quote - TON's fees are small, fixed in shape and paid out of the
/// message's own value, so this is an upper bound used to check a balance
/// covers a transfer. The chain charges what it charges and refunds nothing;
/// the figure is small enough that being generous costs a fraction of a cent.
pub const FEE_TRANSFER: u128 = 10_000_000; // 0.01 GRAM

/// What has to travel *with* a jetton transfer.
///
/// A token transfer here is a message to your own jetton wallet contract, which
/// then messages the recipient's - and each hop costs gas that has to come from
/// somewhere. That somewhere is coin attached to the message, and whatever is
/// not used comes back. This is the single most surprising thing about sending
/// tokens on this chain: it needs GRAM even though it moves USDT.
pub const JETTON_TRANSFER_ATTACHED: u128 = 50_000_000; // 0.05 GRAM
/// Of that, what is forwarded on to the recipient's jetton wallet.
pub const JETTON_FORWARD_AMOUNT: u128 = 1; // enough to trigger a notification

pub fn usdt_master() -> TonAddress {
    TonAddress::parse(USDT_MASTER).expect("the USDT master constant is malformed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_usdt_master_round_trips() {
        assert_eq!(usdt_master().to_friendly_string(), USDT_MASTER);
        assert_eq!(
            usdt_master().to_raw_string(),
            "0:b113a994b5024a16719f69139328eb759596c38a25f59028b146fecdc3621dfe"
        );
    }

    /// Six on TON, six on TRON, six on Ethereum, six on Solana, eighteen on
    /// BNB Chain. One token name, and the one that differs is the one that
    /// costs a factor of a million million.
    #[test]
    fn usdt_has_six_decimals_here() {
        assert_eq!(USDT_DECIMALS, 6);
        assert_eq!(GRAM_DECIMALS, 9);
    }
}
