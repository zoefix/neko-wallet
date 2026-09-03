//! The façade the TUI talks to. Owns the unlocked session and the vault
//! lifecycle: first-run setup, unlock, lock, and password change.

pub mod amount;
pub mod chain;
pub mod error;
pub mod session;
pub mod transfer;
pub mod value;
pub mod wallets;

pub use amount::Amount;
pub use chain::{Asset, ChainAddress, ChainId, CHAINS};
pub use error::CoreError;
pub use session::{Session, VaultFile};
pub use transfer::{ChainTxParams, SignedTransfer, TransferRequest};
pub use value::{Prices, PRICE_SCALE};
pub use wallets::{CachedAssets, NewWalletSpec, WalletView};
