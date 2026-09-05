#[derive(Debug, thiserror::Error)]
pub enum AptosError {
    #[error("that is not an Aptos address")]
    BadAddress,
    #[error("the amount is too large for this chain")]
    AmountTooLarge,
    #[error("the node rejected the request: {0}")]
    Rpc(String),
    #[error("the node's reply was not what this wallet expects: {0}")]
    BadReply(String),
}
