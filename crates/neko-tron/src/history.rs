//! Parsing transaction history from the TronGrid v1 indexer.
//!
//! Direction is decided **locally**, by comparing against the addresses this
//! wallet owns — never taken from an API field. The same transaction can be
//! both an inflow and an outflow (sending to yourself), which is why entries
//! are keyed by (txid, direction) rather than txid alone.

use neko_hd::Address;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxStatus {
    Success,
    Failed,
    Pending,
}

/// Incoming transfers below a thousandth of a token are not payments. Nobody
/// sends a fraction of a cent by accident.
///
/// Expressed as a fraction rather than a fixed number of minimal units,
/// because the number of units in "0.001 tokens" depends on the token: USDT
/// has six decimals on TRON and eighteen on BNB Chain. A constant tuned for
/// six would be a million million times too small on eighteen - no incoming
/// transfer would ever fall below it, and the address-poisoning filter would
/// silently stop filtering.
pub const DUST_FRACTION_DENOMINATOR: i128 = 1_000;

/// The dust threshold for a given precision, in minimal units.
pub fn dust_threshold(decimals: u8) -> i128 {
    10i128
        .checked_pow(decimals as u32)
        .map(|unit| unit / DUST_FRACTION_DENOMINATOR)
        // Below three decimals there is no sub-thousandth amount to speak of;
        // treat nothing as dust rather than everything.
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub txid: String,
    pub block_ts: i64,
    pub symbol: String,
    pub decimals: u8,
    /// Minimal units. Never a float.
    pub amount: i128,
    pub direction: Direction,
    /// The other party, base58.
    pub counterparty: String,
    pub status: TxStatus,
}

impl HistoryEntry {
    /// Suspected address-poisoning dust.
    ///
    /// The attack: generate vanity addresses whose first and last characters
    /// match someone you actually pay, send a fraction of a cent so the address
    /// lands in your history, and wait for you to copy the wrong one when you
    /// next send funds. The dust itself is harmless; the entry in your history
    /// is the payload.
    pub fn is_dust(&self) -> bool {
        self.direction == Direction::In && self.amount < dust_threshold(self.decimals)
    }
}

/// Do two addresses look alike under head/tail abbreviation while actually
/// being different? That is exactly the confusion address poisoning buys.
pub fn looks_alike(a: &str, b: &str) -> bool {
    if a == b {
        return false;
    }
    let (ac, bc): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    if ac.len() < 10 || bc.len() < 10 {
        return false;
    }
    let head = 4;
    let tail = 4;
    ac[..head] == bc[..head] && ac[ac.len() - tail..] == bc[bc.len() - tail..]
}

/// A `41...` hex address as returned by the full-node API.
fn addr_from_hex(s: &str) -> Option<Address> {
    let s = s.trim_start_matches("0x");
    if s.len() != 42 {
        return None;
    }
    let bytes: Option<Vec<u8>> = (0..42)
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect();
    Address::from_bytes(&bytes?).ok()
}

fn addr_any(s: &str) -> Option<Address> {
    Address::parse(s).ok().or_else(|| addr_from_hex(s))
}

/// Parse native TRX transfers from `/v1/accounts/{addr}/transactions`.
///
/// Only `TransferContract` entries are kept. Contract calls appear in this feed
/// too, but their token movements come from the TRC20 endpoint, and counting
/// both would double-report the same transfer.
pub fn parse_trx(body: &Value, owned: &[Address]) -> Vec<HistoryEntry> {
    let mut out = Vec::new();
    let Some(items) = body.get("data").and_then(Value::as_array) else {
        return out;
    };

    for it in items {
        let Some(txid) = it.get("txID").and_then(Value::as_str) else {
            continue;
        };
        let ts = it
            .get("block_timestamp")
            .and_then(Value::as_i64)
            .unwrap_or(0);

        let status = match it.pointer("/ret/0/contractRet").and_then(Value::as_str) {
            Some("SUCCESS") => TxStatus::Success,
            // A failed transaction still landed on-chain and still cost a fee,
            // so it belongs in the history rather than being hidden.
            Some(_) => TxStatus::Failed,
            None => TxStatus::Pending,
        };

        let Some(contracts) = it.pointer("/raw_data/contract").and_then(Value::as_array) else {
            continue;
        };
        for c in contracts {
            if c.get("type").and_then(Value::as_str) != Some("TransferContract") {
                continue;
            }
            let v = c.pointer("/parameter/value");
            let (Some(from), Some(to), Some(amount)) = (
                v.and_then(|v| v.get("owner_address"))
                    .and_then(Value::as_str)
                    .and_then(addr_any),
                v.and_then(|v| v.get("to_address"))
                    .and_then(Value::as_str)
                    .and_then(addr_any),
                v.and_then(|v| v.get("amount")).and_then(Value::as_i64),
            ) else {
                continue;
            };
            out.extend(classify(
                txid,
                ts,
                "TRX",
                6,
                amount as i128,
                from,
                to,
                owned,
                status,
            ));
        }
    }
    out
}

