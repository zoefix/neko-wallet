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
    ///
    /// `None` where the chain has none this wallet can use, which is a real
    /// case rather than an oversight: Linea, zkSync Era and Scroll have no
    /// Uniswap-V2 deployment at any known address, and their depth is in V3
    /// and Solidly forks that do not speak `getAmountsOut`. Those three hold
    /// ether and borrow Ethereum's price. Mantle and HyperEVM have a V2 router
    /// each and still say `None`, because the pools behind them hold a few
    /// hundred dollars - see the note on each.
    pub router: Option<&'static str>,
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
    /// The L1 gas price oracle, on a rollup that charges one.
    ///
    /// An OP-stack chain posts its transactions to Ethereum and charges the
    /// sender for that, **on top of** L2 gas - and `op-geth` includes it in the
    /// balance check. A wallet that models a fee as gas times price is short by
    /// exactly this amount, which is why "send everything" on Base failed with
    /// `have 1949655363709653 want 1949655797961312`: 434,251,659 wei of L1
    /// fee it did not know about.
    ///
    /// The predeploy is at the same address on every OP-stack chain.
    pub l1_fee_oracle: Option<&'static str>,
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
    router: Some("0x10ED43C718714eb63d5aA57B78B54704E256024E"),
    wrapped_native: "0xbb4CdB9CBd36B01bD1cBaEBF2De08d9173bc095c",
    tx_type: TxType::Legacy,
    history_host: Some("https://bsc-mainnet.nodereal.io/v1"),
    l1_fee_oracle: None,
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
    router: Some("0x7a250d5630B4cF539739dF2C5dAcb4c659F2488D"),
    wrapped_native: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
    tx_type: TxType::Eip1559,
    history_host: Some("https://eth-mainnet.nodereal.io/v1"),
    l1_fee_oracle: None,
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
    router: Some("0xa5E0829CaCEd8fFDD4De3c43696c57F7D7A678ff"),
    wrapped_native: "0x0d500B1d8E8eF31E21C99d1Db9A6444d3ADf1270",
    // The base fee moves here as it does on Ethereum, and it moves a long way:
    // hundreds of gwei is ordinary.
    tx_type: TxType::Eip1559,
    history_host: None,
    l1_fee_oracle: None,
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
    // None, though a nearly-empty V2 router does exist here. A router that
    // is never asked is a claim nobody checks, and `prices_on` sends the
    // question to Ethereum.
    router: None,
    wrapped_native: "0x4200000000000000000000000000000000000006",
    tx_type: TxType::Eip1559,
    history_host: None,
    // Base is a rollup: its fee is L2 gas *plus* the cost of posting the
    // transaction to Ethereum.
    l1_fee_oracle: Some("0x420000000000000000000000000000000000000F"),
    prices_on: Some(1),
    blockscout: Some("https://base.blockscout.com"),
};

/// Arbitrum One.
///
/// The second rollup here, and it charges for L1 differently enough that
/// copying Base's answer would have been wrong twice over:
///
/// * **No separate L1 fee.** Nitro folds the cost of posting to Ethereum into
///   the gas *estimate* - a plain transfer estimates at 21,302 rather than
///   21,000 - so `gas_limit x price` already covers it and the balance check
///   is the ordinary one. There is no `GasPriceOracle` predeploy here; asking
///   for one returns nothing. Base needs `l1_fee_oracle` and this does not,
///   and the difference is not cosmetic: reserving a phantom L1 fee here would
///   leave dust behind on every "send everything".
/// * **Its coin is ETH**, like Base's, and like Base it cannot price it. The
///   V2 pools hold about $30,000 and quote 2,099 and 2,150 against Ethereum's
///   2,447 - thin enough to sit 14% stale. So the price comes from Ethereum,
///   where the same asset has a pool worth millions.
///
/// Its USDT is real, unlike Base's: 835 million against Circle's 2.6 billion,
/// and Binance will send USDT here. The contract calls itself `USD₮0`, with a
/// tugrik sign where the T should be - which is exactly the sort of name this
/// wallet refuses to render, so it is checked against and never shown.
pub const ARBITRUM: EvmChain = EvmChain {
    chain_id: 42_161,
    native_symbol: "ETH",
    native_decimals: 18,
    stable: "0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9",
    stable_decimals: 6,
    // U+20AE TUGRIK SIGN. Read from the chain, not typed from the ticker.
    stable_symbol: "USD\u{20ae}0",
    stable_label: "USDT",
    default_rpc: "https://arbitrum-one-rpc.publicnode.com",
    explorer_tx: "https://arbiscan.io/tx/",
    // None, for the same reason as Base's.
    router: None,
    wrapped_native: "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1",
    tx_type: TxType::Eip1559,
    history_host: None,
    // Nitro charges for L1 through the gas estimate, not beside it.
    l1_fee_oracle: None,
    prices_on: Some(1),
    blockscout: Some("https://arbitrum.blockscout.com"),
};

