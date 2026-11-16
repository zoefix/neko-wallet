//! Servers to fall back on when toncenter is refusing: none.
//!
//! No keyless, reputable host was found that speaks toncenter's v2 API: dRPC's
//! keyless tier does not serve TON, and the others either want a key or
//! answer a different API, which is no fallback. TON relies on pacing alone -
//! one request a second without a key, toncenter's published limit - and a
//! key raises it.

pub const FALLBACKS: &[&str] = &[];
