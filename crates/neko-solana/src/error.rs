use thiserror::Error;

#[derive(Debug, Error)]
pub enum SolanaError {
    #[error("the node rejected the request: {0}")]
    Rpc(String),
    #[error("the node's reply was not in the expected shape: {0}")]
    BadReply(String),
    #[error("network: {0}")]
    Network(String),
    #[error("{0}")]
    Address(#[from] neko_hd::HdError),
    #[error("amount does not fit in 64 bits")]
    AmountTooLarge,
    /// Solana caps a message at one packet. Hitting this means the transaction
    /// was built wrong, not that the user asked for too much.
    #[error("the transaction is {0} bytes, over the {1}-byte limit")]
    MessageTooLong(usize, usize),
    #[error("a transaction may reference at most 256 accounts, got {0}")]
    TooManyAccounts(usize),
    /// Every 32-byte string is a potential address, but only some are on the
    /// Ed25519 curve. Deriving a token account walks bumps until one is not.
    #[error("no program-derived address exists for these seeds")]
    NoProgramAddress,
    #[error("this account holds no {0}, so there is nothing to send")]
    NoTokenAccount(String),
}
