//! What an address has done.
//!
//! Solana has no "list transfers for this address" call, and parsing
//! instructions to find them would mean understanding every program that might
//! move a balance. There is a better source: the cluster already records what
//! each account held before and after every transaction, so a transfer is a
//! *difference*, and reading it that way works for a transfer made by any
//! program at all.
//!
//! Two corrections that the raw difference needs, and that make it wrong if
//! they are skipped:
//!
//! * **The fee payer's balance also drops by the fee.** Subtracting it back out
//!   is the difference between "you sent 1 SOL" and "you sent 1.000005 SOL".
//! * **A failed transaction still charges the fee and moves nothing.** Its
//!   balances differ, and it is not a transfer.

use neko_hd::SolanaAddress;
use serde_json::{json, Value};

use crate::client::Rpc;
use crate::error::SolanaError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    In,
    Out,
}

#[derive(Debug, Clone)]
pub struct Transfer {
    pub signature: String,
    pub block_time: i64,
    pub symbol: String,
    pub decimals: u8,
    /// Always positive; `direction` carries the sign.
    pub amount: i128,
    pub direction: Direction,
    pub counterparty: String,
    pub failed: bool,
}

/// How many transactions to ask for in one JSON-RPC batch.
///
/// One request per transaction would be a round trip each. Ten was too many for
/// the public cluster, which answers a batch that size with 429 more often than
/// not; five gets through.
const BATCH: usize = 5;

impl Rpc {
    /// Recent transfers involving `addr`, newest first.
    pub async fn transfers(
        &self,
        addr: SolanaAddress,
        mint: SolanaAddress,
        limit: usize,
    ) -> Result<Vec<Transfer>, SolanaError> {
        let sigs = self.signatures_for(addr, limit).await?;
        let mut out = Vec::new();
        let mut last_err = None;
        for chunk in sigs.chunks(BATCH) {
            // A rate-limited batch loses a page of history, not the history. The
            // alternative - failing the whole read - turns a busy moment on a
            // public endpoint into a screen that says nothing at all.
            match self.get_transactions(chunk).await {
                Ok(txs) => {
                    for (sig, tx) in chunk.iter().zip(txs) {
                        if let Some(t) = extract(&tx, addr, mint, sig) {
                            out.extend(t);
                        }
                    }
                }
                Err(e) => last_err = Some(e),
            }
        }
        // ...but nothing at all, with a reason available, is an error rather
        // than an empty list. An empty list reads as "you have never used this
        // address", which is a different and much worse thing to say.
        match last_err {
            Some(e) if out.is_empty() => Err(e),
            _ => Ok(out),
        }
    }

