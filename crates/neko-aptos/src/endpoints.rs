//! Aptos fullnodes to fall back on when the default is refusing.
//!
//! Used only when no node is configured: a configured node is the only one
//! ever contacted. Each entry was checked against every call this crate
//! makes, namely ledger info, accounts, the gas estimate, view functions,
//! simulation and submission, before it was added. The indexer that history
//! reads has no equivalent elsewhere and does not fall back.
//!
//! The second is Aptos Labs' API gateway: the same operator as the default
//! and possibly the same allowance, but a different front door, which is
//! what helps when one of them is the one having trouble.

pub const FALLBACKS: &[&str] = &[
    "https://aptos-rest.publicnode.com/v1",
    "https://api.mainnet.aptoslabs.com/v1",
];
