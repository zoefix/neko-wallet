//! Which chain, and what an address means on it.
//!
//! Modelled as enums rather than traits, on purpose. A trait would let a new
//! chain be added without touching the screens that spend money; an enum makes
//! the compiler list every place that has to decide, and refuse to build until
//! each one has. For code that moves funds, being forced to look is the point.
//!
//! The addresses are deliberately *not* interchangeable. A TRON address and a
//! BNB Chain address are different types, so pasting one into the other's send
//! form is a parse error rather than a transfer into an account nobody holds
//! the key to.

use crate::error::CoreError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ChainId {
    Tron,
    Bsc,
    Solana,
    Bitcoin,
    Ethereum,
    Ton,
    Polygon,
    Base,
    Arbitrum,
    Optimism,
    Avalanche,
    HyperEvm,
    Mantle,
    Linea,
    ZkSyncEra,
    Scroll,
    Aptos,
    Sui,
}

pub const CHAINS: [ChainId; 18] = [
    ChainId::Tron,
    ChainId::Bsc,
    ChainId::Solana,
    ChainId::Bitcoin,
    ChainId::Ethereum,
    ChainId::Ton,
    ChainId::Polygon,
    ChainId::Base,
    ChainId::Arbitrum,
    ChainId::Optimism,
    ChainId::Avalanche,
    ChainId::HyperEvm,
    ChainId::Mantle,
    ChainId::Linea,
    ChainId::ZkSyncEra,
    ChainId::Scroll,
    ChainId::Aptos,
    ChainId::Sui,
];

