//! What an address has done.
//!
//! A transaction here is not a transfer; it is a contract executing, with one
//! message in and some number out. What moved is in those messages, and a token
//! movement is not in the coin values at all - it is an opcode inside a
//! message body, sent between two contracts neither of which is the address
//! being asked about.

use serde_json::Value;

use crate::address::TonAddress;
use crate::error::TonError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

#[derive(Debug, Clone)]
pub struct Transfer {
    /// Transaction hash, in the base64 form toncenter and the explorers use.
    pub hash: String,
    pub block_time: i64,
    pub symbol: String,
    pub decimals: u8,
    /// Always positive; `direction` carries the sign.
    pub amount: i128,
    pub direction: Direction,
    pub counterparty: String,
}

/// Read a page of `getTransactions`.
///
/// Only coin movements, for now. A jetton transfer is an opcode inside a body
/// between two contracts, and reading it means decoding cells out of the reply
/// rather than the values the API hands over - so it is left out rather than
/// guessed at, and the balance still shows what is held.
pub fn parse(
    result: &Value,
    ours: &TonAddress,
    decimals: u8,
    symbol: &str,
) -> Result<Vec<Transfer>, TonError> {
    let rows = result
        .as_array()
        .ok_or_else(|| TonError::BadReply("transaction list is not an array".into()))?;
    let mine = ours.to_raw_string();

    let mut out = Vec::new();
    for t in rows {
        let hash = t
            .get("transaction_id")
            .and_then(|i| i.get("hash"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let time = t.get("utime").and_then(Value::as_i64).unwrap_or(0);

        // Coin arriving: the message that caused this transaction, if it
        // carried value and came from somewhere.
        if let Some(m) = t.get("in_msg") {
            let value = msg_value(m);
            let src = addr_of(m, "source");
            if value > 0 && !src.is_empty() {
                out.push(Transfer {
                    hash: hash.clone(),
                    block_time: time,
                    symbol: symbol.to_string(),
                    decimals,
                    amount: value,
                    direction: Direction::In,
                    counterparty: src,
                });
            }
        }

        // Coin leaving: the messages this contract sent.
        for m in t
            .get("out_msgs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let value = msg_value(m);
            let dest = addr_of(m, "destination");
            if value > 0 && dest != mine {
                out.push(Transfer {
                    hash: hash.clone(),
                    block_time: time,
                    symbol: symbol.to_string(),
                    decimals,
                    amount: value,
                    direction: Direction::Out,
                    counterparty: dest,
                });
            }
        }
    }
    Ok(out)
}

fn msg_value(m: &Value) -> i128 {
    m.get("value")
        .and_then(|v| {
            v.as_str()
                .and_then(|s| s.parse::<i128>().ok())
                .or_else(|| v.as_i64().map(i128::from))
        })
        .unwrap_or(0)
}

fn addr_of(m: &Value, field: &str) -> String {
    m.get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MINE: &str = "0:335996ba9cce6625ebfdd7019ced50927c1a08b1846a289a55efc37c01d77df9";
    const THEM: &str = "0:5661bcb42ba847235760ce9aaa2dfff103eb7365db06e5df053120bacb77ddfd";

    fn mine() -> TonAddress {
        TonAddress::parse(MINE).unwrap()
    }

    fn tx(in_msg: Value, out: Vec<Value>) -> Value {
        json!({
            "transaction_id": {"hash": "abc="},
            "utime": 1_788_581_626i64,
            "in_msg": in_msg,
            "out_msgs": out,
        })
    }

    #[test]
    fn coin_arriving_is_a_receipt() {
        let t = tx(
            json!({"source": THEM, "destination": MINE, "value": "1000000000"}),
            vec![],
        );
        let got = parse(&json!([t]), &mine(), 9, "GRAM").unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].direction, Direction::In);
        assert_eq!(got[0].amount, 1_000_000_000);
        assert_eq!(got[0].counterparty, THEM);
        assert_eq!(got[0].decimals, 9);
    }

    /// The message that *caused* a transaction carries no value when the
    /// wallet itself sent it - the outgoing messages do. Counting the external
    /// message as a receipt would invent income.
    #[test]
    fn coin_leaving_is_a_payment_and_the_trigger_is_not_a_receipt() {
        let t = tx(
            // An external message: no source, no value.
            json!({"source": "", "destination": MINE, "value": "0"}),
            vec![json!({"source": MINE, "destination": THEM, "value": "500000000"})],
        );
        let got = parse(&json!([t]), &mine(), 9, "GRAM").unwrap();
        assert_eq!(got.len(), 1, "the trigger was counted as a transfer");
        assert_eq!(got[0].direction, Direction::Out);
        assert_eq!(got[0].amount, 500_000_000);
        assert_eq!(got[0].counterparty, THEM);
    }

    /// A message a wallet sends to itself moved nothing.
    #[test]
    fn a_message_back_to_ourselves_is_not_a_payment() {
        let t = tx(
            json!({"source": "", "destination": MINE, "value": "0"}),
            vec![json!({"source": MINE, "destination": MINE, "value": "500000000"})],
        );
        assert!(parse(&json!([t]), &mine(), 9, "GRAM").unwrap().is_empty());
    }

    /// A transaction that carried no value at all is not a transfer.
    #[test]
    fn a_valueless_transaction_is_not_a_transfer() {
        let t = tx(
            json!({"source": THEM, "destination": MINE, "value": "0"}),
            vec![],
        );
        assert!(parse(&json!([t]), &mine(), 9, "GRAM").unwrap().is_empty());
    }
}
