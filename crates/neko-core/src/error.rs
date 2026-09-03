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
    #[error("that is not a valid TRON address")]
    BadAddress,
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
