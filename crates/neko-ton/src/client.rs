//! toncenter's HTTP API.
//!
//! Same contract as every other chain here: the server supplies facts - a
//! balance, a sequence number, the address of a jetton wallet - and never
//! assembles a message. The bytes are built and signed locally, so a server
//! that has been replaced can make this wallet fail but not make it sign a
//! transfer to somebody else.
//!
//! TON needs one thing from a server that the account chains do not: a wallet's
//! `seqno`, which lives inside the contract and can only be read by *running*
//! one of its methods. Sending with a stale one is not rejected as a double
//! spend - it is silently ignored, which looks exactly like a transfer that
//! disappeared.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::address::TonAddress;
use crate::cell::{Cell, CellBuilder};
use crate::chain_consts;
use crate::error::TonError;

pub struct Toncenter {
    base: String,
    api_key: Option<String>,
    http: reqwest::Client,
}

/// What a wallet needs to know before it can build a message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalletState {
    /// Nanotons held.
    pub balance: u128,
    /// Messages this wallet has sent. Part of what gets signed.
    pub seqno: u32,
    /// Whether the contract exists yet. If it does not, the first transfer has
    /// to carry its code.
    pub deployed: bool,
}

impl Toncenter {
    pub fn new(base: Option<&str>, api_key: Option<String>) -> Self {
        Toncenter {
            base: base
                .filter(|u| !u.is_empty())
                .unwrap_or(chain_consts::DEFAULT_API)
                .trim_end_matches('/')
                .to_string(),
            api_key: api_key.filter(|k| !k.is_empty()),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.base
    }

    async fn call(&self, path: &str, body: Option<Value>) -> Result<Value, TonError> {
        let url = format!("{}{path}", self.base);
        let mut last = None;
        for attempt in 0..3u32 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(1 << (attempt - 1))).await;
            }
            let mut req = match &body {
                Some(b) => self.http.post(&url).json(b),
                None => self.http.get(&url),
            };
            if let Some(k) = &self.api_key {
                req = req.header("X-API-Key", k);
            }
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    last = Some(TonError::Network(e.to_string()));
                    continue;
                }
            };
            let status = resp.status();
            let v: Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    last = Some(TonError::BadReply(e.to_string()));
                    continue;
                }
            };
            // The public endpoint answers "Ratelimit exceed" constantly, and
            // that is worth another attempt; anything else it says is a
            // decision.
            if v.get("code").and_then(Value::as_i64) == Some(429) || status.as_u16() == 429 {
                last = Some(TonError::Network("rate limited".into()));
                continue;
            }
            if v.get("ok").and_then(Value::as_bool) != Some(true) {
                let msg = v
                    .get("error")
                    .and_then(Value::as_str)
                    .or_else(|| v.get("result").and_then(Value::as_str))
                    .unwrap_or("unknown error");
                return Err(TonError::Rpc(msg.to_string()));
            }
            return v
                .get("result")
                .cloned()
                .ok_or_else(|| TonError::BadReply("reply has no result".into()));
        }
        Err(last.unwrap_or_else(|| TonError::Network("no attempt succeeded".into())))
    }

    /// Balance, sequence number and whether the contract exists, in one place.
    pub async fn wallet_state(&self, addr: &TonAddress) -> Result<WalletState, TonError> {
        let info = self
            .call(
                &format!("/getAddressInformation?address={}", addr.to_raw_string()),
                None,
            )
            .await?;
        let balance = info
            .get("balance")
            .and_then(|b| {
                b.as_str()
                    .and_then(|s| s.parse::<i128>().ok())
                    .or_else(|| b.as_i64().map(i128::from))
            })
            .unwrap_or(0)
            .max(0) as u128;
        // "uninitialized" and "nonexist" both mean no contract. A wallet in
        // either state can hold coins; it just cannot spend them yet.
        let deployed = info.get("state").and_then(Value::as_str) == Some("active");

        // A wallet that does not exist has sent nothing, and asking its
        // contract for a sequence number would fail rather than say zero.
        let seqno = if deployed { self.seqno(addr).await? } else { 0 };
        Ok(WalletState {
            balance,
            seqno,
            deployed,
        })
    }

    /// Run the wallet's own `seqno` method.
    pub async fn seqno(&self, addr: &TonAddress) -> Result<u32, TonError> {
        let v = self
            .call(
                "/runGetMethod",
                Some(json!({"address": addr.to_raw_string(), "method": "seqno", "stack": []})),
            )
            .await?;
        let s = v
            .get("stack")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_array)
            .and_then(|e| e.get(1))
            .and_then(Value::as_str)
            .ok_or_else(|| TonError::BadReply("seqno is not on the stack".into()))?;
        let hex = s.trim_start_matches("0x");
        u32::from_str_radix(hex, 16).map_err(|_| TonError::BadReply(format!("seqno {s:?}")))
    }

    /// Where this owner's balance of a jetton lives.
    ///
    /// Asked of the jetton master rather than derived here. It *is* derivable -
    /// it is a StateInit hash like any other address - but the code that goes
    /// into it belongs to the token, and a token that changed it would leave
    /// this wallet computing an address nobody uses.
    pub async fn jetton_wallet(
        &self,
        owner: &TonAddress,
        master: &TonAddress,
    ) -> Result<TonAddress, TonError> {
        let arg = address_slice(owner)?;
        let v = self
            .call(
                "/runGetMethod",
                Some(json!({
                    "address": master.to_raw_string(),
                    "method": "get_wallet_address",
                    "stack": [["tvm.Slice", base64(&crate::boc::serialize(&arg)?)]],
                })),
            )
            .await?;
        let b64 = v
            .get("stack")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_array)
            .and_then(|e| e.get(1))
            .and_then(|o| o.get("bytes").or(Some(o)))
            .and_then(Value::as_str)
            .ok_or_else(|| TonError::BadReply("no address on the stack".into()))?;
        let raw = decode_base64(b64)
            .ok_or_else(|| TonError::BadReply("jetton wallet is not base64".into()))?;
        parse_address_cell(&crate::boc::parse(&raw)?)
    }

    /// What a jetton wallet holds. A wallet that has never held the token does
    /// not exist, and that is a balance of zero rather than a failure.
    pub async fn jetton_balance(&self, wallet: &TonAddress) -> Result<u128, TonError> {
        let v = self
            .call(
                "/runGetMethod",
                Some(
                    json!({"address": wallet.to_raw_string(), "method": "get_wallet_data", "stack": []}),
                ),
            )
            .await;
        let v = match v {
            Ok(v) => v,
            // The contract is not there, which means nothing was ever sent to
            // it.
            Err(TonError::Rpc(_)) => return Ok(0),
            Err(e) => return Err(e),
        };
        if v.get("exit_code").and_then(Value::as_i64).unwrap_or(0) != 0 {
            return Ok(0);
        }
        let s = v
            .get("stack")
            .and_then(Value::as_array)
            .and_then(|a| a.first())
            .and_then(Value::as_array)
            .and_then(|e| e.get(1))
            .and_then(Value::as_str)
            .ok_or_else(|| TonError::BadReply("no balance on the stack".into()))?;
        u128::from_str_radix(s.trim_start_matches("0x"), 16)
            .map_err(|_| TonError::BadReply(format!("jetton balance {s:?}")))
    }

    /// Ask the node what a message will cost, which also makes it parse the
    /// message - so a malformed one fails here rather than after broadcast.
    pub async fn estimate_fee(
        &self,
        addr: &TonAddress,
        body: &Arc<Cell>,
        init: Option<&Arc<Cell>>,
    ) -> Result<u128, TonError> {
        let mut params = json!({
            "address": addr.to_raw_string(),
            "body": base64(&crate::boc::serialize(body)?),
            "ignore_chksig": true,
        });
        if let Some(i) = init {
            // The node wants the two halves separately rather than the
            // StateInit cell.
            let (code, data) = (
                i.refs()
                    .first()
                    .ok_or_else(|| TonError::BadBoc("no code".into()))?,
                i.refs()
                    .get(1)
                    .ok_or_else(|| TonError::BadBoc("no data".into()))?,
            );
            params["init_code"] = json!(base64(&crate::boc::serialize(code)?));
            params["init_data"] = json!(base64(&crate::boc::serialize(data)?));
        }
        let v = self.call("/estimateFee", Some(params)).await?;
        let f = v
            .get("source_fees")
            .ok_or_else(|| TonError::BadReply("no fee breakdown".into()))?;
        let sum: i64 = ["in_fwd_fee", "storage_fee", "gas_fee", "fwd_fee"]
            .iter()
            .filter_map(|k| f.get(*k).and_then(Value::as_i64))
            .sum();
        Ok(sum.max(0) as u128)
    }

    pub async fn send(&self, raw: &[u8]) -> Result<String, TonError> {
        let v = self
            .call("/sendBoc", Some(json!({ "boc": base64(raw) })))
            .await?;
        // The node returns an acknowledgement rather than a hash. The hash is
        // the message's own, and is known before it is sent.
        Ok(v.get("@extra")
            .and_then(Value::as_str)
            .unwrap_or("accepted")
            .to_string())
    }

    pub async fn transactions(&self, addr: &TonAddress, limit: u32) -> Result<Value, TonError> {
        self.call(
            &format!(
                "/getTransactions?address={}&limit={limit}&archival=false",
                addr.to_raw_string()
            ),
            None,
        )
        .await
    }
}

