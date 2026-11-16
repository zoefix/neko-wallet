//! Sui GraphQL endpoints to fall back on: none.
//!
//! No other keyless host serves Sui's GraphQL schema: the old beta hosts
//! (`graphql-beta.mainnet.sui.io`, `sui-mainnet.mystenlabs.com`) refuse the
//! connection, and PublicNode and dRPC answer 404 at `/graphql`.

pub const FALLBACKS: &[&str] = &[];
