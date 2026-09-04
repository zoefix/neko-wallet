use thiserror::Error;

#[derive(Debug, Error)]
pub enum BtcError {
    #[error("the server rejected the request: {0}")]
    Rpc(String),
    #[error("the server's reply was not in the expected shape: {0}")]
    BadReply(String),
    #[error("network: {0}")]
    Network(String),
    #[error("{0}")]
    Address(#[from] neko_hd::HdError),
    #[error("amount does not fit in 64 bits")]
    AmountTooLarge,
    /// Not "insufficient funds": on a UTXO chain the fee depends on how many
    /// coins are selected, so the shortfall is only known once selection has
    /// run.
    #[error("not enough coins: {needed} satoshis needed, {available} available")]
    NotEnough { needed: u64, available: u64 },
    #[error("this wallet can only sign for its own P2WPKH inputs")]
    UnsignableInput,
    #[error("a transaction must pay something")]
    NoOutputs,
    /// Below this, an output costs more to spend than it is worth, and the
    /// network will not relay it.
    #[error("{0} satoshis is below the dust threshold of {1}")]
    Dust(u64, u64),
}
