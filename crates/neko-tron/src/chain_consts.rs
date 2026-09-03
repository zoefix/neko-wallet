//! TRON mainnet constants.
//!
//! There is exactly one network. No testnet, no switching: a wallet that can
//! silently be pointed at a different chain is a wallet that can silently show
//! the wrong balances and send to the wrong USDT contract.
//!
//! A custom node URL is still configurable, because that is about *which
//! server* speaks for mainnet, not about which chain you are on. The on-chain
//! `symbol()` / `decimals()` check before every token transfer is what actually
//! guards against a node that lies.

use neko_hd::Address;

pub const DEFAULT_URL: &str = "https://api.trongrid.io";
pub const EXPLORER_TX: &str = "https://tronscan.org/#/transaction/";
pub const USDT_CONTRACT: &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";

pub fn usdt_address() -> Address {
    Address::parse(USDT_CONTRACT).expect("built-in USDT address must parse")
}

/// SunSwap V2 router, used only to quote a price - never to trade.
pub const SUNSWAP_ROUTER: &str = "TKzxdSv2FZKQrEqkKVgp5DcwEXBEKMg2Ax";
/// Wrapped TRX, the pair's other side.
pub const WTRX: &str = "TNUC9Qb1rRpS5CbWLmNMxXBjyFoydXjWFR";

#[cfg(test)]
mod tests {
    use super::*;

    /// A wrong router quotes a wrong price, and a wrong WTRX quotes a pair
    /// that does not exist. Both must at least be well-formed addresses.
    #[test]
    fn the_price_pair_addresses_parse() {
        assert_eq!(
            Address::parse(SUNSWAP_ROUTER).unwrap().to_string(),
            SUNSWAP_ROUTER
        );
        assert_eq!(Address::parse(WTRX).unwrap().to_string(), WTRX);
    }

    #[test]
    fn usdt_address_round_trips() {
        let a = usdt_address();
        assert_eq!(a.to_string(), USDT_CONTRACT);
        assert_eq!(a.to_hex(), "41a614f803b6fd780986a42c78ec9c7f77e6ded13c");
    }
}