/// Optimism.
///
/// The original OP-stack chain - Base runs a fork of its software - and it
/// charges for L1 the same way, which is *not* the way Arbitrum does. Read
/// from the chain rather than assumed, because two of these were surprises:
///
/// * **It charges for L1 beside gas.** The `GasPriceOracle` predeploy is here
///   and answers `getL1Fee`; the code at that address is byte-for-byte the
///   same length as Base's. A plain transfer still estimates at exactly
///   21,000 gas, which is the tell: the posting cost is not in the gas number,
///   so it has to be reserved separately. Arbitrum is the other way round and
///   has no predeploy at all.
/// * **Its V2 router is at a different address from Base's and Arbitrum's.**
///   Those two share `0x4752..AD24`; here that address holds no code, so
///   copying it would have produced a router that silently answers nothing.
///   The one below reports the WETH contract above and a factory of its own.
/// * **Its USDT is plain `USDT`.** No omnichain rename here, unlike Polygon's
///   `USDT0` and Arbitrum's `USD₮0` - 223 million of it, and Binance's own
///   withdrawal entry for Optimism names exactly the contract below.
/// * **It cannot price its coin.** The V2 WETH/USDT pair holds 0.0031 WETH
///   against 7.57 USDT - fifteen dollars in total - and quotes an ether at
///   $7.55. The WETH/USDC pair is the more dangerous one: $592 of liquidity
///   quoting $264, which is wrong by a factor of nine while still looking like
///   a number a coin could cost. The depth here is on Velodrome and Uniswap
///   V3, neither of which speaks `getAmountsOut`.
pub const OPTIMISM: EvmChain = EvmChain {
    chain_id: 10,
    native_symbol: "ETH",
    native_decimals: 18,
    // Tether's own contract, 6 decimals. Binance withdraws USDT here for a
    // 0.04 fee and lists this address for it.
    stable: "0x94b008aA00579c1307B0EF2c499aD98a8ce58e58",
    stable_decimals: 6,
    stable_symbol: "USDT",
    stable_label: "USDT",
    default_rpc: "https://optimism-rpc.publicnode.com",
    explorer_tx: "https://optimistic.etherscan.io/tx/",
    // None. A Uniswap V2 router does exist here, at an address that is
    // **not** the one Base and Arbitrum share - theirs holds no code on this
    // chain - but it is never asked, because `prices_on` sends the question
    // to Ethereum. The distinct address is recorded in the test below, where
    // it can be checked rather than merely stated.
    router: None,
    wrapped_native: "0x4200000000000000000000000000000000000006",
    tx_type: TxType::Eip1559,
    // NodeReal *does* serve an Optimism RPC - unlike Polygon, where the host
    // does not exist - but not the index: `nr_getAssetTransfers` there answers
    // "Method not found". A working host with a missing method is the worse
    // failure of the two, because it looks like the chain is up.
    history_host: None,
    // OP-stack, like Base. This one is load-bearing in the other direction
    // from Arbitrum's `None`.
    l1_fee_oracle: Some("0x420000000000000000000000000000000000000F"),
    prices_on: Some(1),
    // `optimism.blockscout.com` answers 301 and points here.
    blockscout: Some("https://explorer.optimism.io"),
};

