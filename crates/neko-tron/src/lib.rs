//! TRON transaction construction, signing, and chain access.

pub mod chain_consts;
pub mod client;
pub mod error;
pub mod history;
pub mod pb;
pub mod tx;

pub use chain_consts::{usdt_address, DEFAULT_URL, EXPLORER_TX, USDT_CONTRACT};
pub use client::{EnergyEstimate, Prices, Resources, TronGrid};
pub use error::TxError;
pub use history::{Direction, HistoryEntry, TxStatus};
pub use neko_hd::Address;
pub use tx::{ContractType, SignedTx, TxParams};

/// 1 TRX = 1e6 sun.
pub const SUN_PER_TRX: i64 = 1_000_000;
pub const TRX_DECIMALS: u8 = 6;
pub const USDT_DECIMALS: u8 = 6;
/// Blocks before a transaction is treated as final.
pub const CONFIRMATION_BLOCKS: u64 = 19;
/// A contract call needs a fee limit or it fails for lack of energy.
pub const FEE_LIMIT_TRC20: i64 = 100 * SUN_PER_TRX;
pub const FEE_LIMIT_TRX: i64 = 0;
