//! Transaction history from a Blockscout instance.
//!
//! Polygon's answer to a problem the other EVM chains solve with a key. A
//! node's own RPC cannot say what an address has done, NodeReal indexes no
//! Polygon, and `eth_getLogs` is not a substitute: it caps at ten thousand
//! blocks, which on a chain with 1.5-second blocks is four hours, and native
//! transfers emit no logs at all so it could never see them anyway.
//!
//! Blockscout indexes both and asks for no key. The trade is the usual one and
//! the same one Bitcoin already makes with Esplora: whoever answers learns
//! which address was asked about. The host is configurable for that reason.
//!
//! Two requests, because the chain keeps them apart: coin movements are
//! transactions, token movements are events inside them.
//!
//! One limitation worth knowing. Each endpoint returns its newest fifty rows
//! and the token this wallet knows is picked out of them here - the server's
//! own `token=` filter times out on this instance and answers with nothing, so
//! it is not used. An address buried in junk tokens can therefore have real
//! transfers pushed out of that window. The filter is on the contract address,
//! so nothing wrong is ever *shown*; the failure is a row missing, not a row
//! invented.

use neko_hd::EvmAddress;
use serde_json::Value;

use crate::error::EvmError;
use crate::history::Transfer;

impl crate::EvmChain {
    /// The Blockscout instance for this chain, when there is one.
    pub fn blockscout_base(&self) -> Option<&'static str> {
        self.blockscout
    }
}

pub struct Blockscout {
    chain: crate::EvmChain,
    base: String,
    http: reqwest::Client,
}

impl Blockscout {
    /// `None` for a chain with no instance configured, so a caller cannot
    /// build one that would post nowhere.
    pub fn new(chain: crate::EvmChain, base: Option<&str>) -> Option<Self> {
        Some(Blockscout {
            chain,
            base: base
                .filter(|u| !u.is_empty())
                .or(chain.blockscout)?
                .trim_end_matches('/')
                .to_string(),
            // Longer than the RPC clients': this is an index answering a
            // question about an address's whole past, not a node reading a
            // balance, and it takes seconds rather than milliseconds.
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
        })
    }

    pub fn endpoint(&self) -> &str {
        &self.base
    }

    /// One request, retried: a public instance times out under load often
    /// enough that a single attempt turns a working history screen into an
    /// intermittent one.
    async fn get(&self, path: &str) -> Result<Value, EvmError> {
        let url = format!("{}/api/v2/{path}", self.base);
        let mut last = None;
        for attempt in 0..3u32 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(1 << (attempt - 1))).await;
            }
            let resp = match self.http.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    last = Some(EvmError::Network(e.to_string()));
                    continue;
                }
            };
            let status = resp.status();
            if !status.is_success() {
                // A 4xx is a decision and will be the same next time; a 5xx or
                // a gateway timeout is worth another attempt.
                let err = EvmError::Rpc(format!("{} answered {status}", self.base));
                if status.is_client_error() {
                    return Err(err);
                }
                last = Some(err);
                continue;
            }
            match resp.json().await {
                Ok(v) => return Ok(v),
                Err(e) => last = Some(EvmError::BadReply(e.to_string())),
            }
        }
        Err(last.unwrap_or_else(|| EvmError::Network("no attempt succeeded".into())))
    }

    /// Coin and token movements for one address, newest first.
    ///
    /// A failure on either leg is not allowed to discard the other: a wallet
    /// that has both is better served by half a history than by none, and the
    /// two endpoints fail independently.
    pub async fn transfers(
        &self,
        who: EvmAddress,
        token: EvmAddress,
        limit: usize,
    ) -> Result<Vec<Transfer>, EvmError> {
        let a = who.to_string();
        let coin_path = format!("addresses/{a}/transactions");
        let token_path = format!("addresses/{a}/token-transfers?type=ERC-20");
        let (coins, tokens) = tokio::join!(self.get(&coin_path), self.get(&token_path));

        let mut out = Vec::new();
        let mut first_error = None;
        match &coins {
            Ok(v) => out.extend(parse_coins(v, self.chain)),
            Err(e) => first_error = Some(e.to_string()),
        }
        match &tokens {
            Ok(v) => out.extend(parse_tokens(v, self.chain, token)),
            Err(e) => {
                if out.is_empty() {
                    return Err(EvmError::Rpc(first_error.unwrap_or_else(|| e.to_string())));
                }
            }
        }
        if out.is_empty() {
            if let Some(e) = first_error {
                return Err(EvmError::Rpc(e));
            }
        }

        out.sort_by_key(|t| std::cmp::Reverse(t.block_ts));
        out.truncate(limit);
        Ok(out)
    }
}