impl ChainId {
    /// Stored in the database and in settings; never shown to a user.
    pub fn slug(self) -> &'static str {
        match self {
            ChainId::Tron => "tron",
            ChainId::Bsc => "bsc",
            ChainId::Solana => "solana",
            ChainId::Bitcoin => "bitcoin",
            ChainId::Ethereum => "ethereum",
            ChainId::Ton => "ton",
            ChainId::Polygon => "polygon",
            ChainId::Base => "base",
            ChainId::Arbitrum => "arbitrum",
            ChainId::Optimism => "optimism",
            ChainId::Avalanche => "avalanche",
            ChainId::HyperEvm => "hyperevm",
            ChainId::Mantle => "mantle",
            ChainId::Linea => "linea",
            ChainId::ZkSyncEra => "zksync_era",
            ChainId::Scroll => "scroll",
            ChainId::Aptos => "aptos",
            ChainId::Sui => "sui",
        }
    }

    pub fn from_slug(s: &str) -> Option<Self> {
        CHAINS.into_iter().find(|c| c.slug() == s)
    }

    pub fn label(self) -> &'static str {
        match self {
            ChainId::Tron => "TRON",
            ChainId::Bsc => "BNB Chain",
            ChainId::Solana => "Solana",
            ChainId::Bitcoin => "Bitcoin",
            ChainId::Ethereum => "Ethereum",
            // The network, which kept its name when the coin did not.
            ChainId::Ton => "TON",
            ChainId::Polygon => "Polygon",
            ChainId::Base => "Base",
            ChainId::Arbitrum => "Arbitrum",
            ChainId::Optimism => "Optimism",
            ChainId::Avalanche => "Avalanche",
            ChainId::HyperEvm => "HyperEVM",
            ChainId::Mantle => "Mantle",
            ChainId::Linea => "Linea",
            ChainId::ZkSyncEra => "zkSync Era",
            ChainId::Scroll => "Scroll",
            ChainId::Aptos => "Aptos",
            ChainId::Sui => "Sui",
        }
    }

    /// SLIP-44 coin type. BNB Chain uses 60, Ethereum's, as every EVM wallet
    /// does - so an address here matches what MetaMask shows for the same
    /// phrase.
    pub fn coin_type(self) -> u32 {
        match self {
            ChainId::Tron => neko_hd::derive::COIN_TYPE,
            ChainId::Bsc => neko_hd::derive::COIN_TYPE_EVM,
            ChainId::Solana => neko_hd::COIN_TYPE_SOLANA,
            ChainId::Bitcoin => neko_hd::COIN_TYPE_BTC,
            // 60, the same as BNB Chain's: every EVM chain shares Ethereum's
            // coin type, so one phrase gives the same address on all of them.
            ChainId::Ethereum
            | ChainId::Polygon
            | ChainId::Base
            | ChainId::Arbitrum
            | ChainId::Optimism
            | ChainId::Avalanche
            | ChainId::HyperEvm
            | ChainId::Mantle
            | ChainId::Linea
            | ChainId::ZkSyncEra
            | ChainId::Scroll => neko_hd::derive::COIN_TYPE_EVM,
            ChainId::Ton => neko_hd::COIN_TYPE_TON,
            ChainId::Aptos => neko_hd::COIN_TYPE_APTOS,
            ChainId::Sui => neko_hd::COIN_TYPE_SUI,
        }
    }

    pub fn native_symbol(self) -> &'static str {
        match self {
            ChainId::Tron => "TRX",
            ChainId::Bsc => "BNB",
            ChainId::Solana => "SOL",
            ChainId::Bitcoin => "BTC",
            ChainId::Ethereum => "ETH",
            // Renamed from Toncoin on 15 June 2026, back to the name it had in
            // Telegram's 2018 whitepaper. Only the ticker changed.
            ChainId::Ton => "GRAM",
            // Renamed from MATIC in September 2024, and the chain says so
            // itself: the wrapped native contract reports WPOL.
            ChainId::Polygon => neko_evm::POLYGON.native_symbol,
            // ETH, the same coin as Ethereum's. Only the chain id separates a
            // transfer on one from a transfer on the other.
            ChainId::Base => neko_evm::BASE.native_symbol,
            // ETH again. Four chains here call their coin that, and only
            // the chain id tells a transfer on one from a transfer on another.
            ChainId::Arbitrum => neko_evm::ARBITRUM.native_symbol,
            ChainId::Optimism => neko_evm::OPTIMISM.native_symbol,
            ChainId::Avalanche => neko_evm::AVALANCHE.native_symbol,
            ChainId::HyperEvm => neko_evm::HYPER_EVM.native_symbol,
            ChainId::Mantle => neko_evm::MANTLE.native_symbol,
            ChainId::Linea => neko_evm::LINEA.native_symbol,
            ChainId::ZkSyncEra => neko_evm::ZKSYNC_ERA.native_symbol,
            ChainId::Scroll => neko_evm::SCROLL.native_symbol,
            ChainId::Aptos => "APT",
            ChainId::Sui => "SUI",
        }
    }

    /// TRX has 6, BNB has 18. Nothing in this program may assume a number of
    /// decimals; it always comes from the chain or the token.
    pub fn native_decimals(self) -> u8 {
        match self {
            ChainId::Tron => neko_tron::TRX_DECIMALS,
            ChainId::Bsc => neko_evm::BSC.native_decimals,
            ChainId::Ethereum => neko_evm::ETHEREUM.native_decimals,
            ChainId::Polygon => neko_evm::POLYGON.native_decimals,
            ChainId::Base => neko_evm::BASE.native_decimals,
            ChainId::Arbitrum => neko_evm::ARBITRUM.native_decimals,
            ChainId::Optimism => neko_evm::OPTIMISM.native_decimals,
            ChainId::Avalanche => neko_evm::AVALANCHE.native_decimals,
            ChainId::HyperEvm => neko_evm::HYPER_EVM.native_decimals,
            ChainId::Mantle => neko_evm::MANTLE.native_decimals,
            ChainId::Linea => neko_evm::LINEA.native_decimals,
            ChainId::ZkSyncEra => neko_evm::ZKSYNC_ERA.native_decimals,
            ChainId::Scroll => neko_evm::SCROLL.native_decimals,
            ChainId::Aptos => neko_aptos::APT_DECIMALS,
            ChainId::Sui => neko_sui::SUI_DECIMALS,
            ChainId::Ton => neko_ton::GRAM_DECIMALS,
            ChainId::Solana => neko_solana::SOL_DECIMALS,
            ChainId::Bitcoin => neko_btc::BTC_DECIMALS,
        }
    }

    /// The stablecoin this wallet knows about on each chain.
    ///
    /// **Not always USDT.** Seven of these chains carry Tether; Base carries
    /// USDC, because Tether's contract there holds 23 million against Circle's
    /// 4.2 billion and Binance will not send USDT to that chain at all. A USDT
    /// row on Base is a row nobody can put anything into.
    ///
    /// Same name where it is the same token, different precision: 6 decimals
    /// on TRON, 18 on BNB Chain. Treating one like the other is a factor of a
    /// million million, which is why the number travels with the asset rather
    /// than living in a constant. `None` on a chain with no such token.
    ///
    /// Bitcoin is the one: it carries one asset and no contracts, so a screen
    /// that assumed two assets per chain would show an empty row where nothing
    /// exists.
    pub fn stable(self) -> Option<Asset> {
        match self {
            ChainId::Tron => Some(Asset::Trc20 {
                contract: neko_tron::usdt_address(),
                decimals: neko_tron::USDT_DECIMALS,
            }),
            ChainId::Solana => Some(Asset::SplToken {
                mint: neko_solana::usdt_mint(),
                decimals: neko_solana::USDT_DECIMALS,
            }),
            ChainId::Bitcoin => None,
            ChainId::Bsc => Some(Asset::Bep20 {
                contract: neko_evm::BSC.stable_address(),
                decimals: neko_evm::BSC.stable_decimals,
            }),
            // A different contract *and* a different precision from BNB
            // Chain's: six decimals here, eighteen there.
            ChainId::Ethereum => Some(Asset::Erc20 {
                contract: neko_evm::ETHEREUM.stable_address(),
                decimals: neko_evm::ETHEREUM.stable_decimals,
            }),
            // Six decimals like Ethereum's, and a contract that calls itself
            // USDT0 - see `neko_evm::POLYGON`.
            ChainId::Polygon => Some(Asset::PolygonErc20 {
                contract: neko_evm::POLYGON.stable_address(),
                decimals: neko_evm::POLYGON.stable_decimals,
            }),
            ChainId::Base => Some(Asset::BaseErc20 {
                contract: neko_evm::BASE.stable_address(),
                decimals: neko_evm::BASE.stable_decimals,
            }),
            // Real USDT here, unlike Base's - 835 million of it, and Binance
            // will send it. The contract calls itself `USD₮0`.
            ChainId::Arbitrum => Some(Asset::ArbitrumErc20 {
                contract: neko_evm::ARBITRUM.stable_address(),
                decimals: neko_evm::ARBITRUM.stable_decimals,
            }),
            // Plain `USDT`, the only one of the three rollups here whose
            // contract still calls itself that.
            ChainId::Optimism => Some(Asset::OptimismErc20 {
                contract: neko_evm::OPTIMISM.stable_address(),
                decimals: neko_evm::OPTIMISM.stable_decimals,
            }),
            ChainId::Avalanche => Some(Asset::AvalancheErc20 {
                contract: neko_evm::AVALANCHE.stable_address(),
                decimals: neko_evm::AVALANCHE.stable_decimals,
            }),
            ChainId::HyperEvm => Some(Asset::HyperEvmErc20 {
                contract: neko_evm::HYPER_EVM.stable_address(),
                decimals: neko_evm::HYPER_EVM.stable_decimals,
            }),
            ChainId::Mantle => Some(Asset::MantleErc20 {
                contract: neko_evm::MANTLE.stable_address(),
                decimals: neko_evm::MANTLE.stable_decimals,
            }),
            ChainId::Linea => Some(Asset::LineaErc20 {
                contract: neko_evm::LINEA.stable_address(),
                decimals: neko_evm::LINEA.stable_decimals,
            }),
            ChainId::ZkSyncEra => Some(Asset::ZkSyncEraErc20 {
                contract: neko_evm::ZKSYNC_ERA.stable_address(),
                decimals: neko_evm::ZKSYNC_ERA.stable_decimals,
            }),
            ChainId::Scroll => Some(Asset::ScrollErc20 {
                contract: neko_evm::SCROLL.stable_address(),
                decimals: neko_evm::SCROLL.stable_decimals,
            }),
            ChainId::Ton => Some(Asset::Jetton {
                master: neko_ton::usdt_master(),
                decimals: neko_ton::USDT_DECIMALS,
            }),
            // A *fungible asset*, not a coin. Aptos has both systems and the
            // two have different entry points; sending one as the other does
            // not move a wrong amount, it aborts.
            ChainId::Aptos => Some(Asset::AptosFa {
                metadata: neko_aptos::usdt_metadata(),
                decimals: neko_aptos::USDT_DECIMALS,
            }),
            // Circle's dollar, and named by a Move *type* rather than by an
            // address. Binance sends USDC here and no USDT at all.
            ChainId::Sui => Some(Asset::SuiCoin {
                coin_type: neko_sui::USDC_TYPE,
                decimals: neko_sui::USDC_DECIMALS,
            }),
        }
    }

    /// Everything this chain holds, native coin first.
    ///
    /// One list rather than a pair, because Bitcoin has one asset and the other
    /// three have two - and a screen that assumed the count would show an empty
    /// row, or hide a real one, the moment that stopped being true.
    pub fn assets(self) -> Vec<Asset> {
        let mut v = vec![self.native()];
        v.extend(self.stable());
        v
    }

    /// The EVM parameters for this chain, when it is one.
    ///
    /// `None` for TRON, Solana and Bitcoin, which is what keeps a caller from
    /// reaching for a chain id or a USDT precision that does not apply to them.
    pub fn evm(self) -> Option<neko_evm::EvmChain> {
        match self {
            ChainId::Bsc => Some(neko_evm::BSC),
            ChainId::Ethereum => Some(neko_evm::ETHEREUM),
            ChainId::Polygon => Some(neko_evm::POLYGON),
            ChainId::Base => Some(neko_evm::BASE),
            ChainId::Arbitrum => Some(neko_evm::ARBITRUM),
            ChainId::Optimism => Some(neko_evm::OPTIMISM),
            ChainId::Avalanche => Some(neko_evm::AVALANCHE),
            ChainId::HyperEvm => Some(neko_evm::HYPER_EVM),
            ChainId::Mantle => Some(neko_evm::MANTLE),
            ChainId::Linea => Some(neko_evm::LINEA),
            ChainId::ZkSyncEra => Some(neko_evm::ZKSYNC_ERA),
            ChainId::Scroll => Some(neko_evm::SCROLL),
            ChainId::Tron
            | ChainId::Solana
            | ChainId::Bitcoin
            | ChainId::Ton
            | ChainId::Aptos
            | ChainId::Sui => None,
        }
    }

    /// Which of these chains carries this EVM chain id.
    ///
    /// The reverse of [`Self::evm`], and it exists so that turning a client
    /// back into a `ChainId` is a lookup rather than a guess. That code used to
    /// read "if it is Ethereum's id then Ethereum, otherwise BNB Chain", which
    /// answered BNB Chain for every EVM chain added after it.
    pub fn from_evm_chain_id(id: u64) -> Option<Self> {
        CHAINS
            .into_iter()
            .find(|c| c.evm().is_some_and(|e| e.chain_id == id))
    }

    pub fn native(self) -> Asset {
        match self {
            ChainId::Tron => Asset::Trx,
            ChainId::Bsc => Asset::Bnb,
            ChainId::Solana => Asset::Sol,
            ChainId::Bitcoin => Asset::Btc,
            ChainId::Ethereum => Asset::Eth,
            ChainId::Polygon => Asset::Pol,
            ChainId::Base => Asset::BaseEth,
            ChainId::Arbitrum => Asset::ArbitrumEth,
            ChainId::Optimism => Asset::OptimismEth,
            ChainId::Avalanche => Asset::AvalancheNative,
            ChainId::HyperEvm => Asset::HyperEvmNative,
            ChainId::Mantle => Asset::MantleNative,
            ChainId::Linea => Asset::LineaNative,
            ChainId::ZkSyncEra => Asset::ZkSyncEraNative,
            ChainId::Scroll => Asset::ScrollNative,
            ChainId::Ton => Asset::Gram,
            ChainId::Aptos => Asset::Apt,
            ChainId::Sui => Asset::Sui,
        }
    }

    /// What a user sees when checking a transaction.
    pub fn explorer_tx(self, id: &str) -> String {
        match self {
            ChainId::Tron => format!("https://tronscan.org/#/transaction/{id}"),
            ChainId::Bsc => format!("https://bscscan.com/tx/{id}"),
            ChainId::Solana => format!("https://solscan.io/tx/{id}"),
            ChainId::Bitcoin => format!("{}{id}", neko_btc::EXPLORER_TX),
            ChainId::Ethereum => format!("{}{id}", neko_evm::ETHEREUM.explorer_tx),
            ChainId::Polygon => format!("{}{id}", neko_evm::POLYGON.explorer_tx),
            ChainId::Base => format!("{}{id}", neko_evm::BASE.explorer_tx),
            ChainId::Arbitrum => format!("{}{id}", neko_evm::ARBITRUM.explorer_tx),
            ChainId::Optimism => format!("{}{id}", neko_evm::OPTIMISM.explorer_tx),
            ChainId::Avalanche => format!("{}{id}", neko_evm::AVALANCHE.explorer_tx),
            ChainId::HyperEvm => format!("{}{id}", neko_evm::HYPER_EVM.explorer_tx),
            ChainId::Mantle => format!("{}{id}", neko_evm::MANTLE.explorer_tx),
            ChainId::Linea => format!("{}{id}", neko_evm::LINEA.explorer_tx),
            ChainId::ZkSyncEra => format!("{}{id}", neko_evm::ZKSYNC_ERA.explorer_tx),
            ChainId::Scroll => format!("{}{id}", neko_evm::SCROLL.explorer_tx),
            ChainId::Ton => format!("{}{id}", neko_ton::EXPLORER_TX),
            ChainId::Aptos => format!("{}{id}", neko_aptos::EXPLORER_TX),
            ChainId::Sui => format!("{}{id}", neko_sui::EXPLORER_TX),
        }
    }
}

