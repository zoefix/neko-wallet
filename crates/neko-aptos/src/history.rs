//! Transfer history, from Aptos's own indexer.
//!
//! A fullnode can say what an account *sent* - `/accounts/{a}/transactions`
//! lists the transactions it signed - and nothing at all about what it
//! received, because a payment to an account is not a transaction of that
//! account. Showing only one direction is the failure this wallet has already
//! made once on TON, where every USDT anybody sent you was simply missing. So
//! history here comes from the indexer or it says it is unavailable.
//!
//! `fungible_asset_activities` covers both assets. APT is a fungible asset now
//! as well as a coin, so one query answers for the coin and the token, with
//! `asset_type` telling them apart.
//!
//! The indexer asks for no key and rate-limits by IP instead - 40,000 compute
//! units per five minutes, shared by everyone behind the same address. That is
//! a real limit and it is reported as itself rather than as the network being
//! down.

use serde_json::Value;

use crate::address::AptosAddress;
use crate::error::AptosError;

pub const DEFAULT_INDEXER: &str = "https://api.mainnet.aptoslabs.com/v1/graphql";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

#[derive(Debug, Clone)]
pub struct Transfer {
    pub direction: Direction,
    pub amount: i128,
    pub decimals: u8,
    pub symbol: String,
    pub counterparty: String,
    /// Milliseconds, like every other chain here. The indexer answers in
    /// ISO-8601 with microseconds, which is a third unit to get wrong.
    pub block_ts: i64,
    pub id: String,
}

pub struct Indexer {
    url: String,
    http: reqwest::Client,
}

impl Indexer {
    pub fn new(url: Option<&str>) -> Self {
        Indexer {
            url: url
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .unwrap_or(DEFAULT_INDEXER)
                .to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(45))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.url
    }

    /// Both assets, newest first.
    ///
    /// Ordered by `transaction_version` rather than by timestamp: the version
    /// is the indexed column, and ordering on the timestamp times the query
    /// out on a busy account.
    pub async fn transfers(
        &self,
        who: AptosAddress,
        limit: usize,
    ) -> Result<Vec<Transfer>, AptosError> {
        let query = format!(
            "query {{ fungible_asset_activities(\
               where: {{owner_address: {{_eq: \"{who}\"}}}}, \
               order_by: {{transaction_version: desc}}, \
               limit: {limit}) {{ \
                 amount asset_type type owner_address transaction_timestamp \
                 transaction_version is_transaction_success \
               }} }}"
        );
        let r = self
            .http
            .post(&self.url)
            .json(&serde_json::json!({ "query": query }))
            .send()
            .await
            .map_err(|e| AptosError::Rpc(e.to_string()))?;
        let text = r.text().await.map_err(|e| AptosError::Rpc(e.to_string()))?;
        let v: Value = serde_json::from_str(&text).map_err(|_| AptosError::BadReply(cut(&text)))?;
        if let Some(errs) = v.get("errors") {
            let msg = errs
                .as_array()
                .and_then(|a| a.first())
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("the indexer refused the query");
            return Err(AptosError::Rpc(cut(msg)));
        }
        Ok(parse(&v, who))
    }
}