/// Coin movements, out of `/transactions`.
///
/// Every row is a transaction this address sent or received, including contract
/// calls that moved no coin - a token transfer appears here too, as a call
/// worth zero. Those are dropped: the token leg is read from the other endpoint
/// with its real amount, and keeping both would show each token transfer twice,
/// once with the right figure and once as nothing.
pub fn parse_coins(body: &Value, chain: crate::EvmChain) -> Vec<Transfer> {
    items(body)
        .iter()
        .filter_map(|t| {
            let amount = big_int(t.get("value")?)?;
            if amount == 0 {
                return None;
            }
            Some(Transfer {
                hash: t.get("hash")?.as_str()?.to_string(),
                from: hash_of(t.get("from"))?,
                to: hash_of(t.get("to"))?,
                amount,
                symbol: chain.native_symbol.to_string(),
                decimals: chain.native_decimals,
                // Milliseconds: what `Transfer` carries, not what the reply
                // states.
                block_ts: epoch_secs(t.get("timestamp")?.as_str()?)?.checked_mul(1_000)?,
                // "ok" is Blockscout's word for a transaction that did not
                // revert. A reverted one still cost a fee and still belongs in
                // a history, marked as what it is.
                success: t.get("status").and_then(Value::as_str) == Some("ok"),
            })
        })
        .collect()
}

/// Token movements, out of `/token-transfers`, keeping only the one token this
/// wallet knows about.
///
/// `decimals` arrives as a **string**, in both the amount and the token. Read
/// as a number it is a parse failure and the row disappears; assumed to be 18
/// it is a factor of a million million on a six-decimal token.
pub fn parse_tokens(body: &Value, chain: crate::EvmChain, token: EvmAddress) -> Vec<Transfer> {
    let wanted = token.to_string().to_ascii_lowercase();
    items(body)
        .iter()
        .filter_map(|t| {
            let tok = t.get("token")?;
            let contract = tok.get("address_hash").or_else(|| tok.get("address"))?;
            if !contract.as_str()?.eq_ignore_ascii_case(&wanted) {
                return None;
            }
            let total = t.get("total")?;
            Some(Transfer {
                hash: t
                    .get("transaction_hash")
                    .or_else(|| t.get("tx_hash"))?
                    .as_str()?
                    .to_string(),
                from: hash_of(t.get("from"))?,
                to: hash_of(t.get("to"))?,
                amount: big_int(total.get("value")?)?,
                // The chain's own label, not the one in the reply.
                //
                // The row is already matched on the contract, so the name it
                // carries is the real token's - but rendering a string a
                // server sent is the habit this codebase does not have, and
                // the assets screen has to agree. It showed USDT0 here and
                // USDT there, for one balance.
                symbol: chain.stable_label.to_string(),
                decimals: num(total.get("decimals").or_else(|| tok.get("decimals"))?)? as u8,
                // Milliseconds: what `Transfer` carries, not what the reply
                // states.
                block_ts: epoch_secs(t.get("timestamp")?.as_str()?)?.checked_mul(1_000)?,
                // A token transfer that is in this list happened: the event is
                // only emitted by a call that succeeded.
                success: true,
            })
        })
        .collect()
}

