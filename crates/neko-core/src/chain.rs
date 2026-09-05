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
}

pub const CHAINS: [ChainId; 7] = [
    ChainId::Tron,
    ChainId::Bsc,
    ChainId::Solana,
    ChainId::Bitcoin,
    ChainId::Ethereum,
    ChainId::Ton,
    ChainId::Polygon,
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
            ChainId::Ethereum | ChainId::Polygon => neko_hd::derive::COIN_TYPE_EVM,
            ChainId::Ton => neko_hd::COIN_TYPE_TON,
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
            ChainId::Ton => neko_ton::GRAM_DECIMALS,
            ChainId::Solana => neko_solana::SOL_DECIMALS,
            ChainId::Bitcoin => neko_btc::BTC_DECIMALS,
        }
    }

    /// The stablecoin this wallet knows about on each chain.
    ///
    /// Same name, different precision: 6 decimals on TRON, 18 on BNB Chain.
    /// Treating one like the other is a factor of a million million, which is
    /// why the number travels with the asset rather than living in a constant.
    /// `None` on a chain with no such token.
    ///
    /// Bitcoin is the first: it carries one asset and no contracts, so a screen
    /// that assumed two assets per chain would show an empty row where nothing
    /// exists.
    pub fn usdt(self) -> Option<Asset> {
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
                contract: neko_evm::BSC.usdt_address(),
                decimals: neko_evm::BSC.usdt_decimals,
            }),
            // A different contract *and* a different precision from BNB
            // Chain's: six decimals here, eighteen there.
            ChainId::Ethereum => Some(Asset::Erc20 {
                contract: neko_evm::ETHEREUM.usdt_address(),
                decimals: neko_evm::ETHEREUM.usdt_decimals,
            }),
            // Six decimals like Ethereum's, and a contract that calls itself
            // USDT0 - see `neko_evm::POLYGON`.
            ChainId::Polygon => Some(Asset::PolygonErc20 {
                contract: neko_evm::POLYGON.usdt_address(),
                decimals: neko_evm::POLYGON.usdt_decimals,
            }),
            ChainId::Ton => Some(Asset::Jetton {
                master: neko_ton::usdt_master(),
                decimals: neko_ton::USDT_DECIMALS,
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
        v.extend(self.usdt());
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
            ChainId::Tron | ChainId::Solana | ChainId::Bitcoin | ChainId::Ton => None,
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
            ChainId::Ton => Asset::Gram,
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
            ChainId::Ton => format!("{}{id}", neko_ton::EXPLORER_TX),
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
    Ton(neko_ton::TonAddress),
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
            ChainAddress::Ton(_) => ChainId::Ton,
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
            ChainId::Ton => neko_ton::TonAddress::parse(s)
                .map(ChainAddress::Ton)
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
            ChainAddress::Ton(a) => a.as_bytes(),
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
            ChainId::Ton => neko_ton::TonAddress::from_bytes(b)
                .map(ChainAddress::Ton)
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
            ChainAddress::Evm(a) | ChainAddress::Ethereum(a) | ChainAddress::Polygon(a) => Ok(*a),
            _ => Err(CoreError::WrongChain),
        }
    }

    pub fn as_solana(&self) -> Result<neko_hd::SolanaAddress, CoreError> {
        match self {
            ChainAddress::Solana(a) => Ok(*a),
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
            ChainAddress::Ethereum(a) => write!(f, "{a}"),
            ChainAddress::Ton(a) => write!(f, "{a}"),
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
    /// A token on Polygon. Technically an ERC-20 like Ethereum's, and
    /// deliberately not the same variant: [`Asset::Erc20`] means *Ethereum's*,
    /// and one variant for both would make `chain()` answer Ethereum for a
    /// Polygon balance - which is the quiet kind of wrong that sends a transfer
    /// to the right address on the wrong chain.
    PolygonErc20 {
        contract: neko_hd::EvmAddress,
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
            Asset::Gram | Asset::Jetton { .. } => ChainId::Ton,
        }
    }

    pub fn decimals(self) -> u8 {
        match self {
            Asset::Trx => neko_tron::TRX_DECIMALS,
            Asset::Bnb => neko_evm::BSC.native_decimals,
            Asset::Eth => neko_evm::ETHEREUM.native_decimals,
            Asset::Pol => neko_evm::POLYGON.native_decimals,
            Asset::Gram => neko_ton::GRAM_DECIMALS,
            Asset::Sol => neko_solana::SOL_DECIMALS,
            Asset::Btc => neko_btc::BTC_DECIMALS,
            Asset::Trc20 { decimals, .. }
            | Asset::Bep20 { decimals, .. }
            | Asset::SplToken { decimals, .. }
            | Asset::Erc20 { decimals, .. }
            | Asset::PolygonErc20 { decimals, .. }
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
            Asset::Gram => "GRAM",
            // Only USDT is known so far; when a second token is added this
            // has to carry its symbol rather than assume.
            Asset::Trc20 { .. }
            | Asset::Bep20 { .. }
            | Asset::SplToken { .. }
            | Asset::Erc20 { .. }
            // Polygon's contract calls itself USDT0. It is the same token
            // people mean by USDT and it is shown as USDT; the name the
            // contract has is what the send path checks against the chain.
            | Asset::PolygonErc20 { .. }
            | Asset::Jetton { .. } => "USDT",
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
            | Asset::Gram
            | Asset::Jetton { .. } => None,
        }
    }

    /// Also: whether the fee for moving this asset comes out of the asset
    /// itself. That is the difference between "send everything" being
    /// arithmetic and being impossible - a token's fee is paid in the chain's
    /// own coin, so the whole token balance can go, while sending the coin has
    /// to hold back enough of itself to pay for the sending.
    pub fn is_native(self) -> bool {
        matches!(
            self,
            Asset::Trx
                | Asset::Bnb
                | Asset::Sol
                | Asset::Btc
                | Asset::Eth
                | Asset::Pol
                | Asset::Gram
        )
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
        assert_eq!(ChainId::Tron.usdt().unwrap().decimals(), 6);
        assert_eq!(ChainId::Bsc.usdt().unwrap().decimals(), 18);
        assert_eq!(ChainId::Solana.usdt().unwrap().decimals(), 6);
        assert_eq!(ChainId::Ethereum.usdt().unwrap().decimals(), 6);
        assert_eq!(ChainId::Bitcoin.usdt(), None, "there is no USDT on Bitcoin");

        assert_eq!(ChainId::Tron.native_decimals(), 6);
        assert_eq!(ChainId::Bsc.native_decimals(), 18);
        assert_eq!(ChainId::Ethereum.native_decimals(), 18);
        assert_eq!(ChainId::Solana.native_decimals(), 9);
        assert_eq!(ChainId::Bitcoin.native_decimals(), 8);

        // And the two EVM chains are not the same contract, which is the other
        // half of the same mistake.
        assert_ne!(
            ChainId::Bsc.usdt().unwrap(),
            ChainId::Ethereum.usdt().unwrap()
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
        assert_eq!(ChainId::Tron.usdt().unwrap().chain(), ChainId::Tron);
        assert_eq!(ChainId::Bsc.usdt().unwrap().chain(), ChainId::Bsc);
        assert_eq!(ChainId::Bsc.native().symbol(), "BNB");
        assert!(ChainId::Bsc.native().is_native());
        assert!(!ChainId::Bsc.usdt().unwrap().is_native());
    }
}
