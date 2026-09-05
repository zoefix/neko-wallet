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
//!   of a million million between them. On Polygon it does not even have the
//!   same name - the contract calls itself `USDT0`.
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
    pub stable: &'static str,
    /// **Read this rather than assuming.** Six on Ethereum, eighteen on BNB
    /// Chain.
    pub stable_decimals: u8,
    /// What this wallet calls it on screen.
    ///
    /// Not always `"USDT"`. Base's stablecoin is USDC: Tether's contract there
    /// holds 23 million against USDC's 4.2 billion, and Binance will not send
    /// USDT to that chain at all - so a USDT row on Base is one nobody can
    /// ever fill.
    pub stable_label: &'static str,
    /// What that contract calls itself, checked against the chain before a
    /// transfer is signed.
    ///
    /// Not the same thing as [`Self::stable_label`], and not always equal to
    /// it: Tether's Polygon contract reports `USDT0` since the omnichain
    /// migration, and this wallet still shows that holding as USDT. This field
    /// answers "is this the right contract"; the label answers "what do we
    /// call it".
    pub stable_symbol: &'static str,
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
    ///
    /// `None` where there is none. A node's RPC cannot answer "what has this
    /// address done"; that needs an index, and NodeReal does not serve one for
    /// every chain here. Saying so is better than naming a host that does not
    /// resolve and reporting it as the network being down.
    pub history_host: Option<&'static str>,
    /// Where this chain's coin is priced, when it cannot be priced here.
    ///
    /// The chain id of another chain, or `None` to use this one's own pool.
    /// Base is the reason: its coin is ETH, and its Uniswap-V2 WETH/USDT pool
    /// holds about seventeen dollars in total, so `getAmountsOut` there
    /// answers with a number that is not a price. The same coin has a deep
    /// pool on Ethereum, which this wallet already talks to - the same trade
    /// BTC already makes, quoted from BTCB on a chain that has an exchange.
    pub prices_on: Option<u64>,
    /// A Blockscout instance for this chain, which indexes both coin and token
    /// movements and asks for no key.
    ///
    /// Set where there is no alternative. Polygon has no NodeReal endpoint, so
    /// without this its history screen could only say "unavailable" - and a
    /// keyless index is the same trade Bitcoin already makes with Esplora.
    pub blockscout: Option<&'static str>,
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
    stable: "0x55d398326f99059fF775485246999027B3197955",
    stable_decimals: 18,
    stable_symbol: "USDT",
    stable_label: "USDT",
    default_rpc: "https://bsc-dataseed.bnbchain.org",
    explorer_tx: "https://bscscan.com/tx/",
    // PancakeSwap V2, a Uniswap V2 fork - same `getAmountsOut`.
    router: "0x10ED43C718714eb63d5aA57B78B54704E256024E",
    wrapped_native: "0xbb4CdB9CBd36B01bD1cBaEBF2De08d9173bc095c",
    tx_type: TxType::Legacy,
    history_host: Some("https://bsc-mainnet.nodereal.io/v1"),
    prices_on: None,
    blockscout: None,
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
    stable: "0xdAC17F958D2ee523a2206206994597C13D831ec7",
    stable_decimals: 6,
    stable_symbol: "USDT",
    stable_label: "USDT",
    default_rpc: "https://ethereum-rpc.publicnode.com",
    explorer_tx: "https://etherscan.io/tx/",
    router: "0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D",
    wrapped_native: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
    tx_type: TxType::Eip1559,
    history_host: Some("https://eth-mainnet.nodereal.io/v1"),
    prices_on: None,
    blockscout: None,
};

