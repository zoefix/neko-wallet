use thiserror::Error;

#[derive(Debug, Error)]
pub enum TonError {
    #[error("the node rejected the request: {0}")]
    Rpc(String),
    #[error("the node's reply was not in the expected shape: {0}")]
    BadReply(String),
    #[error("network: {0}")]
    Network(String),
    #[error("{0}")]
    Address(#[from] neko_hd::HdError),
    #[error("amount does not fit in 128 bits")]
    AmountTooLarge,

    // --- cells. A cell is bounded in both directions, and a builder that
    // overflows either has been asked to encode something that cannot exist.
    #[error("a cell holds at most 1023 bits, tried to write {0}")]
    CellOverflow(usize),
    #[error("a cell holds at most 4 references, tried to add a fifth")]
    TooManyRefs,
    #[error("malformed bag of cells: {0}")]
    BadBoc(String),
    #[error("this wallet has no {0} to send")]
    NoJettonWallet(String),
}