/// Parse TRC20 transfers from `/v1/accounts/{addr}/transactions/trc20`.
pub fn parse_trc20(body: &Value, owned: &[Address]) -> Vec<HistoryEntry> {
    let mut out = Vec::new();
    let Some(items) = body.get("data").and_then(Value::as_array) else {
        return out;
    };

    for it in items {
        let Some(txid) = it.get("transaction_id").and_then(Value::as_str) else {
            continue;
        };
        if it
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| t != "Transfer")
        {
            continue;
        }
        let ts = it
            .get("block_timestamp")
            .and_then(Value::as_i64)
            .unwrap_or(0);

        // `value` is a decimal STRING. Reading it as a JSON number would go via
        // f64 and start losing precision above ~9e15 minimal units.
        let Some(amount) = it
            .get("value")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<i128>().ok())
        else {
            continue;
        };

        let symbol = it
            .pointer("/token_info/symbol")
            .and_then(Value::as_str)
            .unwrap_or("?")
            .to_string();
        let decimals = it
            .pointer("/token_info/decimals")
            .and_then(Value::as_u64)
            .unwrap_or(6) as u8;

        let (Some(from), Some(to)) = (
            it.get("from").and_then(Value::as_str).and_then(addr_any),
            it.get("to").and_then(Value::as_str).and_then(addr_any),
        ) else {
            continue;
        };

        out.extend(classify(
            txid,
            ts,
            &symbol,
            decimals,
            amount,
            from,
            to,
            owned,
            TxStatus::Success,
        ));
    }
    out
}

/// Emit one entry per direction this wallet participates in. A self-transfer
/// legitimately produces two.
#[allow(clippy::too_many_arguments)]
fn classify(
    txid: &str,
    ts: i64,
    symbol: &str,
    decimals: u8,
    amount: i128,
    from: Address,
    to: Address,
    owned: &[Address],
    status: TxStatus,
) -> Vec<HistoryEntry> {
    let mut out = Vec::new();
    let mk = |direction, counterparty: Address| HistoryEntry {
        txid: txid.to_string(),
        block_ts: ts,
        symbol: symbol.to_string(),
        decimals,
        amount,
        direction,
        counterparty: counterparty.to_string(),
        status,
    };
    if owned.contains(&to) {
        out.push(mk(Direction::In, from));
    }
    if owned.contains(&from) {
        out.push(mk(Direction::Out, to));
    }
    out
}

