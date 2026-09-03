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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usdt_address_round_trips() {
        let a = usdt_address();
        assert_eq!(a.to_string(), USDT_CONTRACT);
        assert_eq!(a.to_hex(), "41a614f803b6fd780986a42c78ec9c7f77e6ded13c");
    }
}
