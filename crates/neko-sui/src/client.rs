//! Sui's JSON-RPC.
//!
//! Asked for four things: what coin objects an address owns, what gas costs,
//! what a transaction would cost, and the submission itself.
//!
//! The public fullnode at `fullnode.mainnet.sui.io` has **deprecated**
//! JSON-RPC and answers "Method not found" to all of it, pointing at gRPC and
//! GraphQL. This wallet talks to a public node that still serves it. That is
//! worth knowing rather than discovering: the default endpoint here is not the
//! one Sui's own documentation names.

use serde_json::{json, Value};

use crate::address::SuiAddress;
use crate::error::SuiError;
use crate::tx::ObjectRef;

pub struct Rpc {
    url: String,
    http: reqwest::Client,
}

/// One coin object and what it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coin {
    pub object: ObjectRef,
    pub balance: u128,
}

impl Rpc {
    pub fn new(url: Option<&str>) -> Self {
        Rpc {
            url: url
                .map(str::trim)
                .filter(|u| !u.is_empty())
                .unwrap_or(crate::DEFAULT_API)
                .trim_end_matches('/')
                .to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.url
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, SuiError> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let r = self
            .http
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .map_err(|e| SuiError::Rpc(e.to_string()))?;
        let text = r.text().await.map_err(|e| SuiError::Rpc(e.to_string()))?;
        let v: Value = serde_json::from_str(&text).map_err(|_| SuiError::BadReply(cut(&text)))?;
        if let Some(e) = v.get("error") {
            let msg = e
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("the node refused the request");
            return Err(SuiError::Rpc(cut(msg)));
        }
        v.get("result")
            .cloned()
            .ok_or_else(|| SuiError::BadReply(cut(&text)))
    }

    /// The total held of one coin type, in its own units.
    pub async fn balance(&self, who: SuiAddress, coin_type: &str) -> Result<u128, SuiError> {
        let v = self
            .call("suix_getBalance", json!([who.to_string(), coin_type]))
            .await?;
        v.get("totalBalance")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| SuiError::BadReply(format!("no totalBalance in {v}")))
    }

    /// The coin objects themselves, newest page first.
    ///
    /// A transfer spends objects rather than a balance, so this is what a
    /// quote actually needs. Capped, because a transaction that folds together
    /// hundreds of objects is refused by the chain and the honest answer is to
    /// say so.
    pub async fn coins(&self, who: SuiAddress, coin_type: &str) -> Result<Vec<Coin>, SuiError> {
        let v = self
            .call(
                "suix_getCoins",
                json!([
                    who.to_string(),
                    coin_type,
                    Value::Null,
                    crate::MAX_COINS_PER_TRANSFER
                ]),
            )
            .await?;
        let rows = v
            .get("data")
            .and_then(Value::as_array)
            .ok_or_else(|| SuiError::BadReply(format!("no coin list in {v}")))?;
        rows.iter().map(parse_coin).collect()
    }

    /// What one unit of computation costs, from the chain's reference price.
    pub async fn reference_gas_price(&self) -> Result<u64, SuiError> {
        let v = self.call("suix_getReferenceGasPrice", json!([])).await?;
        match &v {
            Value::String(s) => s
                .parse()
                .map_err(|_| SuiError::BadReply(format!("gas price {s:?} is not a number"))),
            Value::Number(n) => n
                .as_u64()
                .ok_or_else(|| SuiError::BadReply(format!("gas price {n} is not a u64"))),
            other => Err(SuiError::BadReply(format!("gas price was {other}"))),
        }
    }

