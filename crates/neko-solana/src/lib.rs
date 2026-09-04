//! Solana.
//!
//! Sharing almost nothing with the other two chains, which is why it is its own
//! crate rather than a variant inside one of them. Four differences shape
//! everything here, and each is a way to lose money if it is assumed:
//!
//! * **Ed25519, not secp256k1.** Different signatures, different derivation
//!   (SLIP-0010, hardened at every level), and an address that is the bare
//!   public key with no checksum to catch a typo.
//! * **Token balances live in their own accounts.** Sending USDT to somebody
//!   who has never held it means creating an account and paying its rent -
//!   about forty times a plain transfer's fee, charged to the sender, with no
//!   equivalent on TRON or BNB Chain.
//! * **Blockhashes expire in about a minute.** A transaction signed against a
//!   stale one is simply dropped. Nothing here may carry a blockhash from a fee
//!   quote to a signature the way TRON's block reference is carried.
//! * **Emptying an account is all-or-nothing.** Leaving a balance below the
//!   rent-exempt minimum is rejected outright; leaving exactly zero is fine.

pub mod chain_consts;
pub mod client;
pub mod error;
pub mod pda;
pub mod shortvec;
pub mod tx;

pub use chain_consts::*;
pub use error::SolanaError;
pub use pda::associated_token_address;