    async fn signatures_for(
        &self,
        addr: SolanaAddress,
        limit: usize,
    ) -> Result<Vec<String>, SolanaError> {
        let v = self
            .call_public(
                "getSignaturesForAddress",
                json!([addr.to_string(), {"limit": limit.min(1000)}]),
            )
            .await?;
        Ok(v.as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.get("signature").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn get_transactions(&self, sigs: &[String]) -> Result<Vec<Value>, SolanaError> {
        let batch: Vec<Value> = sigs
            .iter()
            .enumerate()
            .map(|(i, s)| {
                json!({"jsonrpc": "2.0", "id": i, "method": "getTransaction",
                       "params": [s, {"encoding": "jsonParsed", "commitment": "confirmed",
                                      "maxSupportedTransactionVersion": 0}]})
            })
            .collect();
        self.call_batch(batch).await
    }
}

/// Everything this transaction did to `owner`, in SOL and in one token.
///
/// A transaction can touch both, so this returns a list rather than an option:
/// collapsing them would silently drop half of what happened.
pub fn extract(
    tx: &Value,
    owner: SolanaAddress,
    mint: SolanaAddress,
    signature: &str,
) -> Option<Vec<Transfer>> {
    let meta = tx.get("meta")?;
    let failed = !meta.get("err").map(Value::is_null).unwrap_or(true);
    let block_time = tx.get("blockTime").and_then(Value::as_i64).unwrap_or(0);

    let mut out = Vec::new();
    let owner_s = owner.to_string();

    // --- SOL ---
    let keys: Vec<String> = tx
        .get("transaction")?
        .get("message")?
        .get("accountKeys")?
        .as_array()?
        .iter()
        .filter_map(|k| {
            k.get("pubkey")
                .and_then(Value::as_str)
                .or_else(|| k.as_str())
                .map(str::to_string)
        })
        .collect();
    let pre: Vec<i128> = as_i128s(meta.get("preBalances"));
    let post: Vec<i128> = as_i128s(meta.get("postBalances"));
    let fee = meta.get("fee").and_then(Value::as_i64).unwrap_or(0) as i128;

    if let Some(i) = keys.iter().position(|k| *k == owner_s) {
        if let (Some(a), Some(b)) = (pre.get(i), post.get(i)) {
            let mut delta = b - a;
            // Index 0 is the fee payer, and its drop includes the fee. Without
            // this the amount reads high by exactly the fee on every send.
            if i == 0 {
                delta += fee;
            }
            if delta != 0 && !failed {
                let direction = if delta > 0 {
                    Direction::In
                } else {
                    Direction::Out
                };
                out.push(Transfer {
                    signature: signature.to_string(),
                    block_time,
                    symbol: "SOL".into(),
                    decimals: crate::chain_consts::SOL_DECIMALS,
                    amount: delta.abs(),
                    direction,
                    counterparty: other_party(&keys, &pre, &post, i, delta),
                    failed,
                });
            }
        }
    }

    // --- The token ---
    let mint_s = mint.to_string();
    let find = |field: &str| -> Option<i128> {
        meta.get(field)?.as_array()?.iter().find_map(|e| {
            (e.get("owner").and_then(Value::as_str) == Some(owner_s.as_str())
                && e.get("mint").and_then(Value::as_str) == Some(mint_s.as_str()))
            .then(|| {
                e.get("uiTokenAmount")
                    .and_then(|u| u.get("amount"))
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<i128>().ok())
                    .unwrap_or(0)
            })
        })
    };
    // A missing entry on one side is a zero on that side: an account that did
    // not exist before the transaction held nothing, which is exactly what a
    // first-ever receipt looks like.
    let (pre_t, post_t) = (find("preTokenBalances"), find("postTokenBalances"));
    if pre_t.is_some() || post_t.is_some() {
        let delta = post_t.unwrap_or(0) - pre_t.unwrap_or(0);
        if delta != 0 && !failed {
            let decimals = token_decimals(meta).unwrap_or(crate::chain_consts::USDT_DECIMALS);
            out.push(Transfer {
                signature: signature.to_string(),
                block_time,
                symbol: "USDT".into(),
                decimals,
                amount: delta.abs(),
                direction: if delta > 0 {
                    Direction::In
                } else {
                    Direction::Out
                },
                counterparty: token_other_party(meta, &owner_s, &mint_s, delta),
                failed,
            });
        }
    }

    Some(out)
}

fn as_i128s(v: Option<&Value>) -> Vec<i128> {
    v.and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_i64).map(i128::from).collect())
        .unwrap_or_default()
}

/// Who was on the other side.
///
/// This is not cosmetic. The counterparty is what the address-poisoning check
/// compares a destination against, and Solana is where that attack is most
/// common - a wallet with any history at all collects a stream of one-lamport
/// deposits from vanity addresses built to resemble somebody you pay.
///
/// Three ways to find them, in order of how much they claim:
///
/// 1. An account that moved by exactly the opposite amount. Unambiguous.
/// 2. Otherwise, for a receipt, the fee payer - who signed and paid for the
///    transaction that credited us. Their own balance moved by the amount *plus
///    the fee*, which is why step 1 misses them, and it is exactly what a
///    spammed dust deposit looks like.
/// 3. Otherwise, for a send, whichever account gained the most.
///
/// Empty when none of those applies, and the interface says "several" rather
/// than naming one - a transaction can pay many accounts, and picking one would
/// be inventing a fact.
fn other_party(keys: &[String], pre: &[i128], post: &[i128], mine: usize, delta: i128) -> String {
    let moved = |i: usize| match (pre.get(i), post.get(i)) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    };

    if let Some(k) = keys
        .iter()
        .enumerate()
        .filter(|(i, _)| *i != mine)
        .find(|(i, _)| moved(*i) == Some(-delta))
        .map(|(_, k)| k.clone())
    {
        return k;
    }

    // Received: the fee payer signed for it and paid the fee on top, so their
    // balance moved by more than ours did and never matches exactly.
    if delta > 0 && mine != 0 {
        if let Some(k) = keys.first() {
            return k.clone();
        }
    }

    // Sent: whoever gained the most, which for a plain transfer is the one
    // account that gained at all.
    if delta < 0 {
        return keys
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != mine)
            .filter_map(|(i, k)| moved(i).map(|d| (d, k)))
            .filter(|(d, _)| *d > 0)
            .max_by_key(|(d, _)| *d)
            .map(|(_, k)| k.clone())
            .unwrap_or_default();
    }

    String::new()
}

