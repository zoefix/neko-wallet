//! Transfer history, from the indexer's balance changes.
//!
//! Sui reports, per transaction, how each address's holding of each coin type
//! moved. That is exactly what a history screen wants and it saves decoding
//! the programmable block that caused it - a payment can be any shape at all
//! here, so reading the *effect* is both simpler and more honest than trying
//! to recognise the shape.
//!
//! The rows come from GraphQL's `affectedAddress` filter, which is an index
//! over every address a transaction touched - sender, sponsor or recipient. A
//! node on its own can only list what an address *signed*, and a history
//! without what it received is the failure this wallet already shipped once
//! on TON. Timestamps arrive as RFC 3339 and leave as milliseconds, like every
//! other chain here.

use serde_json::Value;

use crate::address::SuiAddress;
use crate::graphql::int;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

#[derive(Debug, Clone)]
pub struct Transfer {
    pub direction: Direction,
    /// Always positive. The sign lives in `direction`.
    pub amount: i128,
    pub decimals: u8,
    pub symbol: String,
    pub counterparty: String,
    pub block_ts: i64,
    pub id: String,
}

/// The coin types this wallet shows, and nothing else.
///
/// Anyone can publish a coin type and call it `SUI`. The filter is on the
/// type's full name - package, module and struct - and the symbol shown is
/// this wallet's own.
pub fn known_coin(coin_type: &str) -> Option<(&'static str, u8)> {
    let t = coin_type.trim();
    if t == crate::SUI_TYPE
        || t == "0x0000000000000000000000000000000000000000000000000000000000000002::sui::SUI"
    {
        return Some(("SUI", crate::SUI_DECIMALS));
    }
    if t == crate::USDC_TYPE {
        return Some((crate::USDC_SYMBOL, crate::USDC_DECIMALS));
    }
    None
}

/// Turn a page of transactions into transfers, newest first.
///
/// `body` is the whole reply: `data.transactions.nodes`, oldest first, as the
/// connection returns them. A transaction that failed moved nothing but its
/// fee and is dropped. So is a change of zero, and so is the gas-only
/// movement of a transaction that sent nothing - paying a fee is not a
/// payment.
pub fn parse(body: &Value, who: SuiAddress) -> Vec<Transfer> {
    let mine = who.to_string();
    let rows = body
        .pointer("/data/transactions/nodes")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
    for tx in rows.iter().rev() {
        let Some(effects) = tx.get("effects") else {
            continue;
        };
        if effects.get("status").and_then(Value::as_str) != Some("SUCCESS") {
            continue;
        }
        let digest = tx
            .get("digest")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let ts = effects
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(rfc3339_millis)
            .unwrap_or(0);
        let sender = address_at(tx, "/sender/address");
        let payer = address_at(tx, "/gasInput/gasSponsor/address");
        let fee = gas_paid(effects);

        let changes: Vec<(String, &'static str, u8, i128)> = effects
            .get("balanceChangesJson")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(change).collect())
            .unwrap_or_default();

        for (owner, symbol, decimals, amount) in &changes {
            if *owner != mine || *amount == 0 {
                continue;
            }
            if *symbol == "SUI" && payer == mine && fee.is_some_and(|f| -*amount == f) {
                continue;
            }
            let direction = if *amount < 0 {
                Direction::Out
            } else {
                Direction::In
            };
            out.push(Transfer {
                direction,
                amount: amount.abs(),
                decimals: *decimals,
                symbol: symbol.to_string(),
                counterparty: counterparty(direction, symbol, &sender, &mine, &changes),
                block_ts: ts,
                id: digest.clone(),
            });
        }
    }
    // Stable, so transactions sharing a checkpoint keep the chain's order.
    out.sort_by_key(|t| std::cmp::Reverse(t.block_ts));
    out
}