/// An address, tied to the chain it belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChainAddress {
    Tron(neko_hd::Address),
    Evm(neko_hd::EvmAddress),
    Solana(neko_hd::SolanaAddress),
    Bitcoin(neko_hd::BtcAddress),
    /// The same twenty bytes as [`ChainAddress::Evm`], and deliberately a
    /// different variant. One phrase gives one address on both EVM chains, but
    /// a *destination* is chain-specific: pasting a BNB Chain address into an
    /// Ethereum send form has to be a decision rather than a coincidence that
    /// happens to parse.
    Ethereum(neko_hd::EvmAddress),
    /// Likewise its own variant, for the same reason: the bytes are identical
    /// to the other two EVM chains' and the *chain* is what makes a
    /// destination right or wrong.
    Polygon(neko_hd::EvmAddress),
    Base(neko_hd::EvmAddress),
    Arbitrum(neko_hd::EvmAddress),
    Optimism(neko_hd::EvmAddress),
    Avalanche(neko_hd::EvmAddress),
    HyperEvm(neko_hd::EvmAddress),
    Mantle(neko_hd::EvmAddress),
    Linea(neko_hd::EvmAddress),
    ZkSyncEra(neko_hd::EvmAddress),
    Scroll(neko_hd::EvmAddress),
    Ton(neko_ton::TonAddress),
    Aptos(neko_aptos::AptosAddress),
    Sui(neko_sui::SuiAddress),
}