/// Avalanche C-Chain.
///
/// Not a rollup and not an L2: its own network, with its own coin, and the
/// only chain added here that prices itself from its own pool. Three things
/// were read rather than assumed:
///
/// * **Its USDT contract calls itself `USDt`** - a lowercase t. Not `USDT`,
///   not Polygon's `USDT0`, not Arbitrum's `USD₮0`. Four chains, four
///   spellings of the same three letters, and the send path compares this one
///   against the chain before signing.
/// * **1.85 billion of it**, against Circle's 375 million, and Binance
///   withdraws USDT here for 0.04 - so unlike Base, Tether is the right choice.
/// * **Its router is Trader Joe's**, whose `WETH()` accessor does not exist -
///   it is called `WAVAX()` there - so the router was verified by quoting
///   through it rather than by asking its name. Cross-checked against Pangolin,
///   which agreed to 0.02%: 7.5920 against 7.5936 for a reference of 7.611.
pub const AVALANCHE: EvmChain = EvmChain {
    chain_id: 43_114,
    native_symbol: "AVAX",
    native_decimals: 18,
    stable: "0x9702230A8Ea53601f5cD2dc00fDBc13d4dF4A8c7",
    stable_decimals: 6,
    // Lowercase `t`. Read from the chain.
    stable_symbol: "USDt",
    stable_label: "USDT",
    default_rpc: "https://avalanche-c-chain-rpc.publicnode.com",
    explorer_tx: "https://snowtrace.io/tx/",
    // Trader Joe V1, a Uniswap V2 fork. Its WAVAX/USDt pool held $41,887.
    router: Some("0x60aE616a2155Ee3d9A68541Ba4544862310933d4"),
    wrapped_native: "0xB31f66AA3C1e785363F0875A1B74E27b85FD66c7",
    tx_type: TxType::Eip1559,
    history_host: None,
    l1_fee_oracle: None,
    prices_on: None,
    blockscout: None,
};

/// HyperEVM.
///
/// The EVM side of Hyperliquid, and the one chain here whose coin this wallet
/// will not put a price on.
///
/// Its only V2 venue is HyperSwap, whose WHYPE/USD₮0 pool holds **$1,104**.
/// The pool's spot price is right - 85.71 against the 85.60 Hyperliquid's own
/// API reports - but a quote for one whole HYPE comes back at 74.04, because
/// buying one unit out of a six-unit pool moves it 13%. A pool that small is
/// not a price; it is a number that happens to be near one today and can be
/// moved for a few hundred dollars. So `router` is `None` and the coin has no
/// price, which the interface shows as unknown rather than as a figure.
///
/// Its stablecoin is Tether's omnichain deployment, `USD₮0`, with the same
/// tugrik sign as Arbitrum's. There is no Binance route to this chain at all -
/// HYPE is not listed there - so it is funded by bridge.
pub const HYPER_EVM: EvmChain = EvmChain {
    chain_id: 999,
    native_symbol: "HYPE",
    native_decimals: 18,
    stable: "0xB8CE59FC3717ada4C02eaDF9682A9e934F625ebb",
    stable_decimals: 6,
    // U+20AE again, as on Arbitrum.
    stable_symbol: "USD\u{20ae}0",
    stable_label: "USDT",
    default_rpc: "https://rpc.hyperliquid.xyz/evm",
    explorer_tx: "https://hyperevmscan.io/tx/",
    // HyperSwap exists and is far too thin to quote from - see above.
    router: None,
    wrapped_native: "0x5555555555555555555555555555555555555555",
    tx_type: TxType::Eip1559,
    history_host: None,
    l1_fee_oracle: None,
    prices_on: None,
    blockscout: None,
};