/// Who was on the other side, when the transaction says so unambiguously.
///
/// Money in came from whoever signed. Money out went to the one other address
/// that gained the same coin; a swap through a pool or a payment split
/// several ways has no single answer, and a guess here is worse than none -
/// this is the field the address-poisoning check compares against.
fn counterparty(
    direction: Direction,
    symbol: &str,
    sender: &str,
    mine: &str,
    changes: &[(String, &'static str, u8, i128)],
) -> String {
    match direction {
        Direction::In if !sender.is_empty() && sender != mine => sender.to_string(),
        Direction::In => String::new(),
        Direction::Out => {
            let mut gained = changes
                .iter()
                .filter(|(o, s, _, a)| o != mine && *s == symbol && *a > 0)
                .map(|(o, ..)| o);
            match (gained.next(), gained.next()) {
                (Some(one), None) => one.clone(),
                _ => String::new(),
            }
        }
    }
}

/// One entry of `balanceChangesJson`: `{"address", "coinType", "amount"}`,
/// kept only for a coin this wallet shows.
fn change(v: &Value) -> Option<(String, &'static str, u8, i128)> {
    let (symbol, decimals) = v
        .get("coinType")
        .and_then(Value::as_str)
        .and_then(known_coin)?;
    let owner = v
        .get("address")
        .and_then(Value::as_str)?
        .to_ascii_lowercase();
    let amount = v.get("amount").and_then(int)?;
    Some((owner, symbol, decimals, amount))
}

/// What the gas payer was charged: computation plus storage, less the rebate
/// - exactly the payer's SUI change when the transaction moved nothing else.
fn gas_paid(effects: &Value) -> Option<i128> {
    let g = effects.pointer("/gasEffects/gasSummary")?;
    let n = |k: &str| g.get(k).and_then(int);
    Some(n("computationCost")? + n("storageCost")? - n("storageRebate")?)
}

fn address_at(v: &Value, path: &str) -> String {
    v.pointer(path)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_ascii_lowercase()
}

/// `2025-08-24T11:51:50.646Z` to milliseconds since the epoch.
///
/// The schema's `DateTime` is RFC 3339 in UTC, with the milliseconds optional.
/// The seconds-versus-milliseconds bug has been introduced twice already in
/// this wallet, so this answers in the unit every other chain uses.
fn rfc3339_millis(s: &str) -> Option<i64> {
    let s = s.strip_suffix('Z')?;
    let (date, time) = s.split_once('T')?;
    let mut d = date.splitn(3, '-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let mut t = time.splitn(3, ':');
    let (hh, mm, rest) = (t.next()?, t.next()?, t.next()?);
    let (ss, frac) = rest.split_once('.').unwrap_or((rest, ""));
    if !(1..=12).contains(&m) || !(1..=31).contains(&day) {
        return None;
    }
    let millis: i64 = frac
        .chars()
        .chain(std::iter::repeat('0'))
        .take(3)
        .collect::<String>()
        .parse()
        .ok()?;
    let secs = days_from_civil(y, m, day) * 86_400
        + hh.parse::<i64>().ok()? * 3_600
        + mm.parse::<i64>().ok()? * 60
        + ss.parse::<i64>().ok()?;
    Some(secs * 1_000 + millis)
}

/// Days since 1970-01-01, by Howard Hinnant's algorithm - no date library in
/// a wallet.
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SUI_LONG: &str =
        "0x0000000000000000000000000000000000000000000000000000000000000002::sui::SUI";

    fn me() -> SuiAddress {
        SuiAddress::parse("0x77776760c06997206b13fa76f127aa016d24a645f04fce516be153ece0bddf23")
            .unwrap()
    }

    const THEM: &str = "0x60dd01bc037e2c1ea2aaf02187701f9f4453ba323338d2f2f521957065b0984d";
    const OTHER: &str = "0xde1ed63a2f110159ddf1f4b33ab69ac40f470a2f13d85aff8defcfcf2beabaa6";

    /// One node of the reply, in the shape `graphql.mainnet.sui.io` answers.
    fn node(
        digest: &str,
        sender: &str,
        ok: bool,
        ts: &str,
        gas: (u64, u64, u64),
        changes: Value,
    ) -> Value {
        json!({
            "digest": digest,
            "sender": {"address": sender},
            "gasInput": {"gasSponsor": {"address": sender}},
            "effects": {
                "status": if ok {"SUCCESS"} else {"FAILURE"},
                "timestamp": ts,
                "balanceChangesJson": changes,
                "gasEffects": {"gasSummary": {
                    "computationCost": gas.0, "storageCost": gas.1, "storageRebate": gas.2
                }}
            }
        })
    }

    fn reply(nodes: Vec<Value>) -> Value {
        json!({"data": {"transactions": {"nodes": nodes}}})
    }

    fn ch(owner: &str, coin: &str, amount: &str) -> Value {
        json!({"address": owner, "coinType": coin, "amount": amount})
    }

    /// The sign of the change is the direction, and the amount shown is always
    /// positive. Trimmed from mainnet transaction `DAHxQrLH…`: one sender paid
    /// USDC to two addresses.
    #[test]
    fn the_sign_decides_the_direction() {
        let mine = me().to_string();
        let body = reply(vec![node(
            "DAHxQrLHUSVFrmW9CJ9LSzVxXu6NcMDtfzuMU2qscWdw",
            THEM,
            true,
            "2025-10-24T14:20:14.520Z",
            (594_000, 4_955_200, 2_287_296),
            json!([
                ch(THEM, SUI_LONG, "-3261904"),
                ch(THEM, crate::USDC_TYPE, "-6589894600"),
                ch(&mine, crate::USDC_TYPE, "3500000000"),
                ch(OTHER, crate::USDC_TYPE, "3089894600"),
            ]),
        )]);
        let out = parse(&body, me());
        assert_eq!(out.len(), 1, "only this address's row: {out:?}");
        assert_eq!(out[0].direction, Direction::In);
        assert_eq!(out[0].amount, 3_500_000_000);
        assert_eq!(out[0].symbol, "USDC");
        assert_eq!(out[0].decimals, 6);
        assert_eq!(out[0].counterparty, THEM, "money in came from the signer");

        let sent = reply(vec![node(
            "AZQmC99RJCbpaRT7irqj",
            &mine,
            true,
            "2025-10-24T14:20:14.520Z",
            (594_000, 4_955_200, 2_287_296),
            json!([
                ch(&mine, SUI_LONG, "-1003261904"),
                ch(THEM, SUI_LONG, "1000000000"),
            ]),
        )]);
        let out = parse(&sent, me());
        assert_eq!(out[0].direction, Direction::Out);
        assert_eq!(out[0].amount, 1_003_261_904, "the amount is not negative");
        assert_eq!(out[0].symbol, "SUI");
        assert_eq!(out[0].decimals, 9);
        assert_eq!(out[0].counterparty, THEM, "the one address that gained it");
    }

    /// A payment split several ways has no single counterparty, and a guess
    /// would feed the address-poisoning check something false.
    #[test]
    fn an_ambiguous_counterparty_is_left_blank() {
        let mine = me().to_string();
        let body = reply(vec![node(
            "x",
            &mine,
            true,
            "2025-10-24T14:20:14.520Z",
            (1, 1, 0),
            json!([
                ch(&mine, crate::USDC_TYPE, "-2000"),
                ch(THEM, crate::USDC_TYPE, "1000"),
                ch(OTHER, crate::USDC_TYPE, "1000"),
            ]),
        )]);
        assert_eq!(parse(&body, me())[0].counterparty, "");
    }

    /// Someone else's balance change in the same transaction is not ours.
    #[test]
    fn another_address_in_the_same_transaction_is_skipped() {
        let body = reply(vec![node(
            "x",
            THEM,
            true,
            "2025-10-24T14:20:14.520Z",
            (1, 1, 0),
            json!([ch(
                "0x0000000000000000000000000000000000000000000000000000000000000009",
                crate::SUI_TYPE,
                "-1"
            )]),
        )]);
        assert!(parse(&body, me()).is_empty());
    }

    /// A coin type this wallet does not carry is dropped rather than rendered
    /// under whatever name its publisher chose.
    #[test]
    fn an_unknown_coin_is_not_shown_under_a_name_it_chose() {
        let body = reply(vec![node(
            "x",
            THEM,
            true,
            "2025-10-24T14:20:14.520Z",
            (1, 1, 0),
            json!([ch(&me().to_string(), "0xdead::sui::SUI", "1000")]),
        )]);
        assert!(parse(&body, me()).is_empty());
        assert_eq!(known_coin("0xdead::sui::SUI"), None);
        assert_eq!(known_coin(crate::SUI_TYPE).unwrap().0, "SUI");
        assert_eq!(known_coin(SUI_LONG).unwrap().0, "SUI");
    }

    /// A failed transaction moved nothing but its fee.
    #[test]
    fn a_failed_transaction_is_not_a_transfer() {
        let body = reply(vec![node(
            "x",
            &me().to_string(),
            false,
            "2025-10-24T14:20:14.520Z",
            (1, 1, 0),
            json!([ch(&me().to_string(), SUI_LONG, "-1000")]),
        )]);
        assert!(parse(&body, me()).is_empty());
    }

    /// Paying a fee is not a payment. From mainnet transaction `4Soe98Nf…`,
    /// a contract call whose only SUI movement was its gas: 500000 + 135257200
    /// - 130300632 = 5456568.
    #[test]
    fn the_fee_of_a_transaction_that_sent_nothing_is_not_a_row() {
        let mine = me().to_string();
        let body = reply(vec![node(
            "4Soe98NfhxjLNTCrKR3fE3WyucBDUeVBf81ZWymFLUWN",
            &mine,
            true,
            "2025-08-24T11:51:50.646Z",
            (500_000, 135_257_200, 130_300_632),
            json!([ch(&mine, SUI_LONG, "-5456568")]),
        )]);
        assert!(parse(&body, me()).is_empty());
    }

    /// Newest first, though the connection answers oldest first.
    #[test]
    fn newest_comes_first() {
        let mine = me().to_string();
        let row = |d: &str, ts: &str| {
            node(
                d,
                THEM,
                true,
                ts,
                (1, 1, 0),
                json!([ch(&mine, crate::USDC_TYPE, "1")]),
            )
        };
        let body = reply(vec![
            row("old", "2025-07-24T06:41:55.036Z"),
            row("mid", "2026-11-07T11:44:56.014Z"),
            row("new", "2026-11-07T11:44:56.014Z"),
        ]);
        let ids: Vec<_> = parse(&body, me()).into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["new", "mid", "old"]);
    }

    /// The schema answers RFC 3339; every chain here reports milliseconds,
    /// and this pins the conversion so a seconds figure cannot slip through.
    #[test]
    fn timestamps_leave_in_milliseconds() {
        let mine = me().to_string();
        let body = reply(vec![node(
            "x",
            THEM,
            true,
            "2026-09-05T14:21:54.214Z",
            (1, 1, 0),
            json!([ch(&mine, SUI_LONG, "1")]),
        )]);
        assert_eq!(parse(&body, me())[0].block_ts, 1_788_618_114_214);
        assert_eq!(rfc3339_millis("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(
            rfc3339_millis("2000-03-01T00:00:01.5Z"),
            Some(951_868_801_500)
        );
        assert_eq!(rfc3339_millis("2000-03-01 00:00:01"), None);
    }
}
