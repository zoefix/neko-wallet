//! Public Solana RPC endpoints to fall back on: none yet.
//!
//! An entry has to serve every method this crate calls. PublicNode's cluster
//! endpoint comes closest and still fails two: it answers 403 to
//! `getTokenAccountBalance` without a personal token, and refuses a batch
//! with more than one `getTransaction`, which is how history reads. dRPC's
//! keyless tier does not serve Solana at all. Until one passes, the public
//! cluster is paced instead - 100 requests per ten seconds per IP.

pub const FALLBACKS: &[&str] = &[];
