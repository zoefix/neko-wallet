use thiserror::Error;

#[derive(Debug, Error)]
pub enum EvmError {
    #[error("the node rejected the request: {0}")]
    Rpc(String),
    #[error("the node's reply was not in the expected shape: {0}")]
    BadReply(String),
    #[error("network: {0}")]
    Network(String),
    #[error("signing failed")]
    Signing,
    #[error("{0}")]
    Address(#[from] neko_hd::HdError),
    #[error("amount does not fit in 256 bits")]
    AmountTooLarge,
}