fn token_other_party(meta: &Value, owner: &str, mint: &str, delta: i128) -> String {
    let side = |field: &str, o: &str| -> Option<i128> {
        meta.get(field)?.as_array()?.iter().find_map(|e| {
            (e.get("owner").and_then(Value::as_str) == Some(o)
                && e.get("mint").and_then(Value::as_str) == Some(mint))
            .then(|| {
                e.get("uiTokenAmount")
                    .and_then(|u| u.get("amount"))
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<i128>().ok())
                    .unwrap_or(0)
            })
        })
    };
    let owners: Vec<String> = ["preTokenBalances", "postTokenBalances"]
        .iter()
        .filter_map(|f| meta.get(*f))
        .filter_map(Value::as_array)
        .flatten()
        .filter_map(|e| e.get("owner").and_then(Value::as_str))
        .filter(|o| *o != owner)
        .map(str::to_string)
        .collect();
    // Token balances carry no fee, so the opposite side matches exactly. When it
    // does not - a swap, or a route through several accounts - the largest move
    // the other way is the best available answer, and an empty string is the
    // honest one when even that is absent.
    let mut best: Option<(i128, String)> = None;
    for o in owners {
        let d =
            side("postTokenBalances", &o).unwrap_or(0) - side("preTokenBalances", &o).unwrap_or(0);
        if d == -delta {
            return o;
        }
        if d.signum() == -delta.signum() && best.as_ref().is_none_or(|(b, _)| d.abs() > b.abs()) {
            best = Some((d, o));
        }
    }
    best.map(|(_, o)| o).unwrap_or_default()
}

