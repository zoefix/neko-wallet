//! Transaction history from Routescan, which asks for no key.
//!
//! Routescan is what runs Snowtrace, and it serves an Etherscan-shaped API
//! for the chains it covers without an account - which for this wallet means
//! **Avalanche and Mantle**, two chains that otherwise had no index at all and
//! showed "history unavailable" to somebody whose funds had plainly arrived.
//!
//! The reply is Etherscan V1's shape, field for field, so the parsing is
//! [`crate::etherscan`]'s rather than a second copy. Only the URL differs: the
//! chain id is a path segment here and a query parameter there, and there is
//! no key.
//!
//! It covers Avalanche and Mantle and answers `chain not supported` for every
//! other chain in this wallet - checked, one at a time, rather than assumed
//! from the fact that it supports two.

use neko_hd::EvmAddress;
use neko_net::{Request, Service};
use serde_json::Value;

use crate::error::EvmError;
use crate::history::Transfer;
use crate::EvmToken;

pub struct Routescan {
    chain: crate::EvmChain,
    net: Service,
}

impl Routescan {
    /// `None` for a chain Routescan does not serve, so a caller cannot build
    /// one that would ask a host with nothing to say.
    pub fn new(chain: crate::EvmChain) -> Option<Self> {
        Some(Routescan {
            chain,
            net: Service::new(chain.routescan?)
                .timeout(std::time::Duration::from_secs(45))
                .classify(crate::etherscan::rate_limit_in_result),
        })
    }

    pub fn endpoint(&self) -> &str {
        self.net.primary()
    }

    async fn call(&self, action: &str, who: &str, extra: &str) -> Result<Value, EvmError> {
        let query = format!(
            "?module=account&action={action}&address={who}{extra}\
             &page=1&offset=50&sort=desc"
        );
        let reply = self
            .net
            .send(Request::get(query))
            .await
            .map_err(|e| EvmError::Network(e.to_string()))?;
        let v: Value =
            serde_json::from_str(&reply.body).map_err(|e| EvmError::BadReply(e.to_string()))?;
        // The same envelope, including the "no transactions found" reply that
        // arrives dressed as an error.
        crate::etherscan::result_of(&v)
    }

    /// Both legs, and a failure on one does not discard the other.
    pub async fn transfers(
        &self,
        who: EvmAddress,
        tokens: &[EvmToken],
        limit: usize,
    ) -> Result<Vec<Transfer>, EvmError> {
        let a = who.to_string();
        // One `tokentx` per token, in turn: the filter takes one contract, and
        // an unfiltered list lets any token that spams this address crowd the
        // real ones out of the page.
        let token_leg = async {
            let mut rows = Vec::new();
            let mut failure = None;
            for t in tokens {
                let contract = format!("&contractaddress={}", t.address());
                match self.call("tokentx", &a, &contract).await {
                    Ok(v) => rows.extend(crate::etherscan::parse_tokens(&v, *t)),
                    Err(e) => failure = failure.or(Some(e)),
                }
            }
            (rows, failure)
        };
        let (coins, (token_rows, token_failure)) =
            tokio::join!(self.call("txlist", &a, ""), token_leg);

        let mut out = token_rows;
        let mut failure = token_failure;
        match coins {
            Ok(v) => out.extend(crate::etherscan::parse_coins(&v, self.chain)),
            Err(e) => failure = Some(e),
        }
        if out.is_empty() {
            if let Some(e) = failure {
                return Err(e);
            }
        }
        out.sort_by_key(|t| std::cmp::Reverse(t.block_ts));
        out.truncate(limit);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Only the two chains it actually serves get a client.
    ///
    /// Every chain here was asked. Routescan answers `chain not supported`
    /// for the other ten, and building a client for one of those would trade
    /// an honest "no index" for a request that fails as a network error.
    #[test]
    fn a_client_exists_only_where_routescan_serves_the_chain() {
        let served: Vec<u64> = crate::ALL
            .iter()
            .filter(|c| Routescan::new(**c).is_some())
            .map(|c| c.chain_id)
            .collect();
        assert_eq!(
            served,
            vec![crate::AVALANCHE.chain_id, crate::MANTLE.chain_id],
            "the set of chains Routescan serves changed"
        );
        assert!(Routescan::new(crate::ETHEREUM).is_none());
        assert!(Routescan::new(crate::HYPER_EVM).is_none());
        assert!(Routescan::new(crate::LINEA).is_none());
    }

    /// The chain id is in the path, not in a query parameter, and there is no
    /// key anywhere in the URL.
    #[test]
    fn the_endpoint_names_the_chain_and_carries_no_key() {
        let r = Routescan::new(crate::AVALANCHE).unwrap();
        assert!(r.endpoint().contains("/43114/"), "{}", r.endpoint());
        assert!(!r.endpoint().contains("apikey"));
        let m = Routescan::new(crate::MANTLE).unwrap();
        assert!(m.endpoint().contains("/5000/"), "{}", m.endpoint());
        assert_ne!(r.endpoint(), m.endpoint());
    }

    /// A real reply, recorded from mainnet.
    ///
    /// Both rows are this wallet's own address on Avalanche: 0.006 AVAX in,
    /// and 4.96 of Tether's token. The token row is the one worth pinning -
    /// the contract calls itself `USDt` and the wallet shows `USDT`, and what
    /// is displayed comes from the chain definition rather than from the
    /// reply.
    #[test]
    fn a_recorded_avalanche_reply_is_read_correctly() {
        let coins = json!([{
            "blockNumber": "94601587",
            "timeStamp": "1788688524",
            "hash": "0x3ffd99b41e7e678ed6d6337d673d44a28b056da6ae76686221864b3c81ffddc1",
            "from": "0x978b21a854a1jjjj0000000000000000000000aa",
            "to": "0xa41811cf4d41e306310cb82b47258c22b80475cc",
            "value": "6000000000000000",
            "isError": "0"
        }]);
        let out = crate::etherscan::parse_coins(&coins, crate::AVALANCHE);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].amount, 6_000_000_000_000_000);
        assert_eq!(out[0].symbol, "AVAX");
        assert_eq!(out[0].decimals, 18);
        // Milliseconds, like every other provider here.
        assert_eq!(out[0].block_ts, 1_788_688_524_000);

        let tokens = json!([{
            "timeStamp": "1788692092",
            "hash": "0x11",
            "from": "0x978b21a854a1jjjj0000000000000000000000aa",
            "to": "0xa41811cf4d41e306310cb82b47258c22b80475cc",
            "value": "4960000",
            "tokenSymbol": "USDt",
            "tokenDecimal": "6",
            "contractAddress": "0x9702230a8ea53601f5cd2dc00fdbc13d4df4a8c7"
        }]);
        let out = crate::etherscan::parse_tokens(&tokens, crate::AVALANCHE.tokens()[0]);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].amount, 4_960_000);
        assert_eq!(out[0].decimals, 6);
        // `USDt` is what the contract says; `USDT` is what this wallet shows.
        assert_eq!(out[0].symbol, "USDT");
        assert_eq!(out[0].block_ts, 1_788_692_092_000);
    }
}