    /// Run a transaction without committing it, and report what it would cost.
    ///
    /// This is also what proves the bytes are right: a dry run that returns a
    /// cost has been parsed by the chain's own decoder.
    pub async fn dry_run(&self, data: &[u8]) -> Result<DryRun, SuiError> {
        let v = self
            .call("sui_dryRunTransactionBlock", json!([b64(data)]))
            .await?;
        let effects = v
            .get("effects")
            .ok_or_else(|| SuiError::BadReply(format!("no effects in {v}")))?;
        let status = effects
            .get("status")
            .and_then(|s| s.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        if status != "success" {
            let why = effects
                .get("status")
                .and_then(|s| s.get("error"))
                .and_then(Value::as_str)
                .unwrap_or(status);
            return Err(SuiError::Rpc(cut(why)));
        }
        let g = effects
            .get("gasUsed")
            .ok_or_else(|| SuiError::BadReply("no gasUsed".into()))?;
        let num = |k: &str| -> u64 {
            g.get(k)
                .and_then(|x| match x {
                    Value::String(s) => s.parse().ok(),
                    Value::Number(n) => n.as_u64(),
                    _ => None,
                })
                .unwrap_or(0)
        };
        // Storage is charged and then partly given back, so the net cost is
        // computation plus storage minus the rebate. Adding the first two and
        // ignoring the third overstates a transfer substantially.
        let cost = num("computationCost") + num("storageCost");
        Ok(DryRun {
            computation: num("computationCost"),
            storage: num("storageCost"),
            rebate: num("storageRebate"),
            net: cost.saturating_sub(num("storageRebate")),
        })
    }

    /// Transfers in both directions, newest first.
    ///
    /// Two queries, because the node filters by sender or by recipient and a
    /// wallet needs both. A failure on one leg does not discard the other: a
    /// history with only what you sent is still better than none, and losing
    /// what you *received* is the specific failure this wallet has shipped
    /// before.
    pub async fn transfers(
        &self,
        who: SuiAddress,
        limit: usize,
    ) -> Result<Vec<crate::history::Transfer>, SuiError> {
        let opts = json!({"showBalanceChanges": true, "showEffects": true});
        let (sent, received) = tokio::join!(
            self.call(
                "suix_queryTransactionBlocks",
                json!([{"filter": {"FromAddress": who.to_string()}, "options": opts},
                       Value::Null, limit, true]),
            ),
            self.call(
                "suix_queryTransactionBlocks",
                json!([{"filter": {"ToAddress": who.to_string()}, "options": opts},
                       Value::Null, limit, true]),
            )
        );

        let mut out = Vec::new();
        let mut first_error = None;
        for leg in [&sent, &received] {
            match leg {
                Ok(v) => out.extend(crate::history::parse(v, who)),
                Err(e) => {
                    if first_error.is_none() {
                        first_error = Some(e.to_string());
                    }
                }
            }
        }
        if out.is_empty() {
            if let Some(e) = first_error {
                return Err(SuiError::Rpc(e));
            }
        }
        // The same transaction can appear in both legs.
        out.sort_by_key(|t| (std::cmp::Reverse(t.block_ts), t.id.clone()));
        out.dedup_by(|a, b| a.id == b.id && a.symbol == b.symbol && a.amount == b.amount);
        out.truncate(limit);
        Ok(out)
    }

    /// Broadcast, and wait for the chain to have executed it.
    pub async fn execute(&self, data: &[u8], signature: &[u8]) -> Result<String, SuiError> {
        let v = self
            .call(
                "sui_executeTransactionBlock",
                json!([
                    b64(data),
                    [b64(signature)],
                    Value::Null,
                    "WaitForLocalExecution"
                ]),
            )
            .await?;
        v.get("digest")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| SuiError::BadReply(format!("no digest in {v}")))
    }
}

/// What a dry run said a transaction would cost, in MIST.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DryRun {
    pub computation: u64,
    pub storage: u64,
    pub rebate: u64,
    /// Computation plus storage, less the rebate. What it actually costs.
    pub net: u64,
}

