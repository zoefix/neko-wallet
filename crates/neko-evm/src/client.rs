//! JSON-RPC against a BNB Chain node.
//!
//! The node supplies facts - a nonce, a gas price, an estimate, a balance -
//! and nothing else. It never assembles a transaction: those bytes are built
//! and signed locally, so a node that has been compromised or replaced can
//! make the wallet fail, but not make it sign a transfer to somebody else.
//!
//! Error handling mirrors the TRON client's, for the same reason. A JSON-RPC
//! `error` object is a decision the node has made - a bad nonce, a reverted
//! call - and retrying it only burns quota. Transport failures and 5xx are
//! worth another attempt.

use neko_hd::EvmAddress;
use serde_json::{json, Value};

use crate::abi;
use crate::error::EvmError;
use crate::tx::TxParams;

pub struct Rpc {
    url: String,
    /// Which chain this talks to. Carried rather than assumed: the chain id
    /// goes into every signature, and the wrong one produces a transaction
    /// that is replayable where the same address holds different funds.
    chain: crate::EvmChain,
    http: reqwest::Client,
}

impl Rpc {
    pub fn new(chain: crate::EvmChain, url: Option<&str>) -> Self {
        Rpc {
            url: url
                .filter(|u| !u.is_empty())
                .unwrap_or(chain.default_rpc)
                .to_string(),
            chain,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, EvmError> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let mut last = None;
        // Three attempts, but only for failures that could plausibly differ
        // next time.
        for attempt in 0..3u32 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(1 << (attempt - 1))).await;
            }
            let resp = match self.http.post(&self.url).json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    last = Some(EvmError::Network(e.to_string()));
                    continue;
                }
            };
            if resp.status().is_server_error() {
                last = Some(EvmError::Network(format!("HTTP {}", resp.status())));
                continue;
            }
            if !resp.status().is_success() {
                return Err(EvmError::Network(format!("HTTP {}", resp.status())));
            }
            let v: Value = resp
                .json()
                .await
                .map_err(|e| EvmError::BadReply(e.to_string()))?;
            if let Some(err) = v.get("error") {
                // The node decided. Trying again changes nothing.
                let msg = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                return Err(EvmError::Rpc(clean_revert(msg)));
            }
            return v
                .get("result")
                .cloned()
                .ok_or_else(|| EvmError::BadReply("reply has no result".into()));
        }
        Err(last.unwrap_or_else(|| EvmError::Network("no attempt succeeded".into())))
    }

    async fn quantity(&self, method: &str, params: Value) -> Result<u128, EvmError> {
        let v = self.call(method, params).await?;
        let s = v
            .as_str()
            .ok_or_else(|| EvmError::BadReply(format!("{method} did not return a quantity")))?;
        parse_quantity(s)
    }

    pub async fn chain_id(&self) -> Result<u64, EvmError> {
        Ok(self.quantity("eth_chainId", json!([])).await? as u64)
    }

    /// Native BNB balance, in wei.
    pub async fn balance(&self, who: EvmAddress) -> Result<u128, EvmError> {
        self.quantity("eth_getBalance", json!([who.to_string(), "latest"]))
            .await
    }

    /// Transactions already sent from this address - the next nonce.
    ///
    /// `pending` rather than `latest`, so a second transfer sent before the
    /// first confirms does not reuse a nonce and replace it.
    pub async fn nonce(&self, who: EvmAddress) -> Result<u64, EvmError> {
        Ok(self
            .quantity(
                "eth_getTransactionCount",
                json!([who.to_string(), "pending"]),
            )
            .await? as u64)
    }

    pub async fn gas_price(&self) -> Result<u128, EvmError> {
        self.quantity("eth_gasPrice", json!([])).await
    }

    /// Simulate the call. Costs nothing and touches no state.
    pub async fn estimate_gas(
        &self,
        from: EvmAddress,
        to: EvmAddress,
        value: u128,
        data: &[u8],
    ) -> Result<u64, EvmError> {
        let mut call = json!({
            "from": from.to_string(),
            "to": to.to_string(),
            "value": to_quantity(value),
        });
        if !data.is_empty() {
            call["data"] = json!(format!("0x{}", hex::encode(data)));
        }
        Ok(self
            .quantity("eth_estimateGas", json!([call, "latest"]))
            .await? as u64)
    }

    /// A read-only contract call.
    /// What this rollup will charge for posting `unsigned` to Ethereum.
    ///
    /// Zero on a chain that is not a rollup, which is not an approximation: it
    /// is the whole of the L1 fee there.
    ///
    /// The oracle is asked with the transaction's own bytes rather than a
    /// length, because the cost depends on how well they compress and a
    /// synthetic payload of the same size answers differently. The signature is
    /// not in an unsigned encoding and is charged for, so a fixed allowance for
    /// it is added: 68 bytes at the worst-case per-byte rate is small next to
    /// being short and having the transaction refused.
    pub async fn l1_fee(&self, unsigned: &[u8]) -> Result<u128, EvmError> {
        let Some(oracle) = self.chain().l1_fee_oracle else {
            return Ok(0);
        };
        let oracle = EvmAddress::parse(oracle)
            .map_err(|_| EvmError::BadReply("the L1 oracle address is malformed".into()))?;

        let out = self.eth_call(oracle, &l1_fee_calldata(unsigned)).await?;
        let tail = out
            .get(out.len().saturating_sub(16)..)
            .ok_or_else(|| EvmError::BadReply("the L1 oracle returned nothing".into()))?;
        let mut buf = [0u8; 16];
        buf.copy_from_slice(tail);
        Ok(u128::from_be_bytes(buf))
    }

    pub async fn eth_call(&self, to: EvmAddress, data: &[u8]) -> Result<Vec<u8>, EvmError> {
        let v = self
            .call(
                "eth_call",
                json!([{"to": to.to_string(), "data": format!("0x{}", hex::encode(data))}, "latest"]),
            )
            .await?;
        let s = v
            .as_str()
            .ok_or_else(|| EvmError::BadReply("eth_call did not return data".into()))?;
        hex::decode(s.trim_start_matches("0x"))
            .map_err(|_| EvmError::BadReply("eth_call returned malformed hex".into()))
    }

    pub async fn token_balance(
        &self,
        token: EvmAddress,
        holder: EvmAddress,
    ) -> Result<u128, EvmError> {
        let out = self.eth_call(token, &abi::balance_of(holder)).await?;
        abi::read_u256(&out)
    }

    /// Ask the contract what it is, before trusting a built-in address.
    ///
    /// A custom RPC endpoint could otherwise point the wallet's USDT constant
    /// at something else, and funds sent to the wrong contract are gone.
    pub async fn verify_token(&self, token: EvmAddress) -> Result<(String, u8), EvmError> {
        let sym = abi::read_string(&self.eth_call(token, &abi::symbol()).await?)?;
        let dec = abi::read_u256(&self.eth_call(token, &abi::decimals()).await?)?;
        if dec > 36 {
            return Err(EvmError::BadReply(format!(
                "{dec} decimals is not plausible"
            )));
        }
        Ok((sym, dec as u8))
    }

    pub async fn send_raw(&self, raw: &[u8]) -> Result<String, EvmError> {
        let v = self
            .call(
                "eth_sendRawTransaction",
                json!([format!("0x{}", hex::encode(raw))]),
            )
            .await?;
        v.as_str()
            .map(str::to_string)
            .ok_or_else(|| EvmError::BadReply("broadcast returned no hash".into()))
    }

    /// `None` while the transaction is still pending.
    pub async fn receipt_status(&self, hash: &str) -> Result<Option<bool>, EvmError> {
        let v = self
            .call("eth_getTransactionReceipt", json!([hash]))
            .await?;
        if v.is_null() {
            return Ok(None);
        }
        let s = v
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| EvmError::BadReply("receipt has no status".into()))?;
        Ok(Some(parse_quantity(s)? == 1))
    }

    pub fn chain(&self) -> crate::EvmChain {
        self.chain
    }

    /// The current base fee, from the latest block header.
    ///
    /// EIP-1559 only. It is set by the protocol from how full recent blocks
    /// were, and it is what a type-2 transaction actually pays - the ceiling
    /// is only headroom.
    pub async fn base_fee(&self) -> Result<u128, EvmError> {
        let v = self
            .call("eth_getBlockByNumber", json!(["latest", false]))
            .await?;
        let s = v
            .get("baseFeePerGas")
            .and_then(Value::as_str)
            .ok_or_else(|| EvmError::BadReply("block header has no base fee".into()))?;
        parse_quantity(s)
    }

    /// What recent blocks have been accepting as a tip.
    ///
    /// Zero is a legitimate answer on a quiet chain, and is what Ethereum
    /// returns at the time of writing.
    pub async fn priority_fee(&self) -> Result<u128, EvmError> {
        self.quantity("eth_maxPriorityFeePerGas", json!([])).await
    }

    /// Everything needed to build a transaction, in one place.
    pub async fn tx_params(
        &self,
        from: EvmAddress,
        to: EvmAddress,
        value: u128,
        data: &[u8],
    ) -> Result<TxParams, EvmError> {
        let nonce = self.nonce(from).await?;
        let estimate = self.estimate_gas(from, to, value, data).await?;
        let fees = match self.chain.tx_type {
            crate::TxType::Legacy => crate::tx::Fees::Legacy {
                gas_price: self.gas_price().await?,
            },
            crate::TxType::Eip1559 => {
                let base = self.base_fee().await?;
                let tip = self.priority_fee().await.unwrap_or(0);
                crate::tx::Fees::Eip1559 {
                    // Double the base fee, plus the tip. The base fee can rise
                    // at most 12.5% per block, so this is headroom for six
                    // consecutive full blocks - and headroom costs nothing,
                    // because only the base fee and the tip are charged.
                    max_fee_per_gas: base.saturating_mul(2).saturating_add(tip),
                    max_priority_fee_per_gas: tip,
                    base_fee: base,
                }
            }
        };
        Ok(TxParams {
            nonce,
            // A fifth over the estimate. Estimation runs against the current
            // state, and a token transfer that has to create a storage slot
            // for a first-time holder costs more by the time it lands; running
            // out of gas still spends the fee.
            gas_limit: estimate + estimate / 5,
            chain_id: self.chain.chain_id,
            fees,
        })
    }
}

