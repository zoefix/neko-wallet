//! The Open Network, whose coin is Gram.
//!
//! The token was renamed from Toncoin to Gram on 15 June 2026, returning to the
//! name it had in Telegram's 2018 whitepaper. Only the ticker changed:
//! addresses, balances and contracts are untouched, and the network is still
//! called TON. So the chain here is TON and the asset is GRAM.
//!
//! Structurally this is the least like the others. There is no account you own
//! a private key to: **a wallet is a smart contract**, its address is the hash
//! of its own initial code and storage, and sending anything means signing an
//! external message that asks that contract to act. The first transfer out of a
//! wallet has to carry the contract's code, because until then the address
//! holds a balance and nothing else.
//!
//! Tokens are contracts too. Your USDT does not live in a ledger keyed by your
//! address; it lives in a *jetton wallet* contract of its own, one per holder
//! per token, at an address derived from both.

pub mod address;
pub mod boc;
pub mod cell;
pub mod error;
pub mod wallet;

pub use address::TonAddress;
pub use error::TonError;
