//! Transaction history for BNB Chain.
//!
//! A node's own RPC cannot answer "what has this address done" - that needs an
//! index, and building one means replaying the chain. So this talks to a
//! provider, and the choice of provider is not incidental:
//!
//! * Etherscan's V2 API withdrew free access for BNB Chain (along with
//!   Avalanche, Base and OP) in favour of paid plans, and the old
//!   `api.bscscan.com` is deprecated outright.
//! * `eth_getLogs` needs no provider, but **native BNB transfers emit no
//!   logs**, so it can only ever return token movements. A history that
//!   silently omits every BNB transfer is worse than none: it looks complete.
//!   Public endpoints also cap the block range at a couple of hours.
//!
//! BSCTrace, via NodeReal's MegaNode, is BNB Chain's own recommended
//! replacement, has a free tier, and its `nr_getAssetTransfers` covers the
//! `external` category - which is exactly the native transfers `eth_getLogs`
//! cannot see.

use neko_hd::EvmAddress;
use serde_json::{json, Value};

use crate::error::EvmError;

pub const DEFAULT_HOST: &str = "https://bsc-mainnet.nodereal.io/v1";
pub const SIGNUP_URL: &str = "https://nodereal.io";

/// One movement of value, as the provider reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    pub hash: String,
    pub from: String,
    pub to: String,
    /// Minimal units. Never a float.
    pub amount: i128,
    pub symbol: String,
    pub decimals: u8,
    pub block_ts: i64,
    pub success: bool,
}

pub struct Bsctrace {
    url: String,
    http: reqwest::Client,
}

impl Bsctrace {
    /// `api_key` is required: the endpoint carries it in the path, and there
    /// is no anonymous access.
    pub fn new(api_key: &str) -> Self {
        Bsctrace {
            url: format!("{DEFAULT_HOST}/{api_key}"),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    async fn call(&self, params: Value) -> Result<Value, EvmError> {
        let body = json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "nr_getAssetTransfers",
            "params": [params],
        });
        let resp = self
            .http
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .map_err(|e| EvmError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(EvmError::Network(format!("HTTP {}", resp.status())));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| EvmError::BadReply(e.to_string()))?;
        if let Some(err) = v.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            // The one error worth naming, because the fix is a specific action
            // rather than "try again".
            if msg.contains("Unauthorized") {
                return Err(EvmError::Rpc(format!(
                    "the BNB Chain history key was rejected - check it, or get one free at {SIGNUP_URL}"
                )));
            }
            return Err(EvmError::Rpc(msg.to_string()));
        }
        v.get("result")
            .cloned()
            .ok_or_else(|| EvmError::BadReply("reply has no result".into()))
    }

    /// Everything this address sent or received, newest first.
    ///
    /// Two calls, because the provider filters on one direction at a time.
    /// A failure of either alone is not fatal - half a history beats none, and
    /// the caller is told nothing is missing only when nothing is.
    pub async fn transfers(
        &self,
        who: EvmAddress,
        token: EvmAddress,
        limit: usize,
    ) -> Result<Vec<Transfer>, EvmError> {
        let who_s = who.to_string();
        let categories = json!(["external", "20"]);
        let contracts = json!([token.to_string()]);

        let out_v = self
            .call(json!({
                "fromAddress": who_s,
                "category": categories,
                "contractAddresses": contracts,
                "withMetadata": true,
                "excludeZeroValue": false,
                "order": "desc",
            }))
            .await;
        let in_v = self
            .call(json!({
                "toAddress": who_s,
                "category": categories,
                "contractAddresses": contracts,
                "withMetadata": true,
                "excludeZeroValue": false,
                "order": "desc",
            }))
            .await;

        let mut all = Vec::new();
        let mut errors = Vec::new();
        for r in [out_v, in_v] {
            match r {
                Ok(v) => all.extend(parse(&v)),
                Err(e) => errors.push(e),
            }
        }
        // Both directions failed: there is nothing to show and a reason to give.
        if all.is_empty() {
            if let Some(e) = errors.into_iter().next() {
                return Err(e);
            }
        }

        all.sort_by(|a, b| b.block_ts.cmp(&a.block_ts).then(b.hash.cmp(&a.hash)));
        // A transfer to yourself appears in both directions; keep both, since
        // that is genuinely two movements, but drop exact duplicates.
        all.dedup_by(|a, b| a.hash == b.hash && a.from == b.from && a.to == b.to);
        all.truncate(limit);
        Ok(all)
    }
}

/// Read the provider's reply, skipping anything malformed rather than failing
/// the whole page: one odd row must not hide a history.
pub fn parse(result: &Value) -> Vec<Transfer> {
    let Some(rows) = result.get("transfers").and_then(Value::as_array) else {
        return Vec::new();
    };
    rows.iter().filter_map(parse_one).collect()
}