/// A JSON-RPC quantity: `0x`-prefixed, minimal, possibly `0x0`.
pub fn parse_quantity(s: &str) -> Result<u128, EvmError> {
    let body = s.trim_start_matches("0x").trim_start_matches("0X");
    if body.is_empty() {
        return Err(EvmError::BadReply(format!("{s:?} is not a quantity")));
    }
    if body.len() > 32 {
        // More than 128 bits. Refuse rather than wrap: a wrapped balance would
        // be displayed as though it were real.
        return Err(EvmError::AmountTooLarge);
    }
    u128::from_str_radix(body, 16).map_err(|_| EvmError::BadReply(format!("{s:?} is not hex")))
}

/// Trim a node's revert message down to the part a person can act on.
///
/// A revert carries an ABI-encoded `Error(string)`, and nodes tend to append
/// it to the message as raw hex. The sentence in front of it is the useful
/// part - "transfer amount exceeds balance" tells somebody exactly what went
/// wrong; two hundred hex characters after it tell them nothing and bury it.
pub fn clean_revert(msg: &str) -> String {
    let cut = match msg.find(": 0x") {
        Some(i)
            if msg[i + 2..]
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b'x') =>
        {
            i
        }
        _ => msg.len(),
    };
    msg[..cut].trim().trim_end_matches(':').trim().to_string()
}

