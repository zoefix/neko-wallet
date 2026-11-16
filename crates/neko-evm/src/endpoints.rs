//! Public nodes to fall back on, per chain, when the default is refusing.
//!
//! Used only when no node is configured for that chain: a configured node is
//! the only one ever contacted. Each entry was checked against every call
//! [`crate::client::Rpc`] makes - chain id, balance, nonce, the fee methods,
//! `eth_call`, `eth_estimateGas`, receipts and `eth_sendRawTransaction` -
//! before it was added. Operators are the chain's own public RPC, PublicNode
//! and dRPC's keyless tier.
//!
//! Some candidates were left out on purpose, and are worth not re-adding
//! without checking again:
//!
//! * BNB Chain's PublicNode answers 403 to a receipt it does not have, which
//!   is what a just-sent transaction looks like; dRPC rate-limits it hard.
//! * polygon-rpc.com answers 401 to everything.
//! * zkSync Era has no candidate that passed: dRPC does not serve
//!   `eth_maxPriorityFeePerGas` there, and PublicNode has no such chain.
//! * Mantle's dRPC times out on its free plan.
//! * 1rpc.io now forwards to whichever backend it picks, and answered
//!   differently from one round to the next.

/// The fallbacks for one chain, in the order they are tried.
pub fn fallbacks(chain_id: u64) -> &'static [&'static str] {
    match chain_id {
        // BNB Chain
        56 => &[
            "https://bsc-dataseed1.binance.org",
            "https://bsc-dataseed1.defibit.io",
        ],
        // Ethereum
        1 => &["https://eth.drpc.org"],
        // Polygon
        137 => &["https://polygon.drpc.org"],
        // Base
        8_453 => &["https://mainnet.base.org", "https://base.drpc.org"],
        // Arbitrum One
        42_161 => &["https://arb1.arbitrum.io/rpc", "https://arbitrum.drpc.org"],
        // Optimism
        10 => &["https://mainnet.optimism.io", "https://optimism.drpc.org"],
        // Avalanche C-Chain
        43_114 => &[
            "https://api.avax.network/ext/bc/C/rpc",
            "https://avalanche.drpc.org",
        ],
        // HyperEVM
        999 => &[
            "https://hyperliquid-rpc.publicnode.com",
            "https://hyperliquid.drpc.org",
        ],
        // Mantle
        5_000 => &["https://mantle-rpc.publicnode.com"],
        // Linea
        59_144 => &["https://linea-rpc.publicnode.com", "https://linea.drpc.org"],
        // Scroll
        534_352 => &[
            "https://scroll-rpc.publicnode.com",
            "https://scroll.drpc.org",
        ],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fallback is a different server, never the default again.
    #[test]
    fn no_chain_falls_back_to_its_own_default() {
        for c in crate::ALL {
            for f in fallbacks(c.chain_id) {
                assert_ne!(*f, c.default_rpc, "chain {}", c.chain_id);
                assert!(f.starts_with("https://"), "{f}");
            }
        }
    }

    /// Every list belongs to a chain this wallet has.
    #[test]
    fn every_list_is_for_a_known_chain() {
        for id in [
            56, 1, 137, 8_453, 42_161, 10, 43_114, 999, 5_000, 59_144, 534_352,
        ] {
            assert!(!fallbacks(id).is_empty());
            assert!(crate::ALL.iter().any(|c| c.chain_id == id), "chain {id}");
        }
        assert!(fallbacks(324).is_empty(), "zkSync Era has none that passed");
    }
}