impl ChainAddress {
    pub fn chain(&self) -> ChainId {
        match self {
            ChainAddress::Tron(_) => ChainId::Tron,
            ChainAddress::Evm(_) => ChainId::Bsc,
            ChainAddress::Solana(_) => ChainId::Solana,
            ChainAddress::Bitcoin(_) => ChainId::Bitcoin,
            ChainAddress::Ethereum(_) => ChainId::Ethereum,
            ChainAddress::Polygon(_) => ChainId::Polygon,
            ChainAddress::Base(_) => ChainId::Base,
            ChainAddress::Arbitrum(_) => ChainId::Arbitrum,
            ChainAddress::Optimism(_) => ChainId::Optimism,
            ChainAddress::Avalanche(_) => ChainId::Avalanche,
            ChainAddress::HyperEvm(_) => ChainId::HyperEvm,
            ChainAddress::Mantle(_) => ChainId::Mantle,
            ChainAddress::Linea(_) => ChainId::Linea,
            ChainAddress::ZkSyncEra(_) => ChainId::ZkSyncEra,
            ChainAddress::Scroll(_) => ChainId::Scroll,
            ChainAddress::Ton(_) => ChainId::Ton,
            ChainAddress::Aptos(_) => ChainId::Aptos,
            ChainAddress::Sui(_) => ChainId::Sui,
        }
    }

    /// Parse text as an address *on a named chain*.
    ///
    /// The chain is a parameter rather than something sniffed from the string,
    /// so a TRON address typed into the BNB Chain send form is rejected
    /// instead of quietly reinterpreted.
    pub fn parse(chain: ChainId, s: &str) -> Result<Self, CoreError> {
        let s = s.trim();
        match chain {
            ChainId::Tron => neko_hd::Address::parse(s)
                .map(ChainAddress::Tron)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Bsc => neko_hd::EvmAddress::parse(s)
                .map(ChainAddress::Evm)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Solana => neko_hd::SolanaAddress::parse(s)
                .map(ChainAddress::Solana)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Bitcoin => neko_hd::BtcAddress::parse(s)
                .map(ChainAddress::Bitcoin)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Ethereum => neko_hd::EvmAddress::parse(s)
                .map(ChainAddress::Ethereum)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Polygon => neko_hd::EvmAddress::parse(s)
                .map(ChainAddress::Polygon)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Base => neko_hd::EvmAddress::parse(s)
                .map(ChainAddress::Base)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Arbitrum => neko_hd::EvmAddress::parse(s)
                .map(ChainAddress::Arbitrum)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Optimism => neko_hd::EvmAddress::parse(s)
                .map(ChainAddress::Optimism)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Avalanche => neko_hd::EvmAddress::parse(s)
                .map(ChainAddress::Avalanche)
                .map_err(|_| CoreError::BadAddress),
            ChainId::HyperEvm => neko_hd::EvmAddress::parse(s)
                .map(ChainAddress::HyperEvm)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Mantle => neko_hd::EvmAddress::parse(s)
                .map(ChainAddress::Mantle)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Linea => neko_hd::EvmAddress::parse(s)
                .map(ChainAddress::Linea)
                .map_err(|_| CoreError::BadAddress),
            ChainId::ZkSyncEra => neko_hd::EvmAddress::parse(s)
                .map(ChainAddress::ZkSyncEra)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Scroll => neko_hd::EvmAddress::parse(s)
                .map(ChainAddress::Scroll)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Ton => neko_ton::TonAddress::parse(s)
                .map(ChainAddress::Ton)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Aptos => neko_aptos::AptosAddress::parse(s)
                .map(ChainAddress::Aptos)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Sui => neko_sui::SuiAddress::parse(s)
                .map(ChainAddress::Sui)
                .map_err(|_| CoreError::BadAddress),
        }
    }

    /// The raw bytes stored alongside the text form, so a corrupted row can be
    /// detected. 21 bytes on TRON, 20 on EVM chains.
    pub fn as_bytes(&self) -> Vec<u8> {
        match self {
            ChainAddress::Tron(a) => a.as_bytes().to_vec(),
            ChainAddress::Evm(a) => a.as_bytes().to_vec(),
            ChainAddress::Solana(a) => a.as_bytes().to_vec(),
            // The locking script, not a key hash: it is what an incoming
            // payment is matched on, and it is what distinguishes the five
            // address types that share the same 20 bytes.
            ChainAddress::Bitcoin(a) => a.as_bytes(),
            ChainAddress::Ethereum(a) => a.as_bytes().to_vec(),
            ChainAddress::Polygon(a) => a.as_bytes().to_vec(),
            ChainAddress::Base(a) => a.as_bytes().to_vec(),
            ChainAddress::Arbitrum(a) => a.as_bytes().to_vec(),
            ChainAddress::Optimism(a) => a.as_bytes().to_vec(),
            ChainAddress::Avalanche(a) => a.as_bytes().to_vec(),
            ChainAddress::HyperEvm(a) => a.as_bytes().to_vec(),
            ChainAddress::Mantle(a) => a.as_bytes().to_vec(),
            ChainAddress::Linea(a) => a.as_bytes().to_vec(),
            ChainAddress::ZkSyncEra(a) => a.as_bytes().to_vec(),
            ChainAddress::Scroll(a) => a.as_bytes().to_vec(),
            ChainAddress::Ton(a) => a.as_bytes(),
            ChainAddress::Aptos(a) => a.as_bytes().to_vec(),
            ChainAddress::Sui(a) => a.as_bytes().to_vec(),
        }
    }