/// Mantle.
///
/// An OP-stack chain whose coin is **not** ether, which is the combination
/// that makes it its own case:
///
/// * **It charges for L1 beside gas**, like Base and Optimism. The predeploy
///   is at the same address and holds the same 2,055 bytes, and it answered
///   37,147,555,070 wei for a 110-byte payload - an order of magnitude more
///   than either of them.
/// * **Its coin is MNT**, so it cannot borrow Ethereum's price the way the
///   ether rollups do. Its own V2 pools hold $395 and $516; MNT exists as an
///   ERC-20 on Ethereum but has no Uniswap-V2 pair there at all. So no price,
///   rather than one drawn from five hundred dollars of liquidity.
///
/// Binance does not withdraw MNT to this network - it lists MNT for fiat only -
/// so this chain is funded by bridge.
pub const MANTLE: EvmChain = EvmChain {
    chain_id: 5_000,
    native_symbol: "MNT",
    native_decimals: 18,
    // USDC, which holds 23.7 million here against Tether's 12.9 million.
    stable: "0x09Bc4E0D864854c6aFB6eB9A9cdF58aC190D0dF9",
    stable_decimals: 6,
    stable_symbol: "USDC",
    stable_label: "USDC",
    default_rpc: "https://rpc.mantle.xyz",
    explorer_tx: "https://mantlescan.xyz/tx/",
    router: None,
    wrapped_native: "0x78c1b0C915c4FAA5FffA6CAbf0219DA63d7f4cb8",
    tx_type: TxType::Eip1559,
    history_host: None,
    // OP-stack, like Base and Optimism.
    l1_fee_oracle: Some("0x420000000000000000000000000000000000000F"),
    prices_on: None,
    blockscout: None,
};

/// Linea.
///
/// A zkEVM, and the quietest chain added here: no L1 fee beside gas, a plain
/// transfer at exactly 21,000, and nothing unusual in its tokens.
///
/// Two things are worth saying. It has **no Uniswap-V2 router at any known
/// address** - its depth is on Lynex and Nile, which are Solidly forks and do
/// not answer `getAmountsOut` - so its ether is priced from Ethereum like the
/// rollups'. And **Binance does not send anything here**: not ether, not USDT,
/// not USDC, only the LINEA token itself. Funding this chain means bridging.
pub const LINEA: EvmChain = EvmChain {
    chain_id: 59_144,
    native_symbol: "ETH",
    native_decimals: 18,
    // USDC, 23.2 million against Tether's 7.3 million.
    stable: "0x176211869cA2b568f2A7D4EE941E073a821EE1ff",
    stable_decimals: 6,
    stable_symbol: "USDC",
    stable_label: "USDC",
    default_rpc: "https://rpc.linea.build",
    explorer_tx: "https://lineascan.build/tx/",
    router: None,
    wrapped_native: "0xe5D7C2a44FfDDf6b295A15c148167daaAf5Cf34f",
    tx_type: TxType::Eip1559,
    history_host: None,
    l1_fee_oracle: None,
    prices_on: Some(1),
    blockscout: None,
};

/// zkSync Era.
///
/// The chain whose gas number looks broken and is not. A plain transfer here
/// estimates at **178,573 gas**, not 21,000, because Era folds the cost of
/// publishing state to Ethereum into the gas figure - the same choice Arbitrum
/// makes, taken much further. There is no oracle predeploy and nothing to
/// reserve beside gas; a wallet that "corrects" this number to 21,000 produces
/// a transaction that cannot be included.
///
/// Its stablecoin is USDC, and that follows Base's rule rather than a
/// preference: Binance sends USDC here and does not send USDT at all, and
/// Circle's 10.6 million against Tether's 2.4 million says the same thing.
pub const ZKSYNC_ERA: EvmChain = EvmChain {
    chain_id: 324,
    native_symbol: "ETH",
    native_decimals: 18,
    stable: "0x1d17CBcF0D6D143135aE902365D2E5e2A16538D4",
    stable_decimals: 6,
    stable_symbol: "USDC",
    stable_label: "USDC",
    default_rpc: "https://mainnet.era.zksync.io",
    explorer_tx: "https://explorer.zksync.io/tx/",
    router: None,
    wrapped_native: "0x5AEa5775959fBC2557Cc8789bC1bf90A239D9a91",
    tx_type: TxType::Eip1559,
    history_host: None,
    l1_fee_oracle: None,
    prices_on: Some(1),
    blockscout: Some("https://zksync.blockscout.com"),
};

