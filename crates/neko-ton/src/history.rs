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

/// Read a page of `getTransactions` for a wallet: its coin movements.
///
/// Tokens are not here. They move between *jetton wallet* contracts, neither of
/// which is this address, so nothing about a USDT transfer appears in what this
/// address did. [`parse_jetton`] reads those from the token's own contract.
///
/// `jetton_wallet`, when known, is our own token contract. Coin legs to and
/// from it are the plumbing of a token transfer - the 0.05 GRAM that travels
/// with one, and the change coming back - and they are dropped, because the
/// transfer they belong to is shown as a token transfer already.
pub fn parse(
    result: &Value,
    ours: &TonAddress,
    jetton_wallet: Option<&TonAddress>,
    decimals: u8,
    symbol: &str,
) -> Result<Vec<Transfer>, TonError> {
    let rows = result
        .as_array()
        .ok_or_else(|| TonError::BadReply("transaction list is not an array".into()))?;
    let mine = ours.to_raw_string();
    let plumbing = jetton_wallet.map(TonAddress::to_raw_string);
    let is_plumbing = |a: &str| plumbing.as_deref().is_some_and(|w| same_address(a, w));

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
        // Whether this transaction happened because *we* asked for it. A
        // wallet acts on an external message - that is what signing produces -
        // and anything with an internal source was somebody else's doing.
        let in_msg = t.get("in_msg");
        let ours_to_send = in_msg.is_none_or(|m| addr_of(m, "source").is_empty());

        if let Some(m) = in_msg {
            let value = msg_value(m);
            let src = addr_of(m, "source");
            if value > 0 && !src.is_empty() && !is_plumbing(&src) {
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

        // Coin leaving: the messages this contract sent *because we told it
        // to*.
        //
        // A wallet emits messages it was not asked for. The commonest is a
        // bounce: a payment arrives at an address whose contract does not exist
        // yet, cannot be delivered, and the chain returns what is left. That is
        // an out_msg carrying value, and reading it as a payment tells somebody
        // they sent money they never sent - which is what this did, on the
        // first wallet that received anything before it had ever spent.
        if !ours_to_send {
            continue;
        }
        for m in t
            .get("out_msgs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let value = msg_value(m);
            let dest = addr_of(m, "destination");
            if value > 0 && dest != mine && !is_plumbing(&dest) {
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

/// Read a page of `getTransactions` for a *jetton wallet*: its token movements.
///
/// Both directions arrive as the incoming message, which is the part that is
/// not obvious. Sending is an owner telling their own token contract to
/// `transfer`; receiving is another token contract performing an
/// `internal_transfer` into it. One field position, two meanings, and the
/// amount is in neither the message value nor any field the API parses - it is
/// inside the body, in bytes.
pub fn parse_jetton(result: &Value, decimals: u8, symbol: &str) -> Result<Vec<Transfer>, TonError> {
    let rows = result
        .as_array()
        .ok_or_else(|| TonError::BadReply("transaction list is not an array".into()))?;

    let mut out = Vec::new();
    for t in rows {
        let Some(m) = t.get("in_msg") else { continue };
        let Some(body) = body_cell(m) else { continue };
        // A body this code does not understand is not a failure: jetton
        // contracts carry burns, mints and custom payloads, and none of them
        // should cost the user the rows that did parse.
        let Ok(Some(mv)) = crate::jetton::parse_move(&body) else {
            continue;
        };
        if mv.amount == 0 {
            continue;
        }
        out.push(Transfer {
            hash: t
                .get("transaction_id")
                .and_then(|i| i.get("hash"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            block_time: t.get("utime").and_then(Value::as_i64).unwrap_or(0),
            symbol: symbol.to_string(),
            decimals,
            amount: mv.amount as i128,
            direction: if mv.outgoing {
                Direction::Out
            } else {
                Direction::In
            },
            counterparty: mv
                .counterparty
                .map(|a| a.to_friendly_string())
                .unwrap_or_default(),
        });
    }
    Ok(out)
}

/// The body of a message, as a cell. Absent for a plain coin transfer, and text
/// for one carrying a comment - neither of which is a token movement.
fn body_cell(m: &Value) -> Option<std::sync::Arc<crate::cell::Cell>> {
    let b64 = m.get("msg_data")?.get("body")?.as_str()?;
    if b64.is_empty() {
        return None;
    }
    crate::boc::parse(&crate::b64::decode(b64)?).ok()
}

/// Whether two addresses the API wrote are the same one.
///
/// It answers in both forms - `0:abc…` in one field and `EQ…` in another - so
/// comparing the strings alone misses half the matches.
fn same_address(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    match (TonAddress::parse(a), TonAddress::parse(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
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
        let got = parse(&json!([t]), &mine(), None, 9, "GRAM").unwrap();
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
        let got = parse(&json!([t]), &mine(), None, 9, "GRAM").unwrap();
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
        assert!(parse(&json!([t]), &mine(), None, 9, "GRAM")
            .unwrap()
            .is_empty());
    }

    /// A transaction that carried no value at all is not a transfer.
    #[test]
    fn a_valueless_transaction_is_not_a_transfer() {
        let t = tx(
            json!({"source": THEM, "destination": MINE, "value": "0"}),
            vec![],
        );
        assert!(parse(&json!([t]), &mine(), None, 9, "GRAM")
            .unwrap()
            .is_empty());
    }
}

#[cfg(test)]
mod real_replies {
    use super::*;
    use serde_json::json;

    /// The wallet from the report: it had received twice and never sent.
    const ZOE: &str = "EQAr-jlxrYwVBJIBJ7hoJYbcK7t0SA3_0ubMuNC4iY7Ng6Ur";
    /// Its USDT contract.
    const ZOE_JETTON: &str = "0:BC274440691AAA34037213C2BAD1939C56307D7CE57B4A634FC25853013A45BA";
    /// Who paid her.
    const PAYER: &str = "EQDKHZ7e70CzqdvZCC83Z4WVR8POC_ZB0J1Y4zo88G-zCXmC";
    /// Her jetton wallet, which sent the bounce back.
    const HER_USDT_NEIGHBOUR: &str = "EQBF9ljVuJ5qeUYDOnZQwd6IRgHZWDXmQ3Y_-S_yWm00GU3J";

    /// The transaction that was reported as an outgoing payment.
    ///
    /// Nothing was sent. A message arrived at a wallet whose contract did not
    /// exist yet, could not be delivered, and the chain returned what was left
    /// of it - 6,421 nanotons of the 73,088 that came in, the rest having gone
    /// to fees. The body of the returned message begins `ffffffff`, which is
    /// how a bounce says what it is.
    fn the_bounce() -> Value {
        json!({
            "transaction_id": {"hash": "q/E6dZCK3mMQztRg+HtuXSfE0SVzHuirZAGMD5CbhUg="},
            "utime": 1_788_586_814i64,
            "fee": "66668",
            "in_msg": {
                "source": HER_USDT_NEIGHBOUR,
                "destination": ZOE,
                "value": "73088",
                "msg_data": {"@type": "msg.dataText", "text": "UmVmdW5kLWZlZXM="},
            },
            "out_msgs": [{
                "source": ZOE,
                "destination": HER_USDT_NEIGHBOUR,
                "value": "6421",
                "msg_data": {
                    "@type": "msg.dataRaw",
                    "body": "te6cckEBAQEAFQAAJv////8AAAAAUmVmdW5kLWZlZXM7k6uU",
                },
            }],
        })
    }

    #[test]
    fn a_bounce_is_not_a_payment_the_user_made() {
        let mine = TonAddress::parse(ZOE).unwrap();
        let rows = parse(&json!([the_bounce()]), &mine, None, 9, "GRAM").unwrap();
        assert!(
            !rows.iter().any(|r| r.direction == Direction::Out),
            "a bounced message was reported as an outgoing payment: {rows:?}"
        );
        // The incoming leg is real - the coins did arrive before they were
        // returned - and stays. It is dust, and the poisoning filter above this
        // is what decides whether dust is worth a row.
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].direction, Direction::In);
        assert_eq!(rows[0].amount, 73_088);
    }

    /// A transfer the wallet really did make still shows. The difference is
    /// what triggered the transaction: signing produces an *external* message,
    /// which has no source.
    #[test]
    fn a_transfer_we_signed_still_shows() {
        let mine = TonAddress::parse(ZOE).unwrap();
        let tx = json!({
            "transaction_id": {"hash": "abc="},
            "utime": 1_788_586_900i64,
            // No source: this transaction happened because a signature arrived.
            "in_msg": {"destination": ZOE, "value": "0"},
            "out_msgs": [{
                "source": ZOE,
                "destination": PAYER,
                "value": "500000000",
            }],
        });
        let rows = parse(&json!([tx]), &mine, None, 9, "GRAM").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].direction, Direction::Out);
        assert_eq!(rows[0].amount, 500_000_000);
        assert_eq!(rows[0].counterparty, PAYER);
    }

    /// The USDT that was received and not shown.
    ///
    /// These are the exact bytes toncenter returned for the incoming message on
    /// her USDT contract: `internal_transfer`, 2,700,000 units - 2.7 USDT - from
    /// the address that had also sent her the GRAM.
    #[test]
    fn the_usdt_that_arrived_is_read_out_of_the_body() {
        let tx = json!({
            "transaction_id": {"hash": "dQNHHwhmxivC+wGFeMSWMvf2uEs6n66B6b364IjA//Q="},
            "utime": 1_788_586_801i64,
            "in_msg": {
                "source": "EQC7aZ-_G_tWeSn0GZ0HclwZvGIBp-CRrSsbMibTHN6l4kr7",
                "destination": ZOE_JETTON,
                "value": "49741198",
                "msg_data": {
                    "@type": "msg.dataRaw",
                    "body": "te6cckEBAQEAVgAApxeNRRkAAABfEo7ztjKTLggBlDs9vd6BZ1O3shBebs8LKo+HnBfsg6E6scZ0eeDfZhMAModnt7vQLOp29kILzdnhZVHw84L9kHQnVjjOjzwb7MJEBfVjfcw=",
                },
            },
            "out_msgs": [],
        });
        let rows = parse_jetton(&json!([tx]), 6, "USDT").unwrap();
        assert_eq!(rows.len(), 1, "the token transfer was not read: {rows:?}");
        assert_eq!(rows[0].direction, Direction::In);
        assert_eq!(rows[0].amount, 2_700_000, "2.7 USDT");
        assert_eq!(rows[0].symbol, "USDT");
        assert_eq!(rows[0].decimals, 6);
        assert_eq!(rows[0].counterparty, PAYER, "who paid her");
    }

    /// A notification and an excess refund travel out of the same contract in
    /// the same transaction and move no tokens. Counting either would double
    /// the row above.
    ///
    /// Both bodies are the real ones. That matters more than it looks: a body
    /// that fails to parse also yields no rows, so a fabricated one would make
    /// this test pass while proving nothing. The op is asserted first, which
    /// only succeeds if the bytes really did decode.
    #[test]
    fn a_notification_or_a_refund_is_not_a_token_movement() {
        for (body, op, what) in [
            (
                "te6cckEBAQEAMwAAYnNi0JwAAABfEo7ztjKTLggBlDs9vd6BZ1O3shBebs8LKo+HnBfsg6E6scZ0eeDfZhLlxclb",
                crate::jetton::OP_TRANSFER_NOTIFICATION,
                "the wallet being told tokens arrived",
            ),
            (
                "te6cckEBAQEADgAAGNUydtsAAABfEo7ztrTx99k=",
                crate::jetton::OP_EXCESSES,
                "unused coin going back to whoever paid",
            ),
        ] {
            let cell = crate::boc::parse(&crate::b64::decode(body).unwrap())
                .unwrap_or_else(|e| panic!("{what}: the pinned body is not a cell: {e}"));
            let mut s = crate::dict::Slice::new(&cell);
            assert_eq!(
                s.load_uint(32).unwrap() as u32,
                op,
                "{what}: this is not the message it claims to be"
            );

            let tx = json!({
                "transaction_id": {"hash": "x="},
                "utime": 1i64,
                "in_msg": {"source": ZOE_JETTON, "value": "1",
                           "msg_data": {"@type": "msg.dataRaw", "body": body}},
                "out_msgs": [],
            });
            let rows = parse_jetton(&json!([tx]), 6, "USDT").unwrap();
            assert!(rows.is_empty(), "{what} was counted as a movement: {rows:?}");
        }
    }

    /// A body that is not a cell yields no rows *and* no panic - and, unlike
    /// the messages above, it never decoded at all. The distinction is why the
    /// op is asserted there: both cases produce an empty list, so an empty list
    /// on its own proves nothing.
    #[test]
    fn a_body_that_is_not_a_cell_is_skipped_rather_than_read() {
        let tx = json!({
            "transaction_id": {"hash": "x="},
            "utime": 1i64,
            "in_msg": {"source": ZOE_JETTON, "value": "1",
                       "msg_data": {"@type": "msg.dataRaw", "body": "bm90IGEgY2VsbA=="}},
            "out_msgs": [],
        });
        assert!(crate::boc::parse(&crate::b64::decode("bm90IGEgY2VsbA==").unwrap()).is_err());
        assert!(parse_jetton(&json!([tx]), 6, "USDT").unwrap().is_empty());
    }

    /// The coin leg of a token transfer is not a coin transfer.
    ///
    /// Sending USDT means sending 0.05 GRAM to your own token contract and
    /// getting most of it back. Both legs are real and neither is something the
    /// user did: the transfer they made is the USDT one, and it is shown.
    #[test]
    fn the_coin_that_carries_a_token_transfer_is_not_a_row_of_its_own() {
        let mine = TonAddress::parse(ZOE).unwrap();
        let jetton = TonAddress::parse(ZOE_JETTON).unwrap();
        let tx = json!({
            "transaction_id": {"hash": "abc="},
            "utime": 1_788_586_900i64,
            "in_msg": {"destination": ZOE, "value": "0"},
            "out_msgs": [{
                "source": ZOE,
                "destination": jetton.to_friendly_string(),
                "value": "50000000",
            }],
        });
        let with = parse(&json!([tx]), &mine, Some(&jetton), 9, "GRAM").unwrap();
        assert!(with.is_empty(), "the attached coin got a row: {with:?}");

        // And without knowing the token contract there is nothing to suppress,
        // so the leg shows - which is why the address is passed in.
        let without = parse(&json!([tx]), &mine, None, 9, "GRAM").unwrap();
        assert_eq!(without.len(), 1);
    }
}