    pub fn from_bytes(chain: ChainId, b: &[u8]) -> Result<Self, CoreError> {
        match chain {
            ChainId::Tron => neko_hd::Address::from_bytes(b)
                .map(ChainAddress::Tron)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Bsc => neko_hd::EvmAddress::from_bytes(b)
                .map(ChainAddress::Evm)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Solana => neko_hd::SolanaAddress::from_bytes(b)
                .map(ChainAddress::Solana)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Bitcoin => neko_hd::BtcAddress::from_bytes(b)
                .map(ChainAddress::Bitcoin)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Ethereum => neko_hd::EvmAddress::from_bytes(b)
                .map(ChainAddress::Ethereum)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Polygon => neko_hd::EvmAddress::from_bytes(b)
                .map(ChainAddress::Polygon)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Base => neko_hd::EvmAddress::from_bytes(b)
                .map(ChainAddress::Base)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Arbitrum => neko_hd::EvmAddress::from_bytes(b)
                .map(ChainAddress::Arbitrum)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Optimism => neko_hd::EvmAddress::from_bytes(b)
                .map(ChainAddress::Optimism)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Avalanche => neko_hd::EvmAddress::from_bytes(b)
                .map(ChainAddress::Avalanche)
                .map_err(|_| CoreError::BadAddress),
            ChainId::HyperEvm => neko_hd::EvmAddress::from_bytes(b)
                .map(ChainAddress::HyperEvm)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Mantle => neko_hd::EvmAddress::from_bytes(b)
                .map(ChainAddress::Mantle)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Linea => neko_hd::EvmAddress::from_bytes(b)
                .map(ChainAddress::Linea)
                .map_err(|_| CoreError::BadAddress),
            ChainId::ZkSyncEra => neko_hd::EvmAddress::from_bytes(b)
                .map(ChainAddress::ZkSyncEra)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Scroll => neko_hd::EvmAddress::from_bytes(b)
                .map(ChainAddress::Scroll)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Ton => neko_ton::TonAddress::from_bytes(b)
                .map(ChainAddress::Ton)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Aptos => neko_aptos::AptosAddress::from_bytes(b)
                .map(ChainAddress::Aptos)
                .map_err(|_| CoreError::BadAddress),
            ChainId::Sui => neko_sui::SuiAddress::from_bytes(b)
                .map(ChainAddress::Sui)
                .map_err(|_| CoreError::BadAddress),
        }
    }

    pub fn as_tron(&self) -> Result<neko_hd::Address, CoreError> {
        match self {
            ChainAddress::Tron(a) => Ok(*a),
            _ => Err(CoreError::WrongChain),
        }
    }

    /// Any EVM chain's address. The bytes are the same shape; which chain they
    /// belong to is settled by the variant, and by the caller having asked for
    /// the right one.
    pub fn as_evm(&self) -> Result<neko_hd::EvmAddress, CoreError> {
        match self {
            ChainAddress::Evm(a)
            | ChainAddress::Ethereum(a)
            | ChainAddress::Polygon(a)
            | ChainAddress::Base(a)
            | ChainAddress::Arbitrum(a)
            | ChainAddress::Optimism(a)
            | ChainAddress::Avalanche(a)
            | ChainAddress::HyperEvm(a)
            | ChainAddress::Mantle(a)
            | ChainAddress::Linea(a)
            | ChainAddress::ZkSyncEra(a)
            | ChainAddress::Scroll(a) => Ok(*a),
            _ => Err(CoreError::WrongChain),
        }
    }

    pub fn as_solana(&self) -> Result<neko_hd::SolanaAddress, CoreError> {
        match self {
            ChainAddress::Solana(a) => Ok(*a),
            _ => Err(CoreError::WrongChain),
        }
    }

    pub fn as_aptos(&self) -> Result<neko_aptos::AptosAddress, CoreError> {
        match self {
            ChainAddress::Aptos(a) => Ok(*a),
            _ => Err(CoreError::WrongChain),
        }
    }

    pub fn as_sui(&self) -> Result<neko_sui::SuiAddress, CoreError> {
        match self {
            ChainAddress::Sui(a) => Ok(*a),
            _ => Err(CoreError::WrongChain),
        }
    }

    pub fn as_ton(&self) -> Result<neko_ton::TonAddress, CoreError> {
        match self {
            ChainAddress::Ton(a) => Ok(*a),
            _ => Err(CoreError::WrongChain),
        }
    }

    pub fn as_bitcoin(&self) -> Result<neko_hd::BtcAddress, CoreError> {
        match self {
            ChainAddress::Bitcoin(a) => Ok(*a),
            _ => Err(CoreError::WrongChain),
        }
    }
}

impl std::fmt::Display for ChainAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChainAddress::Tron(a) => write!(f, "{a}"),
            ChainAddress::Evm(a) => write!(f, "{a}"),
            ChainAddress::Solana(a) => write!(f, "{a}"),
            ChainAddress::Bitcoin(a) => write!(f, "{a}"),
            ChainAddress::Polygon(a) => write!(f, "{a}"),
            ChainAddress::Base(a) => write!(f, "{a}"),
            ChainAddress::Arbitrum(a) => write!(f, "{a}"),
            ChainAddress::Optimism(a) => write!(f, "{a}"),
            ChainAddress::Avalanche(a) => write!(f, "{a}"),
            ChainAddress::HyperEvm(a) => write!(f, "{a}"),
            ChainAddress::Mantle(a) => write!(f, "{a}"),
            ChainAddress::Linea(a) => write!(f, "{a}"),
            ChainAddress::ZkSyncEra(a) => write!(f, "{a}"),
            ChainAddress::Scroll(a) => write!(f, "{a}"),
            ChainAddress::Ethereum(a) => write!(f, "{a}"),
            ChainAddress::Ton(a) => write!(f, "{a}"),
            ChainAddress::Aptos(a) => write!(f, "{a}"),
            ChainAddress::Sui(a) => write!(f, "{a}"),
        }
    }
}