/// Turn the indexer's rows into transfers.
///
/// Split out so it can be tested against a recorded reply. Two things are
/// decided here and both are easy to get wrong: which direction a row is, and
/// which asset it belongs to.
pub fn parse(body: &Value, who: AptosAddress) -> Vec<Transfer> {
    let rows = body
        .get("data")
        .and_then(|d| d.get("fungible_asset_activities"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mine = who.to_string();
    rows.iter()
        .filter_map(|r| {
            // A failed transaction moved nothing. Showing it as a payment is
            // the same mistake as counting a bounce on TON.
            if r.get("is_transaction_success").and_then(Value::as_bool) == Some(false) {
                return None;
            }
            let asset = r.get("asset_type").and_then(Value::as_str)?;
            let (symbol, decimals) = known_asset(asset)?;
            let amount: i128 = r
                .get("amount")
                .and_then(|a| match a {
                    Value::String(s) => s.parse().ok(),
                    Value::Number(n) => n.as_i64().map(i128::from),
                    _ => None,
                })
                .unwrap_or(0);
            if amount == 0 {
                return None;
            }
            // `type` names the Move event, and its tail says which way the
            // asset went for `owner_address`.
            let kind = r.get("type").and_then(Value::as_str).unwrap_or("");
            let direction = if kind.ends_with("WithdrawEvent") || kind.ends_with("Withdraw") {
                Direction::Out
            } else if kind.ends_with("DepositEvent") || kind.ends_with("Deposit") {
                Direction::In
            } else {
                return None;
            };
            let owner = r.get("owner_address").and_then(Value::as_str).unwrap_or("");
            if owner != mine {
                return None;
            }
            Some(Transfer {
                direction,
                amount,
                decimals,
                symbol: symbol.to_string(),
                counterparty: String::new(),
                block_ts: iso_to_millis(
                    r.get("transaction_timestamp")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                ),
                id: r
                    .get("transaction_version")
                    .map(|v| v.to_string().trim_matches('"').to_string())
                    .unwrap_or_default(),
            })
        })
        .collect()
}

/// The two assets this wallet shows, and nothing else.
///
/// An account can hold any number of fungible assets, and the names come from
/// whoever minted them - which is exactly the input this wallet refuses to put
/// on screen. So a row is kept only when its asset is one of the two the chain
/// definition names, and the name shown is this wallet's.
pub fn known_asset(asset_type: &str) -> Option<(&'static str, u8)> {
    let a = asset_type.trim();
    // The coin form, which is a Move type rather than an address.
    if a.eq_ignore_ascii_case("0x1::aptos_coin::AptosCoin") {
        return Some(("APT", crate::APT_DECIMALS));
    }
    // And the fungible-asset form, which is an address - and has to be
    // compared *as one*. The indexer pads it: APT's metadata is `0xa` in the
    // documentation and `0x000…00a` in every reply, so a string comparison
    // against `"0xa"` drops every APT row and leaves a history that shows
    // tokens and no coin.
    let addr = AptosAddress::parse(a).ok()?;
    if addr == AptosAddress::parse(crate::APT_METADATA).ok()? {
        return Some(("APT", crate::APT_DECIMALS));
    }
    if addr == crate::usdt_metadata() {
        // Shown as USDT. The contract calls itself `USDt`.
        return Some(("USDT", crate::USDT_DECIMALS));
    }
    None
}

/// ISO-8601 with microseconds to milliseconds since the epoch.
///
/// Every provider in this wallet reports milliseconds, and this is the third
/// one to arrive in different units - the seconds-versus-milliseconds bug has
/// been introduced twice already.
fn iso_to_millis(s: &str) -> i64 {
    // `2026-09-05T14:54:05.000000` or with a trailing Z.
    let s = s.trim_end_matches('Z');
    let (date, time) = match s.split_once('T') {
        Some(p) => p,
        None => return 0,
    };
    let mut d = date.split('-');
    let (y, m, day) = match (d.next(), d.next(), d.next()) {
        (Some(a), Some(b), Some(c)) => (
            a.parse::<i64>().unwrap_or(0),
            b.parse::<i64>().unwrap_or(0),
            c.parse::<i64>().unwrap_or(0),
        ),
        _ => return 0,
    };
    let mut t = time.split(':');
    let (hh, mm, rest) = match (t.next(), t.next(), t.next()) {
        (Some(a), Some(b), Some(c)) => (
            a.parse::<i64>().unwrap_or(0),
            b.parse::<i64>().unwrap_or(0),
            c,
        ),
        _ => return 0,
    };
    let (ss, frac) = match rest.split_once('.') {
        Some((a, b)) => (a.parse::<i64>().unwrap_or(0), b),
        None => (rest.parse::<i64>().unwrap_or(0), ""),
    };
    let millis: i64 = frac
        .chars()
        .chain(std::iter::repeat('0'))
        .take(3)
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    (days_from_civil(y, m, day) * 86_400 + hh * 3_600 + mm * 60 + ss) * 1_000 + millis
}

/// Days since 1970-01-01, by Howard Hinnant's algorithm. The same one the
/// Blockscout reader uses, for the same reason: no date library in a wallet.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn cut(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= 240 {
        return t.to_string();
    }
    t.chars().take(240).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn me() -> AptosAddress {
        AptosAddress::parse("0xeb663b681209e7087d681c5d3eed12aaa8e1915e7c87794542c3f96e94b3d3bf")
            .unwrap()
    }

    /// A deposit and a withdrawal, in both assets.
    #[test]
    fn both_directions_and_both_assets_are_read() {
        let body = json!({"data": {"fungible_asset_activities": [
            {"amount": "150000000",
             "asset_type": "0x000000000000000000000000000000000000000000000000000000000000000a",
             "type": "0x1::fungible_asset::Deposit", "owner_address": me().to_string(),
             "transaction_timestamp": "2026-09-05T14:54:05.000000",
             "transaction_version": "7094849185", "is_transaction_success": true},
            {"amount": "2500000", "asset_type": super::super::USDT_METADATA,
             "type": "0x1::fungible_asset::Withdraw", "owner_address": me().to_string(),
             "transaction_timestamp": "2026-09-04T01:26:53.500000",
             "transaction_version": "7094000000", "is_transaction_success": true}
        ]}});
        let out = parse(&body, me());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].direction, Direction::In);
        assert_eq!(out[0].symbol, "APT");
        assert_eq!(out[0].decimals, 8);
        assert_eq!(out[0].amount, 150_000_000);
        assert_eq!(out[1].direction, Direction::Out);
        // Shown as USDT even though the contract says `USDt`.
        assert_eq!(out[1].symbol, "USDT");
        assert_eq!(out[1].decimals, 6);
    }

    /// The indexer pads addresses, and the wallet's constant does not.
    ///
    /// APT's metadata is `0xa` where it is written down and
    /// `0x000…00a` in every reply the indexer sends. Comparing the two as
    /// strings drops every APT row - a history showing tokens and no coin,
    /// which looks like an account that never held any.
    #[test]
    fn the_indexers_padded_form_is_the_same_asset() {
        let padded = "0x000000000000000000000000000000000000000000000000000000000000000a";
        assert_eq!(known_asset(padded).unwrap().0, "APT");
        assert_eq!(known_asset("0xa").unwrap().0, "APT");
        assert_eq!(known_asset("0x1::aptos_coin::AptosCoin").unwrap().0, "APT");
        // And the token, which the indexer sends unpadded because it is
        // already full width.
        assert_eq!(known_asset(super::super::USDT_METADATA).unwrap().0, "USDT");
        // Neither of them is everything.
        assert_eq!(known_asset("0xb"), None);
    }

    /// An asset this wallet does not carry is dropped rather than rendered.
    ///
    /// Anyone can mint a fungible asset and call it whatever they like -
    /// including `APT`. The filter is on the asset's address, and the name
    /// shown is this wallet's own.
    #[test]
    fn an_unknown_asset_is_not_shown_under_a_name_it_chose() {
        let body = json!({"data": {"fungible_asset_activities": [
            {"amount": "1", "asset_type": "0xdead::fake::APT",
             "type": "0x1::coin::DepositEvent", "owner_address": me().to_string(),
             "transaction_timestamp": "2026-09-05T00:00:00.000000",
             "transaction_version": "1", "is_transaction_success": true}
        ]}});
        assert!(parse(&body, me()).is_empty());
    }

    /// A failed transaction moved nothing and is not a payment.
    #[test]
    fn a_failed_transaction_is_not_a_transfer() {
        let body = json!({"data": {"fungible_asset_activities": [
            {"amount": "150000000", "asset_type": "0x1::aptos_coin::AptosCoin",
             "type": "0x1::coin::DepositEvent", "owner_address": me().to_string(),
             "transaction_timestamp": "2026-09-05T00:00:00.000000",
             "transaction_version": "1", "is_transaction_success": false}
        ]}});
        assert!(parse(&body, me()).is_empty());
    }

    /// Milliseconds, like every other provider in this wallet.
    #[test]
    fn timestamps_are_milliseconds() {
        // 2026-09-05T14:54:05.000000 UTC
        assert_eq!(
            iso_to_millis("2026-09-05T14:54:05.000000"),
            1_788_620_045_000
        );
        assert_eq!(
            iso_to_millis("2026-09-05T14:54:05.250000"),
            1_788_620_045_250
        );
        // The epoch itself, which catches an off-by-one in the day count.
        assert_eq!(iso_to_millis("1970-01-01T00:00:00.000000"), 0);
        assert_eq!(iso_to_millis("2000-03-01T00:00:00Z"), 951_868_800_000);
        assert_eq!(iso_to_millis("nonsense"), 0);
    }
}
