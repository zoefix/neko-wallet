//! Transaction history from Etherscan's V2 API.
//!
//! One key, sixty-one chains. That is the whole reason it is here: NodeReal
//! indexes two of this wallet's three EVM chains and Blockscout the third, and
//! a single key replaces both with one account and one host.
//!
//! Optional, and never contacted without a key - which is also why this module
//! cannot be verified the way the rest of this crate is. Every other network
//! path here was built against a recorded mainnet reply; this one was built
//! against Etherscan's published shape, because the endpoint answers
//! `Missing/Invalid API Key` to everything until a key exists. The parsing is
//! written to fail loudly rather than to guess, and the tests below pin the
//! two shapes that are easy to get wrong: a string-typed amount, and the
//! "no transactions found" reply that arrives dressed as an error.

use neko_hd::EvmAddress;
use serde_json::Value;

use crate::error::EvmError;
use crate::history::Transfer;

pub const BASE: &str = "https://api.etherscan.io/v2/api";
pub const SIGNUP_URL: &str = "https://etherscan.io/apis";

pub struct Etherscan {
    chain: crate::EvmChain,
    api_key: String,
    http: reqwest::Client,
}

impl Etherscan {
    pub fn new(chain: crate::EvmChain, api_key: &str) -> Option<Self> {
        let api_key = api_key.trim();
        if api_key.is_empty() {
            return None;
        }
        Some(Etherscan {
            chain,
            api_key: api_key.to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        })
    }