/// Something transferable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asset {
    Trx,
    Trc20 {
        contract: neko_hd::Address,
        decimals: u8,
    },
    Bnb,
    Bep20 {
        contract: neko_hd::EvmAddress,
        decimals: u8,
    },
    Sol,
    /// An SPL token. `mint` is the token's own account, not the holder's - on
    /// Solana a balance lives in a separate account derived from the two.
    SplToken {
        mint: neko_hd::SolanaAddress,
        decimals: u8,
    },
    /// The only asset on its chain: no contracts, no tokens.
    Btc,
    Eth,
    Erc20 {
        contract: neko_hd::EvmAddress,
        decimals: u8,
    },
    Gram,
    /// Polygon's coin, renamed from MATIC in September 2024.
    Pol,
    /// Base's coin, which is ETH - the same asset as [`Asset::Eth`], on a
    /// different chain. Its own variant for the same reason Polygon's token
    /// has one: `chain()` has to answer Base.
    BaseEth,
    /// A token on Base.
    BaseErc20 {
        contract: neko_hd::EvmAddress,
        decimals: u8,
    },
    /// Arbitrum's coin, which is ETH again.
    ArbitrumEth,
    /// A token on Arbitrum.
    ArbitrumErc20 {
        contract: neko_hd::EvmAddress,
        decimals: u8,
    },
    /// Optimism's coin, which is ETH for the third time.
    OptimismEth,
    /// A token on Optimism.
    OptimismErc20 {
        contract: neko_hd::EvmAddress,
        decimals: u8,
    },
    /// Avalanche's coin.
    AvalancheNative,
    /// A token on Avalanche.
    AvalancheErc20 {
        contract: neko_hd::EvmAddress,
        decimals: u8,
    },
    /// HyperEvm's coin.
    HyperEvmNative,
    /// A token on HyperEvm.
    HyperEvmErc20 {
        contract: neko_hd::EvmAddress,
        decimals: u8,
    },
    /// Mantle's coin.
    MantleNative,
    /// A token on Mantle.
    MantleErc20 {
        contract: neko_hd::EvmAddress,
        decimals: u8,
    },
    /// Linea's coin.
    LineaNative,
    /// A token on Linea.
    LineaErc20 {
        contract: neko_hd::EvmAddress,
        decimals: u8,
    },
    /// ZkSyncEra's coin.
    ZkSyncEraNative,
    /// A token on ZkSyncEra.
    ZkSyncEraErc20 {
        contract: neko_hd::EvmAddress,
        decimals: u8,
    },
    /// Scroll's coin.
    ScrollNative,
    /// A token on Scroll.
    ScrollErc20 {
        contract: neko_hd::EvmAddress,
        decimals: u8,
    },
    /// A token on Polygon. Technically an ERC-20 like Ethereum's, and
    /// deliberately not the same variant: [`Asset::Erc20`] means *Ethereum's*,
    /// and one variant for both would make `chain()` answer Ethereum for a
    /// Polygon balance - which is the quiet kind of wrong that sends a transfer
    /// to the right address on the wrong chain.
    PolygonErc20 {
        contract: neko_hd::EvmAddress,
        decimals: u8,
    },
    /// Aptos's coin.
    Apt,
    /// A fungible asset on Aptos. `metadata` is the object that identifies the
    /// asset - not a coin type, and not an ERC-20-style contract.
    AptosFa {
        metadata: neko_aptos::AptosAddress,
        decimals: u8,
    },
    /// Sui's coin.
    Sui,
    /// A coin on Sui, named by the Move type its objects hold rather than by
    /// an address.
    SuiCoin {
        coin_type: &'static str,
        decimals: u8,
    },
    /// A jetton. `master` is the token's own contract; the balance lives in a
    /// separate per-holder wallet contract derived from both.
    Jetton {
        master: neko_ton::TonAddress,
        decimals: u8,
    },
}

impl Asset {
    pub fn chain(self) -> ChainId {
        match self {
            Asset::Trx | Asset::Trc20 { .. } => ChainId::Tron,
            Asset::Bnb | Asset::Bep20 { .. } => ChainId::Bsc,
            Asset::Sol | Asset::SplToken { .. } => ChainId::Solana,
            Asset::Btc => ChainId::Bitcoin,
            Asset::Eth | Asset::Erc20 { .. } => ChainId::Ethereum,
            Asset::Pol | Asset::PolygonErc20 { .. } => ChainId::Polygon,
            Asset::BaseEth | Asset::BaseErc20 { .. } => ChainId::Base,
            Asset::ArbitrumEth | Asset::ArbitrumErc20 { .. } => ChainId::Arbitrum,
            Asset::OptimismEth | Asset::OptimismErc20 { .. } => ChainId::Optimism,
            Asset::AvalancheNative | Asset::AvalancheErc20 { .. } => ChainId::Avalanche,
            Asset::HyperEvmNative | Asset::HyperEvmErc20 { .. } => ChainId::HyperEvm,
            Asset::MantleNative | Asset::MantleErc20 { .. } => ChainId::Mantle,
            Asset::LineaNative | Asset::LineaErc20 { .. } => ChainId::Linea,
            Asset::ZkSyncEraNative | Asset::ZkSyncEraErc20 { .. } => ChainId::ZkSyncEra,
            Asset::ScrollNative | Asset::ScrollErc20 { .. } => ChainId::Scroll,
            Asset::Gram | Asset::Jetton { .. } => ChainId::Ton,
            Asset::Apt | Asset::AptosFa { .. } => ChainId::Aptos,
            Asset::Sui | Asset::SuiCoin { .. } => ChainId::Sui,
        }
    }

    pub fn decimals(self) -> u8 {
        match self {
            Asset::Trx => neko_tron::TRX_DECIMALS,
            Asset::Bnb => neko_evm::BSC.native_decimals,
            Asset::Eth => neko_evm::ETHEREUM.native_decimals,
            Asset::Pol => neko_evm::POLYGON.native_decimals,
            Asset::BaseEth => neko_evm::BASE.native_decimals,
            Asset::ArbitrumEth => neko_evm::ARBITRUM.native_decimals,
            Asset::OptimismEth => neko_evm::OPTIMISM.native_decimals,
            Asset::AvalancheNative => neko_evm::AVALANCHE.native_decimals,
            Asset::HyperEvmNative => neko_evm::HYPER_EVM.native_decimals,
            Asset::MantleNative => neko_evm::MANTLE.native_decimals,
            Asset::LineaNative => neko_evm::LINEA.native_decimals,
            Asset::ZkSyncEraNative => neko_evm::ZKSYNC_ERA.native_decimals,
            Asset::ScrollNative => neko_evm::SCROLL.native_decimals,
            Asset::Gram => neko_ton::GRAM_DECIMALS,
            Asset::Apt => neko_aptos::APT_DECIMALS,
            Asset::Sui => neko_sui::SUI_DECIMALS,
            Asset::Sol => neko_solana::SOL_DECIMALS,
            Asset::Btc => neko_btc::BTC_DECIMALS,
            Asset::Trc20 { decimals, .. }
            | Asset::Bep20 { decimals, .. }
            | Asset::SplToken { decimals, .. }
            | Asset::Erc20 { decimals, .. }
            | Asset::PolygonErc20 { decimals, .. }
            | Asset::BaseErc20 { decimals, .. }
            | Asset::ArbitrumErc20 { decimals, .. }
            | Asset::OptimismErc20 { decimals, .. }
            | Asset::AvalancheErc20 { decimals, .. }
            | Asset::HyperEvmErc20 { decimals, .. }
            | Asset::MantleErc20 { decimals, .. }
            | Asset::LineaErc20 { decimals, .. }
            | Asset::ZkSyncEraErc20 { decimals, .. }
            | Asset::ScrollErc20 { decimals, .. }
            | Asset::AptosFa { decimals, .. }
            | Asset::SuiCoin { decimals, .. }
            | Asset::Jetton { decimals, .. } => decimals,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Asset::Trx => "TRX",
            Asset::Bnb => "BNB",
            Asset::Sol => "SOL",
            Asset::Btc => "BTC",
            Asset::Eth => "ETH",
            Asset::Pol => "POL",
            Asset::BaseEth => "ETH",
            Asset::ArbitrumEth => "ETH",
            Asset::OptimismEth => "ETH",
            Asset::AvalancheNative => "AVAX",
            Asset::HyperEvmNative => "HYPE",
            Asset::MantleNative => "MNT",
            Asset::LineaNative => "ETH",
            Asset::ZkSyncEraNative => "ETH",
            Asset::ScrollNative => "ETH",
            Asset::Gram => "GRAM",
            Asset::Apt => "APT",
            Asset::Sui => "SUI",
            // Circle's dollar, like Base's and three others'.
            Asset::SuiCoin { .. } => neko_sui::USDC_SYMBOL,
            // Base's stablecoin is a different token, not a differently named
            // one - see `ChainId::stable`.
            Asset::BaseErc20 { .. } => neko_evm::BASE.stable_label,
            // Four more chains carry Circle's dollar rather than Tether's,
            // each for its own reason - see `neko_evm`. The label comes from
            // the chain, never from what a contract or a server calls itself.
            Asset::MantleErc20 { .. } => neko_evm::MANTLE.stable_label,
            Asset::LineaErc20 { .. } => neko_evm::LINEA.stable_label,
            Asset::ZkSyncEraErc20 { .. } => neko_evm::ZKSYNC_ERA.stable_label,
            // Polygon's contract calls itself USDT0. It is the same token
            // people mean by USDT and it is shown as USDT; the name the
            // contract has is what the send path checks against the chain.
            Asset::Trc20 { .. }
            | Asset::Bep20 { .. }
            | Asset::SplToken { .. }
            | Asset::Erc20 { .. }
            | Asset::PolygonErc20 { .. }
            | Asset::ArbitrumErc20 { .. }
            | Asset::OptimismErc20 { .. }
            | Asset::AvalancheErc20 { .. }
            | Asset::HyperEvmErc20 { .. }
            | Asset::ScrollErc20 { .. }
            // Aptos's contract calls itself `USDt`, with a lowercase t.
            | Asset::AptosFa { .. }
            | Asset::Jetton { .. } => "USDT",
        }
    }

