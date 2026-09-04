//! What an address has done.
//!
//! Same idea as Solana's, for the same reason: rather than interpreting scripts,
//! read what changed. A transaction's effect on an address is the outputs
//! paying it minus the inputs spending from it, and that works whatever the
//! transaction was doing.
//!
//! The correction Bitcoin needs is its own: **change**. A transfer of 0.1 out of
//! a 1.0 coin creates a 0.9 output back to yourself, and counting that as a
//! receipt would report a payment nobody made. Netting the two sides removes it.

use neko_hd::BtcAddress;
use serde_json::Value;

use crate::error::BtcError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

#[derive(Debug, Clone)]
pub struct Transfer {
    pub txid: String,
    /// Unix seconds, or 0 while unconfirmed.
    pub block_time: i64,
    /// Satoshis, always positive; `direction` carries the sign.
    pub amount: i128,
    pub direction: Direction,
    pub counterparty: String,
    pub confirmed: bool,
}

/// Net effect of one transaction on `ours`.
///
/// `None` when nothing moved - a transaction that merely mentions the address,
/// or one whose change exactly cancels, is not a transfer.
pub fn extract(tx: &Value, ours: &BtcAddress) -> Option<Transfer> {
    let mine = ours.to_string();
    let txid = tx.get("txid")?.as_str()?.to_string();
    let status = tx.get("status");
    let confirmed = status
        .and_then(|s| s.get("confirmed"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let block_time = status
        .and_then(|s| s.get("block_time"))
        .and_then(Value::as_i64)
        .unwrap_or(0);

    let addr_of = |v: &Value| -> Option<String> {
        v.get("scriptpubkey_address")
            .and_then(Value::as_str)
            .map(str::to_string)
    };

    // Spent from us.
    let mut spent: i128 = 0;
    let mut senders: Vec<String> = Vec::new();
    if let Some(vin) = tx.get("vin").and_then(Value::as_array) {
        for i in vin {
            let Some(prevout) = i.get("prevout") else {
                continue;
            };
            let value = prevout.get("value").and_then(Value::as_u64).unwrap_or(0) as i128;
            match addr_of(prevout) {
                Some(a) if a == mine => spent += value,
                Some(a) => senders.push(a),
                None => {}
            }
        }
    }

    // Paid to us. Everything else is where the money went.
    let mut received: i128 = 0;
    let mut recipient_total: i128 = 0;
    let mut recipients: Vec<String> = Vec::new();
    if let Some(vout) = tx.get("vout").and_then(Value::as_array) {
        for o in vout {
            let value = o.get("value").and_then(Value::as_u64).unwrap_or(0) as i128;
            match addr_of(o) {
                Some(a) if a == mine => received += value,
                Some(a) => {
                    recipient_total += value;
                    recipients.push(a);
                }
                // An output with no address - OP_RETURN, or a script Esplora
                // could not name. It is still money that left.
                None => recipient_total += value,
            }
        }
    }

    // The netting is what removes change: an output back to ourselves is on
    // both sides of this subtraction.
    let delta = received - spent;
    if delta == 0 {
        return None;
    }

    // What the amount *means* differs by direction, and the obvious choice is
    // wrong one way round.
    //
    // Incoming, it is the net: what arrived.
    //
    // Outgoing, the net would be the payment *plus the fee* - so somebody who
    // sent 0.3 would see 0.35 and reasonably conclude the wallet had sent the
    // wrong amount. What went to the other party is the honest figure, and the
    // fee is the difference, shown where fees are shown.
    let paid_out: i128 = recipient_total;
    let (direction, amount, counterparty) = if delta > 0 {
        (
            Direction::In,
            delta,
            senders.first().cloned().unwrap_or_default(),
        )
    } else if paid_out > 0 {
        (
            Direction::Out,
            paid_out,
            recipients.first().cloned().unwrap_or_default(),
        )
    } else {
        // Every output came back to us: a consolidation. The only thing that
        // actually left is the fee, and reporting it as a transfer of zero
        // would hide a real cost.
        (Direction::Out, -delta, String::new())
    };

    Some(Transfer {
        txid,
        block_time,
        amount,
        direction,
        counterparty,
        confirmed,
    })
}

/// Every transfer in a reply from `/address/{addr}/txs`.
pub fn parse(txs: &Value, ours: &BtcAddress) -> Result<Vec<Transfer>, BtcError> {
    let rows = txs
        .as_array()
        .ok_or_else(|| BtcError::BadReply("transaction list is not an array".into()))?;
    Ok(rows.iter().filter_map(|t| extract(t, ours)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const MINE: &str = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu";
    const THEM: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
    const OTHER: &str = "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa";

    fn mine() -> BtcAddress {
        BtcAddress::parse(MINE).unwrap()
    }
    fn out(addr: &str, value: u64) -> Value {
        json!({"scriptpubkey_address": addr, "value": value})
    }
    fn vin(addr: &str, value: u64) -> Value {
        json!({"prevout": {"scriptpubkey_address": addr, "value": value}})
    }
    fn tx(vins: Vec<Value>, vouts: Vec<Value>) -> Value {
        json!({
            "txid": "aa".repeat(32),
            "status": {"confirmed": true, "block_time": 1_756_000_000},
            "vin": vins,
            "vout": vouts,
        })
    }

    #[test]
    fn a_receipt_is_what_arrived() {
        let t = tx(
            vec![vin(THEM, 500_000)],
            vec![out(MINE, 100_000), out(THEM, 395_000)],
        );
        let r = extract(&t, &mine()).unwrap();
        assert_eq!(r.direction, Direction::In);
        assert_eq!(r.amount, 100_000);
        assert_eq!(r.counterparty, THEM);
    }

    /// The correction that matters. Spending a 1,000,000 coin to pay 300,000
    /// leaves 695,000 in change and 5,000 in fees. The net is 305,000; the
    /// payment is 300,000. Reporting the net would tell somebody the wallet
    /// sent more than they asked it to.
    #[test]
    fn a_send_reports_the_payment_not_the_payment_plus_the_fee() {
        let t = tx(
            vec![vin(MINE, 1_000_000)],
            vec![out(THEM, 300_000), out(MINE, 695_000)],
        );
        let r = extract(&t, &mine()).unwrap();
        assert_eq!(r.direction, Direction::Out);
        assert_eq!(r.amount, 300_000, "the fee was counted as sent");
        assert_eq!(r.counterparty, THEM);
    }

    /// Change is not a receipt. Without netting, the 695,000 above appears as
    /// money arriving.
    #[test]
    fn change_is_not_reported_as_income() {
        let t = tx(
            vec![vin(MINE, 1_000_000)],
            vec![out(THEM, 300_000), out(MINE, 695_000)],
        );
        let r = extract(&t, &mine()).unwrap();
        assert_ne!(r.direction, Direction::In, "change was read as a receipt");
    }

    /// Several coins spent to pay several people. The payment is everything
    /// that went elsewhere.
    #[test]
    fn a_payment_to_several_people_sums_them() {
        let t = tx(
            vec![vin(MINE, 400_000), vin(MINE, 400_000)],
            vec![out(THEM, 250_000), out(OTHER, 250_000), out(MINE, 295_000)],
        );
        let r = extract(&t, &mine()).unwrap();
        assert_eq!(r.direction, Direction::Out);
        assert_eq!(r.amount, 500_000);
    }

    /// Consolidating your own coins pays nobody, and the only thing that leaves
    /// is the fee. Reporting nothing would hide a real cost.
    #[test]
    fn a_consolidation_reports_the_fee() {
        let t = tx(
            vec![vin(MINE, 100_000), vin(MINE, 100_000)],
            vec![out(MINE, 195_000)],
        );
        let r = extract(&t, &mine()).unwrap();
        assert_eq!(r.direction, Direction::Out);
        assert_eq!(r.amount, 5_000, "the fee is what left");
        assert!(r.counterparty.is_empty(), "there was no counterparty");
    }

    /// A transaction that merely mentions the address is not a transfer.
    #[test]
    fn an_untouched_address_has_no_transfer() {
        let t = tx(vec![vin(THEM, 500_000)], vec![out(OTHER, 495_000)]);
        assert!(extract(&t, &mine()).is_none());
    }

    /// Money burned to an OP_RETURN has no address, and still left.
    #[test]
    fn an_output_with_no_address_still_counts_as_paid() {
        let t = tx(
            vec![vin(MINE, 100_000)],
            vec![json!({"value": 40_000}), out(MINE, 55_000)],
        );
        let r = extract(&t, &mine()).unwrap();
        assert_eq!(r.direction, Direction::Out);
        assert_eq!(r.amount, 40_000);
    }

    /// Unconfirmed is a state worth carrying: the transaction can still be
    /// replaced, and a wallet that showed it as settled would be wrong.
    #[test]
    fn unconfirmed_is_reported_as_such() {
        let mut t = tx(vec![vin(THEM, 500_000)], vec![out(MINE, 100_000)]);
        t["status"] = json!({"confirmed": false});
        let r = extract(&t, &mine()).unwrap();
        assert!(!r.confirmed);
        assert_eq!(r.block_time, 0, "an unconfirmed transaction has no time");
    }
}
