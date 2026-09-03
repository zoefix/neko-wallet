use thiserror::Error;

#[derive(Debug, Error)]
pub enum TxError {
    #[error("amount must be positive")]
    NonPositiveAmount,
    #[error("amount exceeds int64")]
    AmountTooLarge,
    #[error("block id must be 32 bytes, got {0}")]
    BadBlockId(usize),
    #[error("timestamp must be positive")]
    BadTimestamp,
    #[error("expiration must be later than the timestamp")]
    BadExpiration,
    #[error("a contract call requires a positive fee limit, or it fails for lack of energy")]
    MissingFeeLimit,
    #[error("signing failed")]
    Sign,
    /// The signature recovers to a different address than the one we intended
    /// to spend from. Refuse to broadcast.
    #[error("signature self-check failed: recovered {got}, expected {want}")]
    SelfCheck { got: String, want: String },
    #[error(transparent)]
    Hd(#[from] neko_hd::HdError),
}