    /// The contract and precision of this asset, when it is a token on an EVM
    /// chain.
    ///
    /// Exhaustive on purpose, with no `_` arm: the send path used to pick
    /// these out with a list of variants and a catch-all that answered "that
    /// asset is not on chain N". A new chain's token fell into the catch-all,
    /// so the failure arrived at the moment of sending and named the chain
    /// rather than the omission. Here the compiler asks instead.
    pub fn evm_token(self) -> Option<(neko_hd::EvmAddress, u8)> {
        match self {
            Asset::Bep20 { contract, decimals }
            | Asset::Erc20 { contract, decimals }
            | Asset::PolygonErc20 { contract, decimals }
            | Asset::BaseErc20 { contract, decimals }
            | Asset::ArbitrumErc20 { contract, decimals }
            | Asset::OptimismErc20 { contract, decimals }
            | Asset::AvalancheErc20 { contract, decimals }
            | Asset::HyperEvmErc20 { contract, decimals }
            | Asset::MantleErc20 { contract, decimals }
            | Asset::LineaErc20 { contract, decimals }
            | Asset::ZkSyncEraErc20 { contract, decimals }
            | Asset::ScrollErc20 { contract, decimals } => Some((contract, decimals)),
            Asset::Trx
            | Asset::Trc20 { .. }
            | Asset::Bnb
            | Asset::Sol
            | Asset::SplToken { .. }
            | Asset::Btc
            | Asset::Eth
            | Asset::Gram
            | Asset::Pol
            | Asset::BaseEth
            | Asset::ArbitrumEth
            | Asset::Apt
            | Asset::AptosFa { .. }
            | Asset::Sui
            | Asset::SuiCoin { .. }
            | Asset::OptimismEth
            | Asset::AvalancheNative
            | Asset::HyperEvmNative
            | Asset::MantleNative
            | Asset::LineaNative
            | Asset::ZkSyncEraNative
            | Asset::ScrollNative
            | Asset::Jetton { .. } => None,
        }
    }

    /// TRON's per-transaction fee limit, in sun.
    ///
    /// `None` off TRON, because the concept does not exist there: BNB Chain
    /// bounds a transaction with a gas limit instead, which the node estimates
    /// per call. Returning an `Option` rather than a plausible-looking zero
    /// means a caller on the wrong chain gets an error instead of a fee limit
    /// of nothing.
    pub fn tron_fee_limit(self) -> Option<i64> {
        match self {
            Asset::Trx => Some(neko_tron::FEE_LIMIT_TRX),
            // A contract call with no fee limit fails for lack of energy.
            Asset::Trc20 { .. } => Some(neko_tron::FEE_LIMIT_TRC20),
            Asset::Bnb
            | Asset::Bep20 { .. }
            | Asset::Sol
            | Asset::SplToken { .. }
            | Asset::Btc
            | Asset::Eth
            | Asset::Erc20 { .. }
            | Asset::Pol
            | Asset::PolygonErc20 { .. }
            | Asset::BaseEth
            | Asset::BaseErc20 { .. }
            | Asset::ArbitrumEth
            | Asset::ArbitrumErc20 { .. }
            | Asset::AvalancheNative
            | Asset::AvalancheErc20 { .. }
            | Asset::HyperEvmNative
            | Asset::HyperEvmErc20 { .. }
            | Asset::MantleNative
            | Asset::MantleErc20 { .. }
            | Asset::LineaNative
            | Asset::LineaErc20 { .. }
            | Asset::ZkSyncEraNative
            | Asset::ZkSyncEraErc20 { .. }
            | Asset::ScrollNative
            | Asset::ScrollErc20 { .. }
            | Asset::OptimismEth
            | Asset::OptimismErc20 { .. }
            | Asset::Gram
            | Asset::Apt
            | Asset::AptosFa { .. }
            | Asset::Sui
            | Asset::SuiCoin { .. }
            | Asset::Jetton { .. } => None,
        }
    }