/// Scroll.
///
/// A zkEVM that charges for L1 beside gas - and **not at the OP-stack
/// address**. Its `L1GasPriceOracle` is at `0x53..02`, holds 3,782 bytes where
/// the OP-stack one holds 2,055, and answered 561,722,474,377 wei for a
/// 110-byte payload. It does answer the same `getL1Fee(bytes)` selector, which
/// is what lets one field cover both designs; the address is what differs, and
/// assuming the OP-stack one would have found no code and reserved nothing.
///
/// Its stablecoin is USDT even though Circle's supply here is slightly larger -
/// 5.09 million against 4.18 million - because Binance withdraws USDT to this
/// chain and does not withdraw USDC. That is the Base rule applied the other
/// way round: what matters is whether the row can be filled.
pub const SCROLL: EvmChain = EvmChain {
    chain_id: 534_352,
    native_symbol: "ETH",
    native_decimals: 18,
    stable: "0xf55BEC9cafDbE8730f096Aa55dad6D22d44099Df",
    stable_decimals: 6,
    stable_symbol: "USDT",
    stable_label: "USDT",
    default_rpc: "https://rpc.scroll.io",
    explorer_tx: "https://scrollscan.com/tx/",
    router: None,
    wrapped_native: "0x5300000000000000000000000000000000000004",
    tx_type: TxType::Eip1559,
    history_host: None,
    // Scroll's own, at its own address.
    l1_fee_oracle: Some("0x5300000000000000000000000000000000000002"),
    prices_on: Some(1),
    // Scroll's explorer serves the Blockscout v2 API, keyless.
    blockscout: Some("https://scrollscan.com"),
};

