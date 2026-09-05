use thiserror::Error;

#[derive(Debug, Error)]
pub enum CoreError {
    /// The only error a failed unlock may produce. It must not distinguish
    /// "wrong email" from "wrong password" from "wrong KDF profile" — any of
    /// those distinctions is an oracle.
    #[error("email or password is incorrect")]
    WrongCredentials,
    #[error("password too weak")]
    WeakPassword(Vec<neko_vault::password::Warning>),
    #[error("vault is locked")]
    Locked,
    #[error("this wallet has no recovery phrase (it was imported as a private key)")]
    NoMnemonic,
    #[error("that mnemonic is not valid BIP39")]
    BadMnemonic,
    #[error("that is not a valid 32-byte private key")]
    BadPrivateKey,
    #[error("{0}")]
    BadAmount(#[from] crate::amount::AmountError),
    #[error("that is not a valid address for this chain")]
    BadAddress,
    /// An address, key or asset from one chain reached code meant for another.
    /// The types keep these apart, so this is unreachable through the
    /// interface; it exists so the impossible case is loud rather than
    /// silently wrong.
    #[error("that address belongs to a different chain")]
    WrongChain,
    #[error("{0}")]
    Evm(#[from] neko_evm::EvmError),
    #[error("{0}")]
    Solana(#[from] neko_solana::SolanaError),
    #[error("{0}")]
    Btc(#[from] neko_btc::BtcError),
    #[error("{0}")]
    Ton(#[from] neko_ton::TonError),
    /// The fee a transaction would actually pay is not the one that was quoted.
    /// On a UTXO chain the fee is implicit, so this catches the difference
    /// between "the change output is missing" and "a miner received it".
    #[error("this transaction would pay {actual} satoshis in fees, not the {quoted} quoted - refusing to sign")]
    FeeMismatch { quoted: u64, actual: u64 },
    /// TON only, and only because TON can tell: the address is the hash of the
    /// contract that holds the public key, so a key that does not belong to the
    /// address it is signing for is provable rather than merely suspected.
    #[error("this key derives {derived}, not {expected} - refusing to sign")]
    WrongSigningKey { expected: String, derived: String },
    /// The token a quote was prepared for is not the token being signed for.
    #[error("this transfer was quoted against {quoted}, not {asked} - refusing to sign")]
    WrongToken { quoted: String, asked: String },
    #[error("insufficient {asset}: you have {have}, this needs {need}")]
    Insufficient {
        asset: String,
        have: String,
        need: String,
    },
    #[error(transparent)]
    Tx(#[from] neko_tron::TxError),
    #[error(transparent)]
    Hd(#[from] neko_hd::HdError),
    #[error(transparent)]
    Vault(#[from] neko_vault::VaultError),
    #[error(transparent)]
    Store(#[from] neko_store::StoreError),
    #[error(transparent)]
    Crypto(#[from] neko_crypto::CryptoError),
}