/// Polygon.
///
/// The third EVM chain, and it needed no new machinery - which is the point of
/// the crate being parameterised rather than forked. Three things are its own:
///
/// * The coin is **POL**, not MATIC. It was renamed in September 2024, and the
///   chain says so itself: the wrapped native contract below reports `WPOL`.
/// * Its USDT contract calls itself **`USDT0`**. Tether migrated Polygon's
///   supply to its omnichain deployment, and the name went with it. The token
///   is the same one people mean by USDT and this wallet shows it as USDT; what
///   is checked against the chain is the name the contract actually has.
/// * **No transfer index.** NodeReal serves BNB Chain and Ethereum, not this
///   one, so history says it is unavailable rather than failing at a host that
///   does not exist. Balances, fees, prices and transfers are unaffected.
pub const POLYGON: EvmChain = EvmChain {
    chain_id: 137,
    native_symbol: "POL",
    native_decimals: 18,
    // Six decimals, like Ethereum's and unlike BNB Chain's.
    stable: "0xc2132D05D31c914a87C6611C10748AEb04B58e8F",
    stable_decimals: 6,
    stable_symbol: "USDT0",
    // Shown as USDT: it is the token everyone means by that name.
    stable_label: "USDT",
    // polygon-rpc.com answers 401 without a key; this one does not.
    default_rpc: "https://polygon-bor-rpc.publicnode.com",
    explorer_tx: "https://polygonscan.com/tx/",
    // QuickSwap V2, a Uniswap V2 fork - same `getAmountsOut`. Cross-checked
    // against SushiSwap's router, which agreed to within half a percent.
    router: "0xa5E0829CaCEd8fFDD4De3c43696c57F7D7A678ff",
    wrapped_native: "0x0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270",
    // The base fee moves here as it does on Ethereum, and it moves a long way:
    // hundreds of gwei is ordinary.
    tx_type: TxType::Eip1559,
    history_host: None,
    prices_on: None,
    blockscout: Some("https://polygon.blockscout.com"),
};

/// Base.
///
/// An OP-stack rollup, and the fourth EVM chain here. Two things it does not
/// share with the others:
///
/// * **Its coin is ETH**, the same ETH as Ethereum's, so one phrase gives one
///   address and one coin across both - only the chain id separates a transfer
///   on one from a transfer on the other.
/// * **It cannot price that coin.** Its Uniswap-V2 WETH/USDT pair exists and
///   holds 0.0069 WETH against 17 USDT, so asking it what an ETH is worth
///   returns about seventeen. The liquidity here is on Aerodrome and Uniswap
///   V3, neither of which speaks `getAmountsOut`. So the price comes from
///   Ethereum, where the same asset has a pool worth millions.
///
/// Gas is paid in ETH and costs almost nothing: a base fee of five thousandths
/// of a gwei is ordinary.
pub const BASE: EvmChain = EvmChain {
    chain_id: 8453,
    native_symbol: "ETH",
    native_decimals: 18,
    // **USDC, not USDT.** Tether's contract on this chain holds 23 million
    // against Circle's 4.2 billion, and Binance does not offer USDT to Base at
    // all - it offers ETH and USDC. A USDT row here is one nobody can fill.
    stable: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
    stable_decimals: 6,
    stable_symbol: "USDC",
    stable_label: "USDC",
    default_rpc: "https://base-rpc.publicnode.com",
    explorer_tx: "https://basescan.org/tx/",
    // Present and very nearly empty. Kept because it is the real address, and
    // never asked for a price: `prices_on` sends that question elsewhere.
    router: "0x4752ba5DBc23f44D87826276BF6Fd6b1C372aD24",
    wrapped_native: "0x4200000000000000000000000000000000000006",
    tx_type: TxType::Eip1559,
    history_host: None,
    prices_on: Some(1),
    blockscout: Some("https://base.blockscout.com"),
};

/// Every chain in this file, so a chain id can be turned back into its
/// parameters without a chain of `if`s that forgets the newest one.
pub const ALL: [EvmChain; 4] = [BSC, ETHEREUM, POLYGON, BASE];

pub fn by_chain_id(id: u64) -> Option<EvmChain> {
    ALL.into_iter().find(|c| c.chain_id == id)
}

impl EvmChain {
    /// The chain whose pool prices this one's coin.
    ///
    /// Itself, unless [`Self::prices_on`] says otherwise.
    pub fn price_chain(&self) -> EvmChain {
        match self.prices_on.and_then(by_chain_id) {
            Some(c) => c,
            None => *self,
        }
    }
}