fn parse_one(t: &Value) -> Option<Transfer> {
    let category = t.get("category")?.as_str()?;
    // BNB and BEP-20 USDT both have eighteen decimals. Anything else would
    // need its precision from the chain, so it is skipped rather than shown
    // with a made-up scale.
    let (symbol, decimals) = match category {
        "external" | "internal" => ("BNB".to_string(), crate::BNB_DECIMALS),
        "20" => (
            t.get("asset")?.as_str().unwrap_or("USDT").to_string(),
            crate::USDT_DECIMALS,
        ),
        _ => return None,
    };
    Some(Transfer {
        hash: t.get("hash")?.as_str()?.to_string(),
        from: t.get("from")?.as_str()?.to_string(),
        to: t.get("to")?.as_str()?.to_string(),
        amount: parse_value(t.get("value")?.as_str()?)?,
        symbol,
        decimals,
        block_ts: t.get("blockTimeStamp").and_then(Value::as_i64).unwrap_or(0),
        // Absent means the provider did not say; treating that as failure
        // would mark real transfers as failed.
        success: t
            .get("receiptsStatus")
            .and_then(Value::as_i64)
            .map(|s| s == 1)
            .unwrap_or(true),
    })
}

/// Amounts arrive as hex, sometimes padded to 32 bytes and sometimes minimal.
///
/// Refuses anything above 128 bits rather than wrapping: a wrapped amount
/// would be displayed as a real figure.
fn parse_value(s: &str) -> Option<i128> {
    let body = s.trim_start_matches("0x").trim_start_matches("0X");
    let body = body.trim_start_matches('0');
    if body.is_empty() {
        return Some(0);
    }
    if body.len() > 31 {
        return None;
    }
    i128::from_str_radix(body, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_survive_padding_and_refuse_overflow() {
        // The padded form the provider actually sends.
        assert_eq!(
            parse_value("0x000000000000000000000000000000000000000000000021ef7ec34f52880000"),
            Some(626_000_000_000_000_000_000)
        );
        // And the minimal form.
        assert_eq!(parse_value("0x232773380d7000"), Some(9_895_000_000_000_000));
        assert_eq!(parse_value("0x0"), Some(0));
        assert_eq!(parse_value("0x"), Some(0));
        // Over 128 bits: refused, never wrapped into a plausible number.
        assert_eq!(parse_value(&format!("0x1{}", "0".repeat(31))), None);
        assert_eq!(parse_value("0xzz"), None);
    }

    /// The real reply shape, taken from the provider.
    #[test]
    fn a_native_transfer_is_read_correctly() {
        let v = serde_json::json!({"transfers": [{
            "category": "external",
            "from": "0x9858effd232b4033e47d90003d41ec34ecaeda94",
            "to": "0x5eae506f855895a3d99c3e6863b3c01600301ffd",
            "value": "0x232773380d7000",
            "asset": "BNB",
            "hash": "0xcb01c531b6d1642dd8aebcb2d88f8ec884ebabb7f75a281e0f007ae27de3ea26",
            "blockTimeStamp": 1620515273,
            "receiptsStatus": 1
        }]});
        let got = parse(&v);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].symbol, "BNB");
        assert_eq!(got[0].decimals, 18);
        assert_eq!(got[0].amount, 9_895_000_000_000_000);
        assert!(got[0].success);
        assert_eq!(got[0].block_ts, 1620515273);
    }

    #[test]
    fn a_token_transfer_is_read_correctly() {
        let v = serde_json::json!({"transfers": [{
            "category": "20",
            "from": "0x8fa75b899f47133df83667b7bb3bc36f1aac27f6",
            "to": "0x8894e0a0c962cb723c1976a4421c95949be2d4e3",
            "value": "0x000000000000000000000000000000000000000000000021ef7ec34f52880000",
            "asset": "USDT",
            "contractAddress": "0x55d398326f99059ff775485246999027b3197955",
            "hash": "0xf2fb91086074132e8d6814178c1d5ae69aaae370b66801f608bfba9f61ffc1c2",
            "blockTimeStamp": 1788469375,
            "receiptsStatus": 1
        }]});
        let got = parse(&v);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].symbol, "USDT");
        // Eighteen here, six on TRON. The number travels with the transfer.
        assert_eq!(got[0].decimals, 18);
        assert_eq!(got[0].amount, 626_000_000_000_000_000_000);
    }

    /// The reply comes from a provider we do not control, so every shape of
    /// nonsense must be skipped rather than panic or become a wrong number.
    #[test]
    fn malformed_rows_are_skipped_not_fatal() {
        let v = serde_json::json!({"transfers": [
            {"category": "external"},                       // missing everything
            {"category": "nft", "hash": "0x1"},             // a category we do not show
            {"category": "20", "from": "0xa", "to": "0xb",
             "value": "0xnothex", "asset": "USDT", "hash": "0x2"},
            {"category": "external", "from": "0xa", "to": "0xb",
             "value": "0x1", "hash": "0x3", "blockTimeStamp": 5, "receiptsStatus": 0},
        ]});
        let got = parse(&v);
        assert_eq!(got.len(), 1, "a good row was lost among bad ones");
        assert_eq!(got[0].hash, "0x3");
        assert!(!got[0].success, "a failed transfer was shown as successful");

        // A row with no transaction hash cannot be shown or looked up, so it
        // is skipped. Worth its own case: it is exactly what a truncated
        // reply looks like.
        assert!(parse(&serde_json::json!({"transfers": [{
            "category": "20", "from": "0xa", "to": "0xb",
            "value": "0x1", "asset": "USDT", "blockTimeStamp": 1
        }]}))
        .is_empty());

        assert!(parse(&serde_json::json!({})).is_empty());
        assert!(parse(&serde_json::json!({"transfers": "nope"})).is_empty());
    }
}
