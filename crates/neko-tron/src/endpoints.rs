//! Public TRON nodes to fall back on when TronGrid is refusing.
//!
//! Keyless TronGrid allows three requests a second per IP and suspends for
//! five seconds past that, which one wallet-list refresh can exceed on its
//! own. These stand behind it, and only when no node is configured: a
//! configured node is the only one ever contacted.
//!
//! Each entry was checked against every `/wallet/*` call this crate makes -
//! account, constant call, energy estimate, resources, chain parameters,
//! block reference and broadcast - before it was added. None of them serves
//! TronGrid's v1 history API, so history never falls back, and none of them
//! is ever sent a TronGrid key.

pub const FALLBACKS: &[&str] = &["https://tron-rpc.publicnode.com"];