impl EvmChain {
    pub fn stable_address(&self) -> EvmAddress {
        parse_const(self.stable)
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
        for c in ALL {
            for s in [c.stable, c.router, c.wrapped_native] {
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
        assert_eq!(POLYGON.chain_id, 137);
        assert_eq!(BASE.chain_id, 8453);
        let ids: Vec<u64> = ALL.iter().map(|c| c.chain_id).collect();
        for (i, a) in ids.iter().enumerate() {
            for b in &ids[i + 1..] {
                assert_ne!(a, b, "two chains share a chain id");
            }
        }

        assert_eq!(BSC.stable_decimals, 18);
        assert_eq!(
            ETHEREUM.stable_decimals, 6,
            "read from the chain, not assumed"
        );
        assert_eq!(POLYGON.stable_decimals, 6);
        let usdt: Vec<&str> = ALL.iter().map(|c| c.stable).collect();
        for (i, a) in usdt.iter().enumerate() {
            for b in &usdt[i + 1..] {
                assert_ne!(a, b, "different contracts entirely");
            }
        }
    }

    /// The contract's own name, which is not always the one on the tin, and
    /// not always the name this wallet shows.
    ///
    /// Read from each chain. BNB Chain and Ethereum say `USDT`. Polygon says
    /// `USDT0` and is still shown as USDT, because it is the token everyone
    /// means by that name. Base's stablecoin is a different token entirely and
    /// is shown as what it is.
    ///
    /// The send path compares against `stable_symbol` before signing, and when
    /// that was the literal `"USDT"` Polygon could not send at all.
    #[test]
    fn the_token_is_checked_against_the_name_it_actually_has() {
        for (c, symbol, label) in [
            (BSC, "USDT", "USDT"),
            (ETHEREUM, "USDT", "USDT"),
            (POLYGON, "USDT0", "USDT"),
            (BASE, "USDC", "USDC"),
        ] {
            assert_eq!(c.stable_symbol, symbol, "chain {}", c.chain_id);
            assert_eq!(c.stable_label, label, "chain {}", c.chain_id);
        }
    }

    /// Base's stablecoin is USDC, and that is not a preference.
    ///
    /// Tether's contract on Base holds about 23 million against Circle's 4.2
    /// billion, and Binance lists nineteen networks for USDT withdrawals with
    /// Base on none of them - it offers ETH and USDC there. A USDT row on this
    /// chain is a row nobody can put anything into.
    #[test]
    fn base_holds_usdc_and_the_others_hold_usdt() {
        assert_eq!(
            BASE.stable, "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
            "Circle's USDC on Base"
        );
        assert_eq!(BASE.stable_decimals, 6);
        for c in [BSC, ETHEREUM, POLYGON] {
            assert_eq!(c.stable_label, "USDT", "chain {}", c.chain_id);
            assert_ne!(c.stable, BASE.stable);
        }
    }

    /// A coin that cannot be priced where it lives is priced somewhere it can.
    ///
    /// Base's own WETH/USDT pool held 0.0069 WETH against 17 USDT when this
    /// was written, so asking it returns about seventeen dollars for an ether.
    /// Every other chain here prices its own coin.
    #[test]
    fn base_prices_its_coin_on_ethereum() {
        assert_eq!(BASE.prices_on, Some(ETHEREUM.chain_id));
        assert_eq!(BASE.price_chain().chain_id, ETHEREUM.chain_id);
        assert_eq!(BASE.price_chain().stable_decimals, ETHEREUM.stable_decimals);
        // And it is the same coin, which is what makes the substitution honest
        // rather than approximate.
        assert_eq!(BASE.native_symbol, ETHEREUM.native_symbol);

        for c in [BSC, ETHEREUM, POLYGON] {
            assert_eq!(c.prices_on, None, "chain {} moved its pricing", c.chain_id);
            assert_eq!(c.price_chain().chain_id, c.chain_id);
        }
    }

    /// A chain id resolves to exactly one chain, and to the right one.
    #[test]
    fn a_chain_id_finds_its_parameters() {
        for c in ALL {
            assert_eq!(
                by_chain_id(c.chain_id).map(|f| f.chain_id),
                Some(c.chain_id)
            );
        }
        assert_eq!(by_chain_id(999_999), None);
    }

    /// Not every chain has an index behind it, and the one without says so.
    #[test]
    fn a_chain_without_a_transfer_index_admits_it() {
        assert!(BSC.history_host.is_some());
        assert!(ETHEREUM.history_host.is_some());
        assert_eq!(BASE.history_host, None);
        assert!(BASE.blockscout.is_some(), "and Base reads from Blockscout");
        assert_eq!(
            POLYGON.history_host, None,
            "NodeReal serves no Polygon endpoint; naming one would fail as a network error"
        );
    }

    /// Ethereum's base fee moves and Ethereum's gas is the expensive kind;
    /// BNB Chain's does not and is not.
    #[test]
    fn the_transaction_format_follows_the_fee_market() {
        assert_eq!(ETHEREUM.tx_type, TxType::Eip1559);
        assert_eq!(POLYGON.tx_type, TxType::Eip1559);
        assert_eq!(BASE.tx_type, TxType::Eip1559);
        assert_eq!(BSC.tx_type, TxType::Legacy);
    }
}
