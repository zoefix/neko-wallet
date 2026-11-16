//! Esplora instances to fall back on when the default is refusing.
//!
//! Used only when no Esplora is configured: a configured one is the only one
//! ever contacted. Each entry was checked against every path this crate uses,
//! namely UTXOs, fee estimates, the tip, an address's transactions and the
//! broadcast, before it was added.
//!
//! mempool.space refuses to list more than 500 coins for one address, which
//! no wallet of this kind comes near; its fee estimates arrive with a 203
//! and a deprecation notice, and are the same shape Esplora's are.

pub const FALLBACKS: &[&str] = &["https://mempool.space/api"];
