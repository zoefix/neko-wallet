//! The façade the TUI talks to. Owns the unlocked session and the vault
//! lifecycle: first-run setup, unlock, lock, and password change.

pub mod amount;
pub mod error;
pub mod session;
pub mod transfer;
pub mod wallets;

pub use amount::Amount;
pub use error::CoreError;
pub use session::{Session, VaultFile};
pub use transfer::{Asset, TransferRequest};
pub use wallets::{CachedAssets, NewWalletSpec, WalletView};
