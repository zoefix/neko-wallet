#[derive(Debug, thiserror::Error)]
pub enum SuiError {
    #[error("that is not a Sui address")]
    BadAddress,
    #[error("that is not a Sui object id")]
    BadObjectId,
    #[error("the amount is too large for this chain")]
    AmountTooLarge,
    #[error("there is not enough of that coin, or it is spread across too many objects")]
    NotEnoughCoins,
    #[error("the node rejected the request: {0}")]
    Rpc(String),
    #[error("the node's reply was not what this wallet expects: {0}")]
    BadReply(String),
}