/// Every chain in this file, so a chain id can be turned back into its
/// parameters without a chain of `if`s that forgets the newest one.
pub const ALL: [EvmChain; 12] = [
    BSC,
    ETHEREUM,
    POLYGON,
    BASE,
    ARBITRUM,
    OPTIMISM,
    AVALANCHE,
    HYPER_EVM,
    MANTLE,
    LINEA,
    ZKSYNC_ERA,
    SCROLL,
];

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
    /// `None` on a chain with no router - see [`Self::router`]. A caller that
    /// wants a price has to handle that rather than receive a plausible
    /// number from a pool that does not exist.
    pub fn router_address(&self) -> Option<EvmAddress> {
        self.router.map(parse_const)
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
            for s in [Some(c.stable), c.router, Some(c.wrapped_native)]
                .into_iter()
                .flatten()
            {
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
        assert_eq!(ARBITRUM.chain_id, 42_161);
        assert_eq!(OPTIMISM.chain_id, 10);
        assert_eq!(AVALANCHE.chain_id, 43_114);
        assert_eq!(HYPER_EVM.chain_id, 999);
        assert_eq!(MANTLE.chain_id, 5_000);
        assert_eq!(LINEA.chain_id, 59_144);
        assert_eq!(ZKSYNC_ERA.chain_id, 324);
        assert_eq!(SCROLL.chain_id, 534_352);
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
            // A tugrik sign where the T should be. Checked against, never
            // shown.
            (ARBITRUM, "USD\u{20ae}0", "USDT"),
            // And Optimism's contract still says plain `USDT`.
            (OPTIMISM, "USDT", "USDT"),
            // A lowercase t. A fourth spelling of the same three letters.
            (AVALANCHE, "USDt", "USDT"),
            // The tugrik again, on a chain with no Binance route at all.
            (HYPER_EVM, "USD\u{20ae}0", "USDT"),
            (MANTLE, "USDC", "USDC"),
            (LINEA, "USDC", "USDC"),
            (ZKSYNC_ERA, "USDC", "USDC"),
            (SCROLL, "USDT", "USDT"),
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

        // Which chains carry which, over `ALL` rather than a list. Four carry
        // USDC and each for its own reason: Base because Binance sends no
        // USDT there at all, zkSync Era for the same reason, and Mantle and
        // Linea because Circle's supply is the larger and neither token has a
        // Binance route to those chains.
        let usdc = ["8453", "324", "5000", "59144"];
        for c in ALL {
            let want = if usdc.contains(&c.chain_id.to_string().as_str()) {
                "USDC"
            } else {
                "USDT"
            };
            assert_eq!(c.stable_label, want, "chain {}", c.chain_id);
        }
        // Scroll is the interesting one: Circle's supply there is the larger
        // of the two and it still carries USDT, because Binance withdraws USDT
        // to Scroll and does not withdraw USDC. A row nobody can fill is worse
        // than a slightly smaller one.
        assert_eq!(SCROLL.stable_label, "USDT");

        // And Optimism is the counter-example that makes the Base decision a
        // finding rather than a policy about rollups: same kind of chain, and
        // there Tether is the bigger of the two - 223 million, with Binance
        // withdrawing to it for a 0.04 fee.
        assert_eq!(
            OPTIMISM.stable, "0x94b008aA00579c1307B0EF2c499aD98a8ce58e58",
            "Tether's contract on Optimism, the address Binance itself lists"
        );
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

        // Arbitrum's coin is ETH too, and its own pools are thin enough to
        // sit 14% stale.
        assert_eq!(ARBITRUM.prices_on, Some(ETHEREUM.chain_id));
        // Optimism's V2 pools are emptier still: fifteen dollars of WETH/USDT
        // quoting an ether at $7.55, and a WETH/USDC pair holding $592 that
        // answers $264 - wrong by a factor of nine while still looking like a
        // number a coin could cost.
        assert_eq!(OPTIMISM.prices_on, Some(ETHEREUM.chain_id));
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
        for c in [BASE, ARBITRUM, OPTIMISM, ZKSYNC_ERA, SCROLL] {
            assert_eq!(c.history_host, None, "chain {}", c.chain_id);
            assert!(
                c.blockscout.is_some(),
                "chain {} reads from Blockscout",
                c.chain_id
            );
        }
        // And four chains have neither: NodeReal does not serve them and no
        // keyless Blockscout answers for them, so history there needs the
        // user's own Etherscan key or says it is unavailable. Etherscan V2
        // lists all four; it lists neither zkSync Era nor Scroll, which is
        // why those two read from Blockscout instead.
        for c in [POLYGON, AVALANCHE, HYPER_EVM, MANTLE, LINEA] {
            assert_eq!(c.history_host, None, "chain {}", c.chain_id);
        }
        for c in [AVALANCHE, HYPER_EVM, MANTLE, LINEA] {
            assert_eq!(
                c.blockscout, None,
                "chain {} has no keyless index",
                c.chain_id
            );
        }
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
        assert_eq!(ARBITRUM.tx_type, TxType::Eip1559);
        // Every chain here but BNB Chain has a moving base fee.
        for c in ALL {
            let want = if c.chain_id == BSC.chain_id {
                TxType::Legacy
            } else {
                TxType::Eip1559
            };
            assert_eq!(c.tx_type, want, "chain {}", c.chain_id);
        }
    }
}

#[cfg(test)]
mod rollup_fees {
    use super::*;