fn parse_coin(v: &Value) -> Result<Coin, SuiError> {
    let id = v
        .get("coinObjectId")
        .and_then(Value::as_str)
        .ok_or_else(|| SuiError::BadReply("a coin with no id".into()))?;
    let version: u64 = v
        .get("version")
        .and_then(|x| match x {
            Value::String(s) => s.parse().ok(),
            Value::Number(n) => n.as_u64(),
            _ => None,
        })
        .ok_or_else(|| SuiError::BadReply("a coin with no version".into()))?;
    let digest = v
        .get("digest")
        .and_then(Value::as_str)
        .ok_or_else(|| SuiError::BadReply("a coin with no digest".into()))?;
    let balance: u128 = v
        .get("balance")
        .and_then(|x| match x {
            Value::String(s) => s.parse().ok(),
            Value::Number(n) => n.as_u64().map(u128::from),
            _ => None,
        })
        .ok_or_else(|| SuiError::BadReply("a coin with no balance".into()))?;
    Ok(Coin {
        object: ObjectRef {
            id: parse_id(id)?,
            version,
            digest: bs58_decode(digest)?,
        },
        balance,
    })
}

fn parse_id(s: &str) -> Result<[u8; 32], SuiError> {
    let h = s.strip_prefix("0x").unwrap_or(s);
    let raw = hex::decode(h).map_err(|_| SuiError::BadObjectId)?;
    if raw.len() != 32 {
        return Err(SuiError::BadObjectId);
    }
    let mut o = [0u8; 32];
    o.copy_from_slice(&raw);
    Ok(o)
}

/// Sui writes object digests in base58 and addresses in hex, in the same
/// reply. Decoding one as the other yields 32 plausible bytes that name
/// nothing.
pub fn bs58_decode(s: &str) -> Result<[u8; 32], SuiError> {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut out: Vec<u8> = vec![0];
    for c in s.bytes() {
        let val = ALPHABET
            .iter()
            .position(|&a| a == c)
            .ok_or(SuiError::BadObjectId)? as u32;
        let mut carry = val;
        for b in out.iter_mut() {
            carry += (*b as u32) * 58;
            *b = (carry & 0xff) as u8;
            carry >>= 8;
        }
        while carry > 0 {
            out.push((carry & 0xff) as u8);
            carry >>= 8;
        }
    }
    // Leading '1's are leading zero bytes, which the loop above cannot
    // produce because zero contributes nothing to the accumulator.
    let leading = s.bytes().take_while(|&c| c == b'1').count();
    out.extend(std::iter::repeat_n(0u8, leading));
    out.reverse();
    if out.len() != 32 {
        return Err(SuiError::BadObjectId);
    }
    let mut o = [0u8; 32];
    o.copy_from_slice(&out);
    Ok(o)
}

/// Base64, which is how Sui carries transaction bytes over JSON-RPC.
pub fn b64(b: &[u8]) -> String {
    const T: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(b.len().div_ceil(3) * 4);
    for c in b.chunks(3) {
        let n = ((c[0] as u32) << 16)
            | ((*c.get(1).unwrap_or(&0) as u32) << 8)
            | (*c.get(2).unwrap_or(&0) as u32);
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        s.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    s
}

fn cut(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= 300 {
        return t.to_string();
    }
    t.chars().take(300).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Base64, against the RFC's own vectors, including both padding cases.
    #[test]
    fn base64_matches_the_reference() {
        for (input, want) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(b64(input), want, "base64 of {input:?}");
        }
    }

    /// A digest round-trips through base58.
    ///
    /// Sui uses base58 for digests and hex for addresses in the same reply,
    /// and reading one as the other gives 32 bytes that name nothing.
    #[test]
    fn digests_decode_from_base58() {
        // 32 bytes of 0xff, and its base58 form.
        let raw = [0xffu8; 32];
        let enc = crate::tx::transaction_digest_of_bytes(&raw);
        assert_eq!(bs58_decode(&enc).unwrap(), raw);
        assert!(bs58_decode("0OIl").is_err(), "not in the alphabet");
    }
}
