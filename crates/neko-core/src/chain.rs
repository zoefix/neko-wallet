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
}

pub const CHAINS: [ChainId; 4] = [
    ChainId::Tron,
    ChainId::Bsc,
    ChainId::Solana,
    ChainId::Bitcoin,
];

impl ChainId {
    /// Stored in the database and in settings; never shown to a user.
    pub fn slug(self) -> &'static str {
        match self {
            ChainId::Tron => "tron",
            ChainId::Bsc => "bsc",
            ChainId::Solana => "solana",
            ChainId::Bitcoin => "bitcoin",
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
        }
    }

    pub fn native_symbol(self) -> &'static str {
        match self {
            ChainId::Tron => "TRX",
            ChainId::Bsc => "BNB",
            ChainId::Solana => "SOL",
            ChainId::Bitcoin => "BTC",
        }
    }

    /// TRX has 6, BNB has 18. Nothing in this program may assume a number of
    /// decimals; it always comes from the chain or the token.
    pub fn native_decimals(self) -> u8 {
        match self {
            ChainId::Tron => neko_tron::TRX_DECIMALS,
            ChainId::Bsc => neko_evm::BNB_DECIMALS,
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
                contract: neko_evm::usdt_address(),
                decimals: neko_evm::USDT_DECIMALS,
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

    pub fn native(self) -> Asset {
        match self {
            ChainId::Tron => Asset::Trx,
            ChainId::Bsc => Asset::Bnb,
            ChainId::Solana => Asset::Sol,
            ChainId::Bitcoin => Asset::Btc,
        }
    }

    /// What a user sees when checking a transaction.
    pub fn explorer_tx(self, id: &str) -> String {
        match self {
            ChainId::Tron => format!("https://tronscan.org/#/transaction/{id}"),
            ChainId::Bsc => format!("https://bscscan.com/tx/{id}"),
            ChainId::Solana => format!("https://solscan.io/tx/{id}"),
            ChainId::Bitcoin => format!("{}{id}", neko_btc::EXPLORER_TX),
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
}

impl ChainAddress {
    pub fn chain(&self) -> ChainId {
        match self {
            ChainAddress::Tron(_) => ChainId::Tron,
            ChainAddress::Evm(_) => ChainId::Bsc,
            ChainAddress::Solana(_) => ChainId::Solana,
            ChainAddress::Bitcoin(_) => ChainId::Bitcoin,
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
        }
    }

    pub fn as_tron(&self) -> Result<neko_hd::Address, CoreError> {
        match self {
            ChainAddress::Tron(a) => Ok(*a),
            _ => Err(CoreError::WrongChain),
        }
    }

    pub fn as_evm(&self) -> Result<neko_hd::EvmAddress, CoreError> {
        match self {
            ChainAddress::Evm(a) => Ok(*a),
            _ => Err(CoreError::WrongChain),
        }
    }

    pub fn as_solana(&self) -> Result<neko_hd::SolanaAddress, CoreError> {
        match self {
            ChainAddress::Solana(a) => Ok(*a),
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
}

impl Asset {
    pub fn chain(self) -> ChainId {
        match self {
            Asset::Trx | Asset::Trc20 { .. } => ChainId::Tron,
            Asset::Bnb | Asset::Bep20 { .. } => ChainId::Bsc,
            Asset::Sol | Asset::SplToken { .. } => ChainId::Solana,
            Asset::Btc => ChainId::Bitcoin,
        }
    }

    pub fn decimals(self) -> u8 {
        match self {
            Asset::Trx => neko_tron::TRX_DECIMALS,
            Asset::Bnb => neko_evm::BNB_DECIMALS,
            Asset::Sol => neko_solana::SOL_DECIMALS,
            Asset::Btc => neko_btc::BTC_DECIMALS,
            Asset::Trc20 { decimals, .. }
            | Asset::Bep20 { decimals, .. }
            | Asset::SplToken { decimals, .. } => decimals,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Asset::Trx => "TRX",
            Asset::Bnb => "BNB",
            Asset::Sol => "SOL",
            Asset::Btc => "BTC",
            // Only USDT is known so far; when a second token is added this
            // has to carry its symbol rather than assume.
            Asset::Trc20 { .. } | Asset::Bep20 { .. } | Asset::SplToken { .. } => "USDT",
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
            Asset::Bnb | Asset::Bep20 { .. } | Asset::Sol | Asset::SplToken { .. } | Asset::Btc => {
                None
            }
        }
    }

    /// Also: whether the fee for moving this asset comes out of the asset
    /// itself. That is the difference between "send everything" being
    /// arithmetic and being impossible - a token's fee is paid in the chain's
    /// own coin, so the whole token balance can go, while sending the coin has
    /// to hold back enough of itself to pay for the sending.
    pub fn is_native(self) -> bool {
        matches!(self, Asset::Trx | Asset::Bnb | Asset::Sol | Asset::Btc)
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
        // Pinned: these are written into the database.
        assert_eq!(ChainId::Tron.slug(), "tron");
        assert_eq!(ChainId::Bsc.slug(), "bsc");
        assert_eq!(ChainId::from_slug("ethereum"), None);
    }

    /// The difference that will bite somebody if it ever regresses.
    #[test]
    fn usdt_has_a_different_precision_on_each_chain() {
        assert_eq!(ChainId::Tron.usdt().unwrap().decimals(), 6);
        assert_eq!(ChainId::Bsc.usdt().unwrap().decimals(), 18);
        assert_eq!(ChainId::Tron.native_decimals(), 6);
        assert_eq!(ChainId::Bsc.native_decimals(), 18);
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