    /// Which chains charge for L1 beside gas, and at which address.
    ///
    /// Three do, and they do not all use the same oracle:
    ///
    /// * **Base, Optimism, Mantle** are OP-stack and share the predeploy at
    ///   `0x42..0F`, which holds the same 2,055 bytes on all three.
    /// * **Scroll** charges too, at **its own address**, `0x53..02`, which
    ///   holds 3,782 bytes. It answers the same `getL1Fee(bytes)` selector -
    ///   which is why one field covers both designs - but looking for it at
    ///   the OP-stack address finds no code and reserves nothing.
    ///
    /// Everything else pays for L1 through the gas number or not at all.
    /// Arbitrum and zkSync Era are the two that fold it into gas, and the
    /// chain says so: a plain transfer estimates at 21,422 on Arbitrum and
    /// 178,573 on Era, against exactly 21,000 on every chain in the first
    /// group. Reserving a second fee on either would hold back money the chain
    /// never charges.
    ///
    /// Written over `ALL`. The previous version named four chains and called
    /// the fifth the only one with an oracle; when a sixth arrived it was in
    /// neither list and the test went on passing.
    #[test]
    fn which_chains_charge_for_l1_separately_is_read_from_each_chain() {
        const OP_STACK: &str = "0x420000000000000000000000000000000000000F";
        const SCROLLS: &str = "0x5300000000000000000000000000000000000002";

        for c in ALL {
            let want = match c.chain_id {
                8453 | 10 | 5_000 => Some(OP_STACK),
                534_352 => Some(SCROLLS),
                _ => None,
            };
            assert_eq!(
                c.l1_fee_oracle, want,
                "chain {} has the wrong L1 oracle",
                c.chain_id
            );
        }

        // Four chains reserve one, and every chain is in exactly one group, so
        // a new one cannot be added without this test having an opinion.
        assert_eq!(ALL.iter().filter(|c| c.l1_fee_oracle.is_some()).count(), 4);

        // Scroll's is not the OP-stack address, and that is the point: the
        // same selector at a different place.
        assert_ne!(SCROLL.l1_fee_oracle, BASE.l1_fee_oracle);
        assert_eq!(BASE.l1_fee_oracle, MANTLE.l1_fee_oracle);
        assert_eq!(BASE.l1_fee_oracle, OPTIMISM.l1_fee_oracle);

        // The two that fold it into gas are rollups and are not on the list.
        // "Is it a rollup" is the wrong question, asked twice.
        for c in [ARBITRUM, ZKSYNC_ERA] {
            assert_eq!(
                c.l1_fee_oracle, None,
                "chain {} charges through the gas estimate, not beside it",
                c.chain_id
            );
        }
    }