    async fn call(&self, action: &str, who: &str, extra: &str) -> Result<Value, EvmError> {
        let url = format!(
            "{BASE}?chainid={}&module=account&action={action}&address={who}{extra}\
             &page=1&offset=50&sort=desc&apikey={}",
            self.chain.chain_id, self.api_key
        );
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| EvmError::Network(e.to_string()))?;
        let v: Value = resp
            .json()
            .await
            .map_err(|e| EvmError::BadReply(e.to_string()))?;
        result_of(&v)
    }

    pub async fn transfers(
        &self,
        who: EvmAddress,
        token: EvmAddress,
        limit: usize,
    ) -> Result<Vec<Transfer>, EvmError> {
        let a = who.to_string();
        let contract = format!("&contractaddress={token}");
        let (coins, tokens) = tokio::join!(
            self.call("txlist", &a, ""),
            self.call("tokentx", &a, &contract),
        );

        let mut out = Vec::new();
        let mut failure = None;
        match coins {
            Ok(v) => out.extend(parse_coins(&v, self.chain)),
            Err(e) => failure = Some(e),
        }
        match tokens {
            Ok(v) => out.extend(parse_tokens(&v)),
            Err(e) => failure = failure.or(Some(e)),
        }
        // Only if nothing at all came back: half a history beats none.
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

/// Unwrap Etherscan's envelope.
///
/// `status: "0"` is not always a failure. An address with nothing to report
/// comes back as `status "0", message "No transactions found", result []` -
/// and treating that as an error puts a red line on the screen of every wallet
/// that has not used the chain yet.
pub fn result_of(v: &Value) -> Result<Value, EvmError> {
    let result = v.get("result").cloned().unwrap_or(Value::Null);
    if v.get("status").and_then(Value::as_str) == Some("1") {
        return Ok(result);
    }
    if result.is_array() {
        // An empty list, however it was labelled.
        return Ok(result);
    }
    let message = result
        .as_str()
        .or_else(|| v.get("message").and_then(Value::as_str))
        .unwrap_or("unknown error");
    Err(EvmError::Rpc(message.to_string()))
}

/// Coin movements, out of `txlist`.
pub fn parse_coins(result: &Value, chain: crate::EvmChain) -> Vec<Transfer> {
    rows(result)
        .iter()
        .filter_map(|t| {
            let amount = dec_str(t.get("value")?)?;
            if amount == 0 {
                return None;
            }
            Some(Transfer {
                hash: t.get("hash")?.as_str()?.to_string(),
                from: t.get("from")?.as_str()?.to_string(),
                to: t.get("to")?.as_str()?.to_string(),
                amount,
                symbol: chain.native_symbol.to_string(),
                decimals: chain.native_decimals,
                // Milliseconds: what `Transfer` carries, not what the reply
                // states.
                block_ts: (dec_str(t.get("timeStamp")?)? as i64).checked_mul(1_000)?,
                // "isError" is "1" when the call reverted. It still happened
                // and it still cost a fee.
                success: t.get("isError").and_then(Value::as_str) != Some("1"),
            })
        })
        .collect()
}

/// Token movements, out of `tokentx`, which was already filtered to one
/// contract by the request.
pub fn parse_tokens(result: &Value) -> Vec<Transfer> {
    rows(result)
        .iter()
        .filter_map(|t| {
            Some(Transfer {
                hash: t.get("hash")?.as_str()?.to_string(),
                from: t.get("from")?.as_str()?.to_string(),
                to: t.get("to")?.as_str()?.to_string(),
                amount: dec_str(t.get("value")?)?,
                // Our own label, not `tokenSymbol` from the reply - see
                // `crate::history::TOKEN_LABEL`.
                symbol: crate::history::TOKEN_LABEL.to_string(),
                // Stated by the reply. Six on Ethereum and Polygon, eighteen
                // on BNB Chain, for the same token name.
                decimals: dec_str(t.get("tokenDecimal")?)? as u8,
                // Milliseconds: what `Transfer` carries, not what the reply
                // states.
                block_ts: (dec_str(t.get("timeStamp")?)? as i64).checked_mul(1_000)?,
                success: true,
            })
        })
        .collect()
}

fn rows(result: &Value) -> Vec<Value> {
    result.as_array().cloned().unwrap_or_default()
}

/// Every number in this API is a decimal string, including the ones that would
/// fit in a JSON number.
fn dec_str(v: &Value) -> Option<i128> {
    match v {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_i64().map(i128::from),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The reply that is not an error, however it looks.
    #[test]
    fn no_transactions_found_is_an_empty_history() {
        let empty = json!({"status": "0", "message": "No transactions found", "result": []});
        assert_eq!(result_of(&empty).unwrap(), json!([]));

        // A real failure still is one.
        let bad = json!({"status": "0", "message": "NOTOK", "result": "Missing/Invalid API Key"});
        let err = result_of(&bad).unwrap_err().to_string();
        assert!(err.contains("Missing/Invalid API Key"), "{err}");

        let ok = json!({"status": "1", "message": "OK", "result": [{"hash": "0x1"}]});
        assert_eq!(result_of(&ok).unwrap(), json!([{"hash": "0x1"}]));
    }

    /// Amounts and timestamps are decimal strings. Read as numbers they are a
    /// parse failure and the row vanishes.
    #[test]
    fn everything_arrives_as_a_string() {
        let result = json!([{
            "hash": "0xaaa",
            "from": "0x1111111111111111111111111111111111111111",
            "to": "0x2222222222222222222222222222222222222222",
            "value": "406593928190000000000",
            "timeStamp": "1782513790",
            "isError": "0",
        }]);
        let rows = parse_coins(&result, crate::POLYGON);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].amount, 406_593_928_190_000_000_000);
        assert_eq!(rows[0].block_ts, 1_782_513_790_000, "milliseconds");
        assert_eq!(rows[0].symbol, "POL");
        assert!(rows[0].success);
    }

    /// A token's precision comes from the reply, and a reverted call is still
    /// a row.
    #[test]
    fn a_token_keeps_the_precision_the_reply_states() {
        let result = json!([{
            "hash": "0xbbb",
            "from": "0x1111111111111111111111111111111111111111",
            "to": "0x2222222222222222222222222222222222222222",
            "value": "2700000",
            "tokenSymbol": "USDT0",
            "tokenDecimal": "6",
            "timeStamp": "1782547200",
        }]);
        let rows = parse_tokens(&result);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].amount, 2_700_000);
        assert_eq!(rows[0].decimals, 6, "not the chain's eighteen");

        let reverted = json!([{
            "hash": "0xccc", "from": "0x1", "to": "0x2",
            "value": "1", "timeStamp": "1", "isError": "1",
        }]);
        assert!(!parse_coins(&reverted, crate::POLYGON)[0].success);
    }

    /// No key, no client - so this is never contacted by accident.
    #[test]
    fn a_blank_key_makes_no_client() {
        assert!(Etherscan::new(crate::POLYGON, "").is_none());
        assert!(Etherscan::new(crate::POLYGON, "   ").is_none());
        assert!(Etherscan::new(crate::POLYGON, "abc").is_some());
    }
}