/// Precision as the cluster reported it for this transaction, rather than as
/// this crate assumes. One token name has three precisions across three chains.
fn token_decimals(meta: &Value) -> Option<u8> {
    ["postTokenBalances", "preTokenBalances"]
        .iter()
        .filter_map(|f| meta.get(*f))
        .filter_map(Value::as_array)
        .flatten()
        .find_map(|e| {
            e.get("uiTokenAmount")
                .and_then(|u| u.get("decimals"))
                .and_then(Value::as_u64)
                .map(|d| d as u8)
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ME: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
    const THEM: &str = "2ojv9BAiHUrvsm9gxDe7fJSzbNZSJcxZvf8dqmWGHG8S";
    const SYSTEM: &str = "11111111111111111111111111111111";
    const MINT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";

    fn me() -> SolanaAddress {
        SolanaAddress::parse(ME).unwrap()
    }
    fn mint() -> SolanaAddress {
        SolanaAddress::parse(MINT).unwrap()
    }

    fn tx(keys: &[&str], pre: &[i64], post: &[i64], fee: i64, err: Value) -> Value {
        json!({
            "blockTime": 1_756_000_000,
            "meta": {"err": err, "fee": fee, "preBalances": pre, "postBalances": post},
            "transaction": {"message": {"accountKeys":
                keys.iter().map(|k| json!({"pubkey": k})).collect::<Vec<_>>()}}
        })
    }

    fn token_entry(owner: &str, amount: &str) -> Value {
        json!({"owner": owner, "mint": MINT,
               "uiTokenAmount": {"amount": amount, "decimals": 6}})
    }

    /// The correction that decides whether "you sent 1 SOL" reads as
    /// 1.000005. The fee comes out of the same balance as the amount, and only
    /// for the account that signed.
    #[test]
    fn a_send_reports_the_amount_without_the_fee() {
        // We are the fee payer, so index 0.
        let t = tx(
            &[ME, THEM, SYSTEM],
            &[1_000_000_000, 0, 1],
            &[899_995_000, 100_000_000, 1],
            5_000,
            Value::Null,
        );
        let out = extract(&t, me(), mint(), "sig").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].direction, Direction::Out);
        assert_eq!(out[0].amount, 100_000_000, "the fee was counted as sent");
        assert_eq!(out[0].symbol, "SOL");
        assert_eq!(out[0].decimals, 9);
        assert_eq!(out[0].counterparty, THEM);
    }

    /// A receipt is credited in full: the sender paid the fee, not us.
    ///
    /// The counterparty here never matches by amount - the sender's balance
    /// dropped by the amount *plus* the fee - which is why the fee payer is the
    /// fallback rather than an empty string.
    #[test]
    fn a_receipt_names_the_sender_even_though_the_amounts_differ() {
        let t = tx(
            &[THEM, ME],
            &[1_000_000_000, 0],
            &[899_995_000, 100_000_000],
            5_000,
            Value::Null,
        );
        let out = extract(&t, me(), mint(), "sig").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].direction, Direction::In);
        assert_eq!(out[0].amount, 100_000_000);
        assert_eq!(
            out[0].counterparty, THEM,
            "without a counterparty the address-poisoning check has nothing to compare against"
        );
    }

    /// One lamport from a vanity address is what address poisoning looks like
    /// on this chain, and the sender has to be recorded or the lookalike
    /// warning can never fire.
    #[test]
    fn dust_poisoning_still_records_who_sent_it() {
        let t = tx(
            &[THEM, ME],
            &[1_000_000_000, 500],
            &[999_994_999, 501],
            5_000,
            Value::Null,
        );
        let out = extract(&t, me(), mint(), "sig").unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].amount, 1);
        assert_eq!(out[0].counterparty, THEM);
    }

    /// A failed transaction still charges the fee, so its balances differ. It
    /// moved nothing, and listing it as a transfer would be a lie about money.
    #[test]
    fn a_failed_transaction_is_not_a_transfer() {
        let t = tx(
            &[ME, THEM, SYSTEM],
            &[1_000_000_000, 0, 1],
            &[999_995_000, 0, 1],
            5_000,
            json!({"InstructionError": [0, {"Custom": 1}]}),
        );
        assert!(extract(&t, me(), mint(), "sig").unwrap().is_empty());
    }

    /// The first time an address receives a token it has no prior balance, so
    /// there is no "before" entry at all. Reading that as zero is what makes a
    /// first receipt show up; treating it as missing data would hide it.
    #[test]
    fn a_first_token_receipt_has_no_before_entry() {
        let mut t = tx(
            &[THEM, ME],
            &[1_000_000_000, 0],
            &[997_955_720, 0],
            5_000,
            Value::Null,
        );
        t["meta"]["preTokenBalances"] = json!([]);
        t["meta"]["postTokenBalances"] = json!([token_entry(ME, "5000000")]);

        let out = extract(&t, me(), mint(), "sig").unwrap();
        let usdt: Vec<_> = out.iter().filter(|e| e.symbol == "USDT").collect();
        assert_eq!(usdt.len(), 1, "a first receipt was dropped");
        assert_eq!(usdt[0].direction, Direction::In);
        assert_eq!(usdt[0].amount, 5_000_000);
        assert_eq!(usdt[0].decimals, 6, "read from the cluster, not assumed");
    }

    /// One transaction can move both, and collapsing them would silently drop
    /// half of what happened.
    #[test]
    fn a_transaction_that_moves_both_reports_both() {
        let mut t = tx(
            &[ME, THEM, SYSTEM],
            &[1_000_000_000, 0, 1],
            &[899_995_000, 100_000_000, 1],
            5_000,
            Value::Null,
        );
        t["meta"]["preTokenBalances"] = json!([token_entry(ME, "5000000"), token_entry(THEM, "0")]);
        t["meta"]["postTokenBalances"] =
            json!([token_entry(ME, "1000000"), token_entry(THEM, "4000000")]);

        let out = extract(&t, me(), mint(), "sig").unwrap();
        assert_eq!(out.len(), 2);
        let sol = out.iter().find(|e| e.symbol == "SOL").unwrap();
        let usdt = out.iter().find(|e| e.symbol == "USDT").unwrap();
        assert_eq!(sol.amount, 100_000_000);
        assert_eq!(usdt.amount, 4_000_000);
        assert_eq!(usdt.direction, Direction::Out);
        // Token balances carry no fee, so this side matches exactly.
        assert_eq!(usdt.counterparty, THEM);
    }

    /// An address that was merely mentioned - a program, a rent payer - has not
    /// transferred anything, and must not appear in a list of transfers.
    #[test]
    fn an_unchanged_balance_is_not_a_transfer() {
        let t = tx(
            &[THEM, ME],
            &[1_000_000_000, 42],
            &[999_995_000, 42],
            5_000,
            Value::Null,
        );
        assert!(extract(&t, me(), mint(), "sig").unwrap().is_empty());
    }

    /// Another token's movement is not ours. The mint is matched, not assumed
    /// from the fact that some token moved.
    #[test]
    fn a_different_mint_is_ignored() {
        let mut t = tx(
            &[THEM, ME],
            &[1_000_000_000, 42],
            &[999_995_000, 42],
            5_000,
            Value::Null,
        );
        t["meta"]["preTokenBalances"] = json!([{
            "owner": ME, "mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "uiTokenAmount": {"amount": "0", "decimals": 6}}]);
        t["meta"]["postTokenBalances"] = json!([{
            "owner": ME, "mint": "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
            "uiTokenAmount": {"amount": "9000000", "decimals": 6}}]);
        assert!(extract(&t, me(), mint(), "sig").unwrap().is_empty());
    }
}
