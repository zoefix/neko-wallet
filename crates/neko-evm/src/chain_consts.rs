//! The EVM chains this wallet knows, and what differs between them.
//!
//! Everything below the chain - RLP, the ABI encoding, the address, the
//! JSON-RPC - is identical, which is why one crate serves both. What is *not*
//! identical is exactly the list in [`EvmChain`], and getting any of it wrong
//! is a specific way to lose money:
//!
//! * The **chain id** is what makes a signature valid on one chain and useless
//!   on the others. Signing with the wrong one produces a transaction that is
//!   replayable where the same address holds different funds.
//! * The **USDT contract** differs, and so does its **precision**: six decimals
//!   on Ethereum, eighteen on BNB Chain. Same token name, same wallet, a factor
//!   of a million million between them.
//! * The **transaction format** differs, and that is a fee question rather than
//!   a correctness one - see [`TxType`].

use neko_hd::EvmAddress;

/// Which transaction format a chain gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxType {
    /// Type 0, with an EIP-155 signature. One gas price, no ceiling.
    Legacy,
    /// Type 2, EIP-1559. A ceiling and a tip, of which only the base fee plus
    /// the tip is actually charged - the rest is headroom that is refunded.
    Eip1559,
}

/// One EVM chain's parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvmChain {
    pub chain_id: u64,
    pub native_symbol: &'static str,
    pub native_decimals: u8,
    pub usdt: &'static str,
    /// **Read this rather than assuming.** Six on Ethereum, eighteen on BNB
    /// Chain.
    pub usdt_decimals: u8,
    pub default_rpc: &'static str,
    pub explorer_tx: &'static str,
    /// A Uniswap-V2-compatible router, used only to *quote* a price. This
    /// wallet never trades.
    pub router: &'static str,
    /// The wrapped native coin, which is the pair's other side.
    pub wrapped_native: &'static str,
    pub tx_type: TxType,
    /// NodeReal's host for this chain's transfer index, used only when the user
    /// supplies a key.
    pub history_host: &'static str,
}

/// BNB Chain.
///
/// Legacy transactions, deliberately. BNB Chain accepts both formats, gas there
/// is cheap and stable enough that a priority fee buys nothing, and type 0 is
/// the format every node and explorer has understood since the beginning.
pub const BSC: EvmChain = EvmChain {
    chain_id: 56,
    native_symbol: "BNB",
    native_decimals: 18,
    // **18 decimals, not 6.** The same token on Ethereum and TRON has 6.
    usdt: "0x55d398326f99059fF775485246999027B3197955",
    usdt_decimals: 18,
    default_rpc: "https://bsc-dataseed.bnbchain.org",
    explorer_tx: "https://bscscan.com/tx/",
    // PancakeSwap V2, a Uniswap V2 fork - same `getAmountsOut`.
    router: "0x10ED43C718714eb63d5aA57B78B54704E256024E",
    wrapped_native: "0xbb4CdB9CBd36B01bD1cBaEBF2De08d9173bc095c",
    tx_type: TxType::Legacy,
    history_host: "https://bsc-mainnet.nodereal.io/v1",
};

/// Ethereum.
///
/// EIP-1559 transactions, and here the choice matters. Ethereum's base fee
/// moves between blocks, and a legacy transaction commits to one price: too low
/// and it simply never confirms, too high and the excess is kept. Type 2 names
/// a ceiling and a tip, pays only the base fee plus the tip, and refunds the
/// rest - so headroom for a rising base fee costs nothing.
pub const ETHEREUM: EvmChain = EvmChain {
    chain_id: 1,
    native_symbol: "ETH",
    native_decimals: 18,
    // Tether's original contract. **6 decimals**, confirmed against the chain.
    usdt: "0xdAC17F958D2ee523a2206206994597C13D831ec7",
    usdt_decimals: 6,
    default_rpc: "https://ethereum-rpc.publicnode.com",
    explorer_tx: "https://etherscan.io/tx/",
    router: "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D",
    wrapped_native: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
    tx_type: TxType::Eip1559,
    history_host: "https://eth-mainnet.nodereal.io/v1",
};

impl EvmChain {
    pub fn usdt_address(&self) -> EvmAddress {
        parse_const(self.usdt)
    }
    pub fn router_address(&self) -> EvmAddress {
        parse_const(self.router)
    }
    pub fn wrapped_native_address(&self) -> EvmAddress {
        parse_const(self.wrapped_native)
    }
}

fn parse_const(s: &str) -> EvmAddress {
    EvmAddress::parse(s).expect("a constant address in this file is malformed")
}

/// Plain native-coin transfer. Fixed by the protocol on every EVM chain.
pub const GAS_TRANSFER: u64 = 21_000;

/// BTCB, Binance-pegged Bitcoin, and its precision. Lives on BNB Chain and is
/// how BTC is priced - Bitcoin has no exchange on its own chain.
pub const BTCB: &str = "0x7130d2A12B9BCbFAe4f2634d864A1Ee1Ce3Ead9c";
pub const BTCB_DECIMALS: u8 = 18;

#[cfg(test)]
mod tests {
    use super::*;

    /// A mistyped contract address sends funds somewhere unrecoverable, or
    /// quotes a price from something else entirely. Round-tripping through
    /// EIP-55 proves each is well-formed and that no character was transposed.
    #[test]
    fn every_constant_address_is_checksummed() {
        for c in [BSC, ETHEREUM] {
            for s in [c.usdt, c.router, c.wrapped_native] {
                assert_eq!(
                    EvmAddress::parse(s).unwrap().to_string(),
                    s,
                    "on chain {}",
                    c.chain_id
                );
            }
        }
        assert_eq!(EvmAddress::parse(BTCB).unwrap().to_string(), BTCB);
    }

    /// The two numbers that decide whether a signature is valid and whether an
    /// amount is right. Pinned because a wrong chain id is a replay hazard and
    /// a wrong precision is a factor of a million million.
    #[test]
    fn the_chains_are_told_apart_by_the_two_things_that_matter() {
        assert_eq!(BSC.chain_id, 56);
        assert_eq!(ETHEREUM.chain_id, 1);
        assert_ne!(BSC.chain_id, ETHEREUM.chain_id);

        assert_eq!(BSC.usdt_decimals, 18);
        assert_eq!(
            ETHEREUM.usdt_decimals, 6,
            "read from the chain, not assumed"
        );
        assert_ne!(BSC.usdt, ETHEREUM.usdt, "different contracts entirely");
    }

    /// Ethereum's base fee moves and Ethereum's gas is the expensive kind;
    /// BNB Chain's does not and is not.
    #[test]
    fn the_transaction_format_follows_the_fee_market() {
        assert_eq!(ETHEREUM.tx_type, TxType::Eip1559);
        assert_eq!(BSC.tx_type, TxType::Legacy);
    }
}