fn items(body: &Value) -> Vec<Value> {
    body.get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn hash_of(v: Option<&Value>) -> Option<String> {
    Some(v?.get("hash")?.as_str()?.to_string())
}

/// A number that may be written as a string, which is how this API sends
/// anything that might not fit in a double.
fn big_int(v: &Value) -> Option<i128> {
    match v {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_i64().map(i128::from),
        _ => None,
    }
}

fn num(v: &Value) -> Option<u64> {
    match v {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_u64(),
        _ => None,
    }
}

/// `2026-06-27T07:59:25.000000Z` to seconds since the epoch.
///
/// Written out rather than pulled in: a date library is a large dependency to
/// add to a wallet for one field, and the arithmetic is Howard Hinnant's
/// days-from-civil, which is exact for every date this will ever see.
pub fn epoch_secs(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    let n = |from: usize, to: usize| s.get(from..to)?.parse::<i64>().ok();
    let (y, m, d) = (n(0, 4)?, n(5, 7)?, n(8, 10)?);
    let (hh, mm, ss) = (n(11, 13)?, n(14, 16)?, n(17, 19)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) || hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    // Shift the year so it starts in March: leap day lands at the end, and the
    // month-length pattern repeats without a table.
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hh * 3_600 + mm * 60 + ss)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Checked against an independent calculation, including the two dates the
    /// fixtures below carry and the three that break a naive implementation:
    /// a leap day, the last second of a year, and a century that is not a leap
    /// year.
    #[test]
    fn timestamps_convert_to_the_right_second() {
        for (text, want) in [
            ("1970-01-01T00:00:00.000000Z", 0),
            ("2000-02-29T12:00:00.000000Z", 951_825_600),
            ("2024-12-31T23:59:59.000000Z", 1_735_689_599),
            ("2026-06-26T22:43:10.000000Z", 1_782_513_790),
            ("2026-06-27T07:59:25.000000Z", 1_782_547_165),
            // 2100 is divisible by 4 and is not a leap year.
            ("2100-03-01T00:00:00.000000Z", 4_107_542_400),
        ] {
            assert_eq!(epoch_secs(text), Some(want), "{text}");
        }
        for bad in ["", "nonsense", "2026-06-27", "2026-13-01T00:00:00Z"] {
            assert_eq!(epoch_secs(bad), None, "{bad:?} parsed");
        }
    }

    const WHO: &str = "0x4173e5DB0E14736bC38c29502fA7b9643446A4b4";

    /// The exact shape Blockscout returned for a real Polygon address.
    ///
    /// Two things here are the reason this is a fixture and not a hand-written
    /// object: `value` and `decimals` are **strings**, and the addresses are
    /// nested inside objects rather than being fields.
    #[test]
    fn a_coin_transfer_is_read_out_of_a_real_reply() {
        let body = json!({"items": [{
            "timestamp": "2026-06-26T22:43:10.000000Z",
            "hash": "0xf73e6e456d5aab47843e60b51bef8e0b71498465591169642f577b129cf430f7",
            "from": {"hash": WHO, "is_contract": false},
            "to": {"hash": "0xA45dDC73cc89B7ad22E39Cf42e72642ad629EA0E", "is_contract": false},
            "value": "406593928190000000000",
            "status": "ok",
            "result": "success",
            "method": null,
        }]});
        let rows = parse_coins(&body, crate::POLYGON);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].amount, 406_593_928_190_000_000_000);
        assert_eq!(rows[0].symbol, "POL");
        assert_eq!(rows[0].decimals, 18);
        assert_eq!(rows[0].block_ts, 1_782_513_790_000, "milliseconds");
        assert!(rows[0].success);
        assert_eq!(rows[0].from, WHO);
    }

    /// A contract call that moved no coin is not a coin transfer.
    ///
    /// Every token transfer appears in `/transactions` as well, as a call
    /// worth zero. Counting those would put a second, empty row beside every
    /// real token movement.
    #[test]
    fn a_zero_value_call_is_not_a_coin_movement() {
        let body = json!({"items": [{
            "timestamp": "2026-06-26T22:43:10.000000Z",
            "hash": "0xabc",
            "from": {"hash": WHO},
            "to": {"hash": "0xc2132D05D31c914a87C6611C10748AEb04B58e8F"},
            "value": "0",
            "status": "ok",
            "method": "transfer",
        }]});
        assert!(parse_coins(&body, crate::POLYGON).is_empty());
    }

    /// Only the token this wallet knows about, and at the precision the reply
    /// states rather than the one the chain's native coin happens to use.
    #[test]
    fn only_the_wanted_token_comes_through_and_keeps_its_precision() {
        let usdt = crate::POLYGON.stable_address();
        let body = json!({"items": [
            {
                "timestamp": "2026-06-27T07:59:25.000000Z",
                "transaction_hash": "0xf625b9fc396217b53bded16eed79f9b994bc36fe39ed9488b8bc2f02766450b0",
                "from": {"hash": "0xAa7F03deE84967dcE7C2Bbc6A060bA5A9227ddF9"},
                "to": {"hash": WHO},
                // Some other token, at eighteen decimals.
                "total": {"decimals": "18", "value": "15000000000000000000"},
                "token": {"address_hash": "0x4dC0AC521C1f50c2a65E914E498B19C236969C71",
                          "decimals": "18", "symbol": "NERO"},
            },
            {
                "timestamp": "2026-06-27T08:00:00.000000Z",
                "transaction_hash": "0xdead",
                "from": {"hash": "0xAa7F03deE84967dcE7C2Bbc6A060bA5A9227ddF9"},
                "to": {"hash": WHO},
                // USDT, at six.
                "total": {"decimals": "6", "value": "2700000"},
                "token": {"address_hash": "0xc2132d05d31c914a87c6611c10748aeb04b58e8f",
                          "decimals": "6", "symbol": "USDT0"},
            },
        ]});
        let rows = parse_tokens(&body, crate::POLYGON, usdt);
        assert_eq!(rows.len(), 1, "another token came through: {rows:?}");
        assert_eq!(rows[0].amount, 2_700_000);
        assert_eq!(rows[0].decimals, 6, "read from the reply, not assumed");
        assert_eq!(
            rows[0].symbol, "USDT",
            "the reply says USDT0 and the screen must not repeat it: the assets \
             screen calls this same holding USDT"
        );
        assert_eq!(
            rows[0].block_ts, 1_782_547_200_000,
            "08:00:00 in milliseconds, not the other row's time in seconds"
        );
    }

    /// A reply with nothing in it is an empty history, not a failure.
    #[test]
    fn an_empty_reply_is_an_empty_history() {
        let empty = json!({"items": []});
        assert!(parse_coins(&empty, crate::POLYGON).is_empty());
        assert!(parse_tokens(&empty, crate::POLYGON, crate::POLYGON.stable_address()).is_empty());
        assert!(parse_coins(&json!({}), crate::POLYGON).is_empty());
    }

    /// A chain with no instance cannot make a client, so nothing can post to a
    /// host that was never configured.
    #[test]
    fn a_chain_without_an_instance_makes_no_client() {
        assert!(Blockscout::new(crate::POLYGON, None).is_some());
        assert!(Blockscout::new(crate::BSC, None).is_none());
        // A configured host wins, and works for any chain.
        let c = Blockscout::new(crate::BSC, Some("https://example.invalid/")).unwrap();
        assert_eq!(
            c.endpoint(),
            "https://example.invalid",
            "the slash is trimmed"
        );
    }
}