    /// Also: whether the fee for moving this asset comes out of the asset
    /// itself. That is the difference between "send everything" being
    /// arithmetic and being impossible - a token's fee is paid in the chain's
    /// own coin, so the whole token balance can go, while sending the coin has
    /// to hold back enough of itself to pay for the sending.
    ///
    /// **Asked of the chain rather than listed.** This was a `matches!` over
    /// the nine coin variants, and `matches!` gets no exhaustiveness check: a
    /// new chain's coin falls through as `false` and the crate still compiles.
    /// Optimism's did, and the two things that answer to this flag are the two
    /// that decide whether "send everything" is payable - `hold_back_fee`,
    /// which then reserves nothing, and `native_needed`, which then leaves the
    /// amount out of the balance check. Both fail in the direction that offers
    /// a transfer the chain will refuse.
    pub fn is_native(self) -> bool {
        self == self.chain().native()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugs_round_trip_and_are_stable() {
        for c in CHAINS {
            assert_eq!(ChainId::from_slug(c.slug()), Some(c));
        }
        // Pinned: these are written into the database, so renaming one
        // orphans every row that already carries it.
        assert_eq!(ChainId::Tron.slug(), "tron");
        assert_eq!(ChainId::Bsc.slug(), "bsc");
        assert_eq!(ChainId::Solana.slug(), "solana");
        assert_eq!(ChainId::Bitcoin.slug(), "bitcoin");
        assert_eq!(ChainId::Ethereum.slug(), "ethereum");
        assert_eq!(ChainId::from_slug("dogecoin"), None);
    }

    /// One token name, four chains, and not one precision between them.
    ///
    /// The difference that will bite somebody if it ever regresses: eighteen
    /// on BNB Chain and six everywhere else, which is a factor of a million
    /// million between two rows that both say "USDT".
    #[test]
    fn usdt_has_a_different_precision_on_each_chain() {
        assert_eq!(ChainId::Tron.stable().unwrap().decimals(), 6);
        assert_eq!(ChainId::Bsc.stable().unwrap().decimals(), 18);
        assert_eq!(ChainId::Solana.stable().unwrap().decimals(), 6);
        assert_eq!(ChainId::Ethereum.stable().unwrap().decimals(), 6);
        assert_eq!(
            ChainId::Bitcoin.stable(),
            None,
            "there is no USDT on Bitcoin"
        );

        assert_eq!(ChainId::Tron.native_decimals(), 6);
        assert_eq!(ChainId::Bsc.native_decimals(), 18);
        assert_eq!(ChainId::Ethereum.native_decimals(), 18);
        assert_eq!(ChainId::Solana.native_decimals(), 9);
        assert_eq!(ChainId::Bitcoin.native_decimals(), 8);

        // And the two EVM chains are not the same contract, which is the other
        // half of the same mistake.
        assert_ne!(
            ChainId::Bsc.stable().unwrap(),
            ChainId::Ethereum.stable().unwrap()
        );
    }

    /// Both EVM chains derive the same address, and that is correct - one
    /// phrase, one coin type, the address every EVM wallet shows. What must
    /// *not* be shared is the chain id, which is what makes a signature valid
    /// on one and useless on the other.
    #[test]
    fn the_evm_chains_share_an_address_and_not_a_chain_id() {
        assert_eq!(ChainId::Bsc.coin_type(), ChainId::Ethereum.coin_type());
        assert_ne!(
            ChainId::Bsc.evm().unwrap().chain_id,
            ChainId::Ethereum.evm().unwrap().chain_id
        );
        assert_eq!(ChainId::Bsc.evm().unwrap().chain_id, 56);
        assert_eq!(ChainId::Ethereum.evm().unwrap().chain_id, 1);
        // And the non-EVM chains have no such parameters to reach for.
        for c in [ChainId::Tron, ChainId::Solana, ChainId::Bitcoin] {
            assert!(c.evm().is_none(), "{c:?} is not an EVM chain");
        }
    }

    /// An address for one chain must never parse as an address for the other.
    /// This is the check that stops funds going somewhere unrecoverable.
    #[test]
    fn addresses_do_not_cross_chains() {
        const TRON: &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
        const EVM: &str = "0x55d398326f99059fF775485246999027B3197955";

        assert!(ChainAddress::parse(ChainId::Tron, TRON).is_ok());
        assert!(ChainAddress::parse(ChainId::Bsc, EVM).is_ok());

        assert!(
            ChainAddress::parse(ChainId::Bsc, TRON).is_err(),
            "a TRON address was accepted as a BNB Chain address"
        );
        assert!(
            ChainAddress::parse(ChainId::Tron, EVM).is_err(),
            "an EVM address was accepted as a TRON address"
        );
    }

    #[test]
    fn addresses_round_trip_through_bytes() {
        for (chain, text) in [
            (ChainId::Tron, "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"),
            (ChainId::Bsc, "0x55d398326f99059fF775485246999027B3197955"),
        ] {
            let a = ChainAddress::parse(chain, text).unwrap();
            assert_eq!(a.to_string(), text);
            assert_eq!(ChainAddress::from_bytes(chain, &a.as_bytes()).unwrap(), a);
            assert_eq!(a.chain(), chain);
        }
    }

    #[test]
    fn assets_know_their_chain() {
        assert_eq!(ChainId::Tron.stable().unwrap().chain(), ChainId::Tron);
        assert_eq!(ChainId::Bsc.stable().unwrap().chain(), ChainId::Bsc);
        assert_eq!(ChainId::Bsc.native().symbol(), "BNB");
        assert!(ChainId::Bsc.native().is_native());
        assert!(!ChainId::Bsc.stable().unwrap().is_native());
    }

    /// Every chain's coin knows it is one, and no token does.
    ///
    /// Written over `CHAINS` because the single-chain version above passed for
    /// as long as `is_native` was a hand-written list that had gone stale:
    /// BNB was on it, and Optimism's ether was not. What that flag decides is
    /// whether "send everything" holds back the fee, so a coin missing from it
    /// offers the user a transfer the chain refuses.
    #[test]
    fn every_chains_coin_is_native_and_no_token_is() {
        for c in CHAINS {
            assert!(
                c.native().is_native(),
                "{c:?}: its own coin is not recognised as one"
            );
            assert_eq!(c.native().chain(), c, "{c:?}: coin belongs elsewhere");
            if let Some(t) = c.stable() {
                assert!(!t.is_native(), "{c:?}: its token claims to be the coin");
                assert_eq!(t.chain(), c, "{c:?}: token belongs elsewhere");
            }
        }
        // Every asset on every chain, so a chain that grows a third one is
        // covered too.
        for c in CHAINS {
            let natives = c.assets().iter().filter(|a| a.is_native()).count();
            assert_eq!(natives, 1, "{c:?}: exactly one asset pays the fees");
        }
    }
}
