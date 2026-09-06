//! Transfer history, from the node's balance changes.
//!
//! Sui reports, per transaction, how each address's holding of each coin type
//! moved. That is exactly what a history screen wants and it saves decoding
//! the programmable block that caused it - a payment can be any shape at all
//! here, so reading the *effect* is both simpler and more honest than trying
//! to recognise the shape.
//!
//! Two queries, because the node filters by sender or by recipient and a
//! wallet needs both. Timestamps arrive in milliseconds already, which makes
//! this the one chain here that needs no unit conversion.

use serde_json::Value;

use crate::address::SuiAddress;

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

/// Turn a page of transaction blocks into transfers.
///
/// A transaction that failed changed no balances and is dropped. So is a
/// change of zero, and so is the gas-only movement of a transaction that sent
/// nothing - paying a fee is not a payment.
pub fn parse(body: &Value, who: SuiAddress) -> Vec<Transfer> {
    let mine = who.to_string();
    let rows = body
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut out = Vec::new();
    for tx in &rows {
        let ok = tx
            .get("effects")
            .and_then(|e| e.get("status"))
            .and_then(|s| s.get("status"))
            .and_then(Value::as_str)
            .map(|s| s == "success")
            .unwrap_or(false);
        if !ok {
            continue;
        }
        let digest = tx
            .get("digest")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let ts = tx
            .get("timestampMs")
            .and_then(|v| match v {
                Value::String(s) => s.parse::<i64>().ok(),
                Value::Number(n) => n.as_i64(),
                _ => None,
            })
            .unwrap_or(0);

        for ch in tx
            .get("balanceChanges")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let owner = ch
                .get("owner")
                .and_then(|o| o.get("AddressOwner"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            if owner != mine {
                continue;
            }
            let Some((symbol, decimals)) = ch
                .get("coinType")
                .and_then(Value::as_str)
                .and_then(known_coin)
            else {
                continue;
            };
            let amount: i128 = match ch.get("amount") {
                Some(Value::String(s)) => s.parse().unwrap_or(0),
                Some(Value::Number(n)) => n.as_i64().map(i128::from).unwrap_or(0),
                _ => 0,
            };
            if amount == 0 {
                continue;
            }
            out.push(Transfer {
                direction: if amount < 0 {
                    Direction::Out
                } else {
                    Direction::In
                },
                amount: amount.abs(),
                decimals,
                symbol: symbol.to_string(),
                counterparty: String::new(),
                block_ts: ts,
                id: digest.clone(),
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn me() -> SuiAddress {
        SuiAddress::parse("0x77776760c06997206b13fa76f127aa016d24a645f04fce516be153ece0bddf23")
            .unwrap()
    }

    fn tx(amount: &str, coin: &str, owner: &str, ok: bool) -> Value {
        json!({
            "digest": "AZQmC99RJCbpaRT7irqj",
            "timestampMs": "1788637314214",
            "effects": {"status": {"status": if ok {"success"} else {"failure"}}},
            "balanceChanges": [
                {"coinType": coin, "amount": amount, "owner": {"AddressOwner": owner}}
            ]
        })
    }

    /// The sign of the change is the direction, and the amount shown is always
    /// positive.
    #[test]
    fn the_sign_decides_the_direction() {
        let body = json!({"data": [
            tx("-612088", crate::SUI_TYPE, &me().to_string(), true),
            tx("2500000", crate::USDC_TYPE, &me().to_string(), true),
        ]});
        let out = parse(&body, me());
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].direction, Direction::Out);
        assert_eq!(out[0].amount, 612_088, "the amount is not negative");
        assert_eq!(out[0].symbol, "SUI");
        assert_eq!(out[0].decimals, 9);
        assert_eq!(out[1].direction, Direction::In);
        assert_eq!(out[1].symbol, "USDC");
        assert_eq!(out[1].decimals, 6);
    }

    /// Someone else's balance change in the same transaction is not ours.
    #[test]
    fn another_address_in_the_same_transaction_is_skipped() {
        let body = json!({"data": [tx(
            "-1", crate::SUI_TYPE,
            "0x0000000000000000000000000000000000000000000000000000000000000009", true)]});
        assert!(parse(&body, me()).is_empty());
    }

    /// A coin type this wallet does not carry is dropped rather than rendered
    /// under whatever name its publisher chose.
    #[test]
    fn an_unknown_coin_is_not_shown_under_a_name_it_chose() {
        let body = json!({"data": [tx(
            "1000", "0xdead::sui::SUI", &me().to_string(), true)]});
        assert!(parse(&body, me()).is_empty());
        assert_eq!(known_coin("0xdead::sui::SUI"), None);
        assert_eq!(known_coin(crate::SUI_TYPE).unwrap().0, "SUI");
    }

    /// A failed transaction moved nothing.
    #[test]
    fn a_failed_transaction_is_not_a_transfer() {
        let body = json!({"data": [tx("-1000", crate::SUI_TYPE, &me().to_string(), false)]});
        assert!(parse(&body, me()).is_empty());
    }

    /// Sui reports milliseconds already - the one chain here that needs no
    /// conversion, and worth pinning so nobody adds one.
    #[test]
    fn timestamps_arrive_in_milliseconds() {
        let body = json!({"data": [tx("-1", crate::SUI_TYPE, &me().to_string(), true)]});
        assert_eq!(parse(&body, me())[0].block_ts, 1_788_637_314_214);
    }
}