pub fn to_quantity(v: u128) -> String {
    format!("0x{v:x}")
}

/// A 32-byte big-endian word, which is how the ABI writes every scalar.
fn u256(v: u128) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&v.to_be_bytes());
    w
}

/// What a signature adds to a transaction's encoded length. Charged for by the
/// rollup and absent from the unsigned bytes the oracle is shown.
const SIGNATURE_BYTES: usize = 68;

/// `getL1Fee(bytes)` against the transaction as it will actually be posted.
///
/// The stand-in bytes for the signature must not repeat, and that is not a
/// detail. Since Fjord the oracle prices a transaction by how well it
/// *compresses* rather than by its length: this padding was 68 copies of
/// `0xff`, which compresses to almost nothing, so the wallet reserved less
/// than the chain charges. A real signature is 64 bytes of `r` and `s` and
/// does not compress at all.
///
/// Measured against Optimism's own oracle: a counter costs exactly what a
/// random signature costs, while `0xff` padding came in 193 million wei short
/// on a plain transfer and 633 million short on a token one. Being short here
/// is what has "send everything" refused by the node, which is the failure
/// this whole query exists to prevent.
fn l1_fee_calldata(unsigned: &[u8]) -> Vec<u8> {
    let mut padded = unsigned.to_vec();
    padded.extend((0..SIGNATURE_BYTES).map(|i| i as u8));
    // getL1Fee(bytes) = 0x49948e0e, then offset, length, padded body.
    let mut data = Vec::with_capacity(4 + 64 + padded.len() + 32);
    data.extend_from_slice(&[0x49, 0x94, 0x8e, 0x0e]);
    data.extend_from_slice(&u256(32));
    data.extend_from_slice(&u256(padded.len() as u128));
    data.extend_from_slice(&padded);
    data.resize(data.len() + (32 - padded.len() % 32) % 32, 0);
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantities_round_trip() {
        assert_eq!(parse_quantity("0x0").unwrap(), 0);
        assert_eq!(parse_quantity("0x38").unwrap(), 56);
        assert_eq!(parse_quantity("0xde0b6b3a7640000").unwrap(), 10u128.pow(18));
        assert_eq!(to_quantity(0), "0x0");
        assert_eq!(to_quantity(56), "0x38");
        for v in [0u128, 1, 56, 10u128.pow(18), u128::MAX] {
            assert_eq!(parse_quantity(&to_quantity(v)).unwrap(), v);
        }
    }

    /// Replies arrive from a node we do not control, so every shape of
    /// nonsense must be an error rather than a panic or a wrong number.
    #[test]
    fn malformed_quantities_are_refused() {
        for s in ["", "0x", "zz", "0xzz", "0x1p"] {
            assert!(parse_quantity(s).is_err(), "{s:?} was accepted");
        }
        // 33 hex digits: over 128 bits.
        assert!(matches!(
            parse_quantity(&format!("0x1{}", "0".repeat(32))),
            Err(EvmError::AmountTooLarge)
        ));
    }

    /// The sentence survives; the hex dump does not.
    #[test]
    fn revert_messages_keep_the_part_that_helps() {
        let raw = "execution reverted: BEP20: transfer amount exceeds balance: 0x08c379a0000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000264245503230";
        assert_eq!(
            clean_revert(raw),
            "execution reverted: BEP20: transfer amount exceeds balance"
        );
        // Nothing to trim, nothing trimmed.
        assert_eq!(clean_revert("nonce too low"), "nonce too low");
        assert_eq!(clean_revert(""), "");
        // A colon that is not a hex dump must be left alone.
        assert_eq!(
            clean_revert("bad params: expected 2"),
            "bad params: expected 2"
        );
    }

    #[test]
    fn the_default_endpoint_is_used_when_none_is_configured() {
        assert_eq!(Rpc::new(crate::BSC, None).url, crate::BSC.default_rpc);
        assert_eq!(Rpc::new(crate::BSC, Some("")).url, crate::BSC.default_rpc);
        assert_eq!(
            Rpc::new(crate::ETHEREUM, None).url,
            crate::ETHEREUM.default_rpc
        );
        assert_eq!(
            Rpc::new(crate::BSC, Some("https://x.example")).url,
            "https://x.example"
        );
    }
}