/// Merge both feeds, newest first, dropping exact duplicates.
pub fn merge(mut entries: Vec<HistoryEntry>) -> Vec<HistoryEntry> {
    entries.sort_by(|a, b| b.block_ts.cmp(&a.block_ts).then(a.txid.cmp(&b.txid)));
    entries.dedup_by(|a, b| {
        a.txid == b.txid
            && a.direction == b.direction
            && a.symbol == b.symbol
            && a.amount == b.amount
    });
    entries
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MINE: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";
    const THEIRS: &str = "TNYxHL2s6Wjpx86NRwhekYzc27p3oDYrk6";

    fn owned() -> Vec<Address> {
        vec![Address::parse(MINE).unwrap()]
    }

    fn trx_body(from: &str, to: &str, amount: i64, ret: &str) -> Value {
        json!({"data": [{
            "txID": "aa".repeat(32),
            "block_timestamp": 1_756_000_000_000i64,
            "ret": [{"contractRet": ret}],
            "raw_data": {"contract": [{
                "type": "TransferContract",
                "parameter": {"value": {
                    "owner_address": Address::parse(from).unwrap().to_hex(),
                    "to_address": Address::parse(to).unwrap().to_hex(),
                    "amount": amount,
                }},
            }]},
        }]})
    }

    #[test]
    fn incoming_and_outgoing_are_classified_locally() {
        let inbound = parse_trx(&trx_body(THEIRS, MINE, 1_500_000, "SUCCESS"), &owned());
        assert_eq!(inbound.len(), 1);
        assert_eq!(inbound[0].direction, Direction::In);
        assert_eq!(inbound[0].counterparty, THEIRS);
        assert_eq!(inbound[0].amount, 1_500_000);

        let outbound = parse_trx(&trx_body(MINE, THEIRS, 1, "SUCCESS"), &owned());
        assert_eq!(outbound[0].direction, Direction::Out);
        assert_eq!(outbound[0].counterparty, THEIRS);
    }

    /// Sending to yourself is one transaction but two ledger entries.
    #[test]
    fn self_transfer_produces_both_directions() {
        let e = parse_trx(&trx_body(MINE, MINE, 5, "SUCCESS"), &owned());
        assert_eq!(e.len(), 2);
        assert!(e.iter().any(|x| x.direction == Direction::In));
        assert!(e.iter().any(|x| x.direction == Direction::Out));
    }

    #[test]
    fn unrelated_transactions_are_ignored() {
        let other = "TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf";
        assert!(parse_trx(&trx_body(other, THEIRS, 1, "SUCCESS"), &owned()).is_empty());
    }

    /// Failed transactions still cost a fee, so hiding them would misinform.
    #[test]
    fn failed_transactions_are_kept_and_marked() {
        let e = parse_trx(&trx_body(MINE, THEIRS, 1, "REVERT"), &owned());
        assert_eq!(e[0].status, TxStatus::Failed);
    }

    /// The amount arrives as a string precisely so it does not go through f64.
    #[test]
    fn trc20_amounts_are_read_as_strings() {
        let body = json!({"data": [{
            "transaction_id": "bb".repeat(32),
            "block_timestamp": 1_756_000_000_000i64,
            "from": THEIRS,
            "to": MINE,
            "type": "Transfer",
            // 2^53 + 1: unrepresentable as f64.
            "value": "9007199254740993",
            "token_info": {"symbol": "USDT", "decimals": 6},
        }]});
        let e = parse_trc20(&body, &owned());
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].amount, 9_007_199_254_740_993);
        assert_eq!(e[0].symbol, "USDT");
        assert_ne!(
            e[0].amount as f64 as i128, e[0].amount,
            "test premise: f64 must lose this"
        );
    }

    #[test]
    fn non_transfer_trc20_events_are_skipped() {
        let body = json!({"data": [{
            "transaction_id": "cc".repeat(32),
            "from": THEIRS, "to": MINE, "type": "Approval", "value": "1",
            "token_info": {"symbol": "USDT", "decimals": 6},
        }]});
        assert!(parse_trc20(&body, &owned()).is_empty());
    }

    #[test]
    fn merge_sorts_newest_first_and_dedups() {
        let mut e = parse_trx(&trx_body(THEIRS, MINE, 1, "SUCCESS"), &owned());
        e.extend(parse_trx(&trx_body(THEIRS, MINE, 1, "SUCCESS"), &owned()));
        let mut older = parse_trx(&trx_body(THEIRS, MINE, 2, "SUCCESS"), &owned());
        older[0].block_ts = 1;
        older[0].txid = "dd".repeat(32);
        e.extend(older);

        let merged = merge(e);
        assert_eq!(merged.len(), 2, "duplicate entry survived");
        assert!(merged[0].block_ts > merged[1].block_ts, "not newest first");
    }

    #[test]
    fn malformed_payloads_do_not_panic() {
        for body in [
            json!({}),
            json!({"data": "nope"}),
            json!({"data": [{}]}),
            json!(null),
        ] {
            assert!(parse_trx(&body, &owned()).is_empty());
            assert!(parse_trc20(&body, &owned()).is_empty());
        }
    }

    #[test]
    fn dust_is_only_flagged_on_incoming_transfers() {
        let mut e = parse_trx(&trx_body(THEIRS, MINE, 5, "SUCCESS"), &owned())[0].clone();
        assert!(e.is_dust(), "5 sun inbound is not a payment");

        e.amount = 1_500_000;
        assert!(!e.is_dust(), "1.5 TRX flagged as dust");

        // Sending dust yourself is your business, not an attack on you.
        let out = parse_trx(&trx_body(MINE, THEIRS, 5, "SUCCESS"), &owned())[0].clone();
        assert!(!out.is_dust());
    }

    #[test]
    fn dust_threshold_sits_below_any_real_payment() {
        assert_eq!(dust_threshold(6), 1_000, "0.001 token at 6 decimals");
        let mut e = parse_trx(&trx_body(THEIRS, MINE, 999, "SUCCESS"), &owned())[0].clone();
        assert!(e.is_dust());
        e.amount = 1_000;
        assert!(!e.is_dust(), "the threshold must be exclusive");
    }

    /// The threshold has to follow the token's precision. USDT has six
    /// decimals on TRON and eighteen on BNB Chain; a fixed number of minimal
    /// units would be a million million times too small on the latter, so no
    /// incoming transfer would ever be below it and the address-poisoning
    /// filter would quietly stop filtering.
    #[test]
    fn dust_is_the_same_value_at_every_precision() {
        assert_eq!(dust_threshold(6), 1_000);
        assert_eq!(dust_threshold(18), 1_000_000_000_000_000);

        // The same real amount - a tenth of a thousandth of a token - is dust
        // at both precisions.
        let mut e = parse_trx(&trx_body(THEIRS, MINE, 1, "SUCCESS"), &owned())[0].clone();
        e.decimals = 18;
        e.amount = 100_000_000_000_000; // 0.0001 tokens
        assert!(e.is_dust(), "dust at eighteen decimals was not detected");

        e.amount = 1_000_000_000_000_000_000; // 1 whole token
        assert!(!e.is_dust(), "a real payment was flagged as dust");
    }

    /// Absurd precisions must not panic or overflow.
    #[test]
    fn extreme_precisions_are_handled() {
        assert_eq!(dust_threshold(0), 0);
        assert_eq!(dust_threshold(2), 0);
        assert_eq!(dust_threshold(3), 1);
        // 10^39 overflows i128; degrade to "nothing is dust" rather than panic.
        assert_eq!(dust_threshold(39), 0);
        assert_eq!(dust_threshold(255), 0);
    }

    /// The exact confusion the attack manufactures.
    #[test]
    fn lookalike_addresses_are_detected() {
        assert!(looks_alike(
            "TNYxHL2s6Wjpx86NRwhekYzc27p3oDYrk6",
            "TNYxAAAAAAAAAAAAAAAAAAAAAAAAAAYrk6"
        ));
        // Identical is not a lookalike; it is the same address.
        assert!(!looks_alike(
            "TNYxHL2s6Wjpx86NRwhekYzc27p3oDYrk6",
            "TNYxHL2s6Wjpx86NRwhekYzc27p3oDYrk6"
        ));
        assert!(!looks_alike(
            "TNYxHL2s6Wjpx86NRwhekYzc27p3oDYrk6",
            "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH"
        ));
        assert!(!looks_alike("T", "T"), "short strings must not panic");
        assert!(!looks_alike("", "TNYxHL2s6Wjpx86NRwhekYzc27p3oDYrk6"));
    }

    #[test]
    fn hex_and_base58_addresses_both_parse() {
        let a = Address::parse(MINE).unwrap();
        assert_eq!(addr_any(MINE), Some(a));
        assert_eq!(addr_any(&a.to_hex()), Some(a));
        assert_eq!(addr_any("garbage"), None);
    }
}