/// An address on its own, as a cell - which is how a get-method takes one.
fn address_slice(a: &TonAddress) -> Result<Arc<Cell>, TonError> {
    let mut b = CellBuilder::new();
    b.store_uint(0b10, 2)?;
    b.store_bit(false)?;
    b.store_uint(a.workchain as u8 as u64, 8)?;
    b.store_bytes(&a.hash)?;
    b.build_arc()
}

/// Read an address back out of a cell a get-method returned.
fn parse_address_cell(c: &Arc<Cell>) -> Result<TonAddress, TonError> {
    let bad = || TonError::BadReply("not an address cell".into());
    if c.bits() < 267 {
        return Err(bad());
    }
    let bit = |i: usize| (c.data()[i / 8] >> (7 - (i % 8))) & 1;
    if bit(0) != 1 || bit(1) != 0 || bit(2) != 0 {
        return Err(bad());
    }
    // Workchain and hash begin at bit 3, which is not a byte boundary - so
    // they are read out bit by bit rather than sliced.
    let mut wc = 0u8;
    for i in 0..8 {
        wc = (wc << 1) | bit(3 + i);
    }
    let mut hash = [0u8; 32];
    for i in 0..256 {
        hash[i / 8] = (hash[i / 8] << 1) | bit(11 + i);
    }
    Ok(TonAddress::new(wc as i8, hash))
}

fn base64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for c in input.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn decode_base64(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for ch in s.bytes() {
        if ch == b'=' {
            break;
        }
        let v = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An address written into a cell and read back has to be the same
    /// address. The fields do not sit on byte boundaries, so this is the one
    /// place bit-level reading is easy to get subtly wrong.
    #[test]
    fn an_address_survives_a_cell_round_trip() {
        for s in [
            "EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs",
            "0:5661bcb42ba847235760ce9aaa2dfff103eb7365db06e5df053120bacb77ddfd",
            "0:0000000000000000000000000000000000000000000000000000000000000000",
            "0:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        ] {
            let a = TonAddress::parse(s).unwrap();
            let cell = address_slice(&a).unwrap();
            assert_eq!(cell.bits(), 267);
            let back = parse_address_cell(&cell).unwrap();
            assert_eq!(back.hash, a.hash);
            assert_eq!(back.workchain, a.workchain);
        }
    }

    #[test]
    fn base64_matches_the_rfc() {
        for (input, want) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"foo", "Zm9v"),
            (b"foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input), want);
            assert_eq!(decode_base64(want).as_deref(), Some(input));
        }
    }
}