#[cfg(test)]
mod l1_fee_query {
    use super::*;

    /// The selector, the ABI frame, and the length the oracle is told.
    #[test]
    fn the_call_is_shaped_the_way_the_oracle_expects() {
        let unsigned = [0xaa_u8; 48];
        let data = l1_fee_calldata(&unsigned);

        assert_eq!(&data[..4], &[0x49, 0x94, 0x8e, 0x0e], "getL1Fee(bytes)");
        // offset 32, then the length: the payload plus the signature allowance.
        assert_eq!(data[4..36], u256(32));
        let want = unsigned.len() + SIGNATURE_BYTES;
        assert_eq!(data[36..68], u256(want as u128));
        assert_eq!(&data[68..68 + unsigned.len()], &unsigned[..]);
        // Padded to a whole number of words, and no further.
        assert_eq!((data.len() - 68) % 32, 0);
        assert!(data.len() - 68 - want < 32);
    }

    /// The signature stand-in must not compress, because the chain prices
    /// compressed size rather than length.
    ///
    /// This is the regression. The padding was 68 copies of `0xff`; measured
    /// against Optimism's oracle that reserved 193 million wei too little on a
    /// plain transfer and 633 million too little on a token one, because
    /// FastLZ turns a run of identical bytes into almost nothing while a real
    /// signature is incompressible. Being short is what has "send everything"
    /// refused - the failure the L1 query exists to prevent - so the test is
    /// that the padding has no repeats at all.
    #[test]
    fn the_signature_allowance_does_not_compress_away() {
        let data = l1_fee_calldata(&[]);
        let pad = &data[68..68 + SIGNATURE_BYTES];
        assert_eq!(pad.len(), 68);

        // No byte repeats, so there is no run for a compressor to collapse.
        let mut seen = [false; 256];
        for b in pad {
            assert!(!seen[*b as usize], "byte {b:#04x} appears twice in the padding");
            seen[*b as usize] = true;
        }

        // And specifically not the constant it used to be.
        assert!(
            !pad.iter().all(|b| *b == pad[0]),
            "a constant fill compresses to nothing and under-reserves the fee"
        );
    }
}