    /// Where each chain's coin gets its price, and which chains have none.
    ///
    /// Three answers, and every chain must give exactly one of them:
    ///
    /// * **Its own pool.** BNB Chain, Ethereum, Polygon, Avalanche.
    /// * **Ethereum's pool**, for the five chains whose coin is ether.
    /// * **Nowhere**, for Mantle and HyperEVM. Their coins are MNT and HYPE,
    ///   which exist on no chain this wallet talks to, and their own V2 pools
    ///   hold $395 and $1,104. A pool that size answers a number rather than
    ///   an error, which is the dangerous shape: HyperEVM's spot price is
    ///   right to 0.1% and a quote for one whole coin comes back 13% low,
    ///   because one unit is a sixth of the pool.
    ///
    /// The combination that must never happen is a router with no depth behind
    /// it, which is why "no price" is spelled out here rather than left to
    /// whether a call happens to fail.
    #[test]
    fn every_chain_says_where_its_coin_is_priced_or_that_it_is_not() {
        let mut own = 0;
        let mut borrowed = 0;
        let mut none = 0;
        for c in ALL {
            match (c.router, c.prices_on) {
                (Some(_), None) => {
                    own += 1;
                    // Ethereum is the one chain that holds ether *and* prices
                    // it here; every other ether chain borrows from it.
                    assert!(
                        c.native_symbol != "ETH" || c.chain_id == ETHEREUM.chain_id,
                        "chain {} holds ether and should borrow Ethereum's price",
                        c.chain_id
                    );
                }
                (None, Some(id)) => {
                    borrowed += 1;
                    assert_eq!(id, ETHEREUM.chain_id);
                    assert_eq!(
                        c.native_symbol, "ETH",
                        "chain {} borrows a price for a coin that is not ether",
                        c.chain_id
                    );
                }
                (None, None) => {
                    none += 1;
                    assert!(
                        matches!(c.chain_id, 5_000 | 999),
                        "chain {} has no price and is not one of the two that may not",
                        c.chain_id
                    );
                }
                (Some(_), Some(_)) => panic!(
                    "chain {} both names a router and prices elsewhere; only one can be true",
                    c.chain_id
                ),
            }
        }
        assert_eq!((own, borrowed, none), (4, 6, 2));

        // Avalanche is the only chain added since Polygon that quotes its own
        // coin, and the only new router in the file.
        assert_eq!(
            AVALANCHE.router,
            Some("0x60aE616a2155Ee3d9A68541Ba4544862310933d4")
        );
        // These two have a V2 router each and still say `None`: the pools
        // behind them are too small to be a price.
        assert_eq!(MANTLE.router, None);
        assert_eq!(HYPER_EVM.router, None);
    }

    /// No two chains share a router, and every router that is listed is one
    /// this wallet has quoted through.
    ///
    /// A router copied from the chain above is the quiet failure this guards
    /// against: the address either holds no code, in which case the price
    /// silently goes missing, or holds a *different* chain's router and quotes
    /// a pool that has nothing to do with the coin being priced. Optimism is
    /// where that nearly happened - Base and Arbitrum share `0x4752..AD24`,
    /// and on Optimism that address holds no code at all - and it stopped
    /// mattering only because all three now price on Ethereum instead.
    ///
    /// Four routers remain, one per chain that quotes its own coin.
    #[test]
    fn no_two_chains_share_a_router() {
        let routers: Vec<(u64, &str)> = ALL
            .iter()
            .filter_map(|c| c.router.map(|r| (c.chain_id, r)))
            .collect();
        assert_eq!(routers.len(), 4, "one router per self-pricing chain");
        for (i, (ca, a)) in routers.iter().enumerate() {
            for (cb, b) in &routers[i + 1..] {
                assert_ne!(a, b, "chains {ca} and {cb} share a router address");
            }
        }

        // Each is on the chain it belongs to, checked by the one thing that
        // ties them together: the wrapped native each quotes against.
        assert_eq!(
            AVALANCHE.wrapped_native,
            "0xB31f66AA3C1e785363F0875A1B74E27b85FD66c7",
            "WAVAX, which Trader Joe's router pairs against"
        );

        // Base and Optimism share a wrapped-native address - OP-stack puts it
        // at a fixed predeploy - so the coin's address can never tell those
        // two apart. Only the chain id does.
        assert_eq!(BASE.wrapped_native, OPTIMISM.wrapped_native);
        assert_ne!(BASE.chain_id, OPTIMISM.chain_id);
    }

    /// Every rollup here is priced off Ethereum, and every chain that is not a
    /// rollup prices itself. Both are consequences of the coin being ETH.
    #[test]
    fn the_chains_whose_coin_is_ether_borrow_ethereums_price() {
        for c in ALL {
            if c.chain_id == ETHEREUM.chain_id {
                assert_eq!(c.prices_on, None);
            } else if c.native_symbol == "ETH" {
                assert_eq!(
                    c.prices_on,
                    Some(ETHEREUM.chain_id),
                    "chain {} holds ether and should borrow its price",
                    c.chain_id
                );
            } else {
                assert_eq!(c.prices_on, None, "chain {}", c.chain_id);
            }
        }
    }
}
