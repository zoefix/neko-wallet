//! TronGrid HTTP client.
//!
//! Uses the full-node JSON API for everything that touches money, and the v1
//! indexer only for reading history.
//!
//! **We never call `/wallet/createtransaction`.** The node supplies a block
//! reference and nothing else; the wallet builds and signs the bytes itself.

use std::time::Duration;

use neko_hd::Address;
use serde_json::{json, Value};

use crate::tx::TxParams;

/// TronGrid returns HTTP 200 with an `Error` field for deterministic failures.
/// Retrying those only burns quota, so the distinction is load-bearing.
#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("network request failed: {0}")]
    Transport(String),
    #[error("rate limited by the node")]
    RateLimited,
    #[error("node returned HTTP {status}")]
    Http { status: u16, body: String },
    /// A deterministic rejection. Never retried.
    #[error("{0}")]
    Business(String),
    #[error("unexpected response shape: {0}")]
    Malformed(String),
    #[error("broadcast rejected: {0}")]
    Broadcast(String),
    #[error("this node does not provide transaction history (needs the TronGrid v1 API)")]
    NoHistoryApi,
}

impl ChainError {
    fn retryable(&self) -> bool {
        match self {
            // A deterministic failure will fail again identically.
            ChainError::Business(_) | ChainError::Broadcast(_) | ChainError::NoHistoryApi => false,
            ChainError::Malformed(_) => false,
            ChainError::RateLimited => true,
            ChainError::Http { status, .. } => *status >= 500,
            ChainError::Transport(_) => true,
        }
    }
}

/// What an account can spend before TRX is burned.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Resources {
    pub energy_available: i64,
    pub energy_limit: i64,
    pub bandwidth_available: i64,
    pub bandwidth_limit: i64,
}

/// An energy estimate, split so the total can be explained.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EnergyEstimate {
    /// What the contract call itself costs.
    pub base: i64,
    /// The dynamic-energy surcharge applied to popular contracts.
    pub penalty: i64,
}

impl EnergyEstimate {
    pub fn total(&self) -> i64 {
        self.base + self.penalty
    }
}

/// Chain governance prices, in sun.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prices {
    pub sun_per_energy: i64,
    pub sun_per_bandwidth: i64,
}

impl Default for Prices {
    fn default() -> Self {
        // Only used if the chain query fails; the UI labels the result as an
        // estimate either way.
        Self {
            sun_per_energy: 210,
            sun_per_bandwidth: 1000,
        }
    }
}

pub struct TronGrid {
    http: reqwest::Client,
    base: String,
    api_key: Option<String>,
}

const MAX_RETRIES: u32 = 3;
const TIMEOUT: Duration = Duration::from_secs(30);

impl TronGrid {
    /// `base_url` chooses which server speaks for mainnet; it does not choose a
    /// chain. Mainnet is the only chain this wallet knows.
    pub fn new(base_url: Option<&str>, api_key: Option<String>) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(TIMEOUT)
                .build()
                .expect("http client must build"),
            base: base_url
                .filter(|u| !u.is_empty())
                .unwrap_or(crate::chain_consts::DEFAULT_URL)
                .trim_end_matches('/')
                .to_string(),
            api_key: api_key.filter(|k| !k.is_empty()),
        }
    }

    async fn post_once(&self, path: &str, body: &Value) -> Result<Value, ChainError> {
        let mut req = self.http.post(format!("{}{path}", self.base)).json(body);
        if let Some(k) = &self.api_key {
            req = req.header("TRON-PRO-API-KEY", k);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ChainError::Transport(e.to_string()))?;
        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| ChainError::Transport(e.to_string()))?;

        if status.as_u16() == 429 {
            return Err(ChainError::RateLimited);
        }
        if !status.is_success() {
            return Err(ChainError::Http {
                status: status.as_u16(),
                body: text,
            });
        }
        let v: Value = serde_json::from_str(&text)
            .map_err(|_| ChainError::Malformed(text.chars().take(200).collect()))?;

        // HTTP 200 with an Error field: deterministic, do not retry.
        if let Some(e) = v.get("Error").and_then(Value::as_str) {
            return Err(ChainError::Business(e.to_string()));
        }
        Ok(v)
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value, ChainError> {
        let mut last = None;
        for attempt in 0..MAX_RETRIES {
            match self.post_once(path, &body).await {
                Ok(v) => return Ok(v),
                Err(e) if e.retryable() => {
                    // 1s, 2s, 4s.
                    tokio_sleep(Duration::from_secs(1 << attempt)).await;
                    last = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last.unwrap_or_else(|| ChainError::Transport("exhausted retries".into())))
    }

    async fn get(&self, path: &str) -> Result<Value, ChainError> {
        let mut req = self.http.get(format!("{}{path}", self.base));
        if let Some(k) = &self.api_key {
            req = req.header("TRON-PRO-API-KEY", k);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| ChainError::Transport(e.to_string()))?;
        let status = resp.status();
        if status.as_u16() == 404 {
            return Err(ChainError::NoHistoryApi);
        }
        let text = resp
            .text()
            .await
            .map_err(|e| ChainError::Transport(e.to_string()))?;
        if !status.is_success() {
            return Err(ChainError::Http {
                status: status.as_u16(),
                body: text,
            });
        }
        serde_json::from_str(&text)
            .map_err(|_| ChainError::Malformed(text.chars().take(200).collect()))
    }

    /// The only thing we take from the node when building a transaction.
    pub async fn tx_params(&self, fee_limit: i64) -> Result<TxParams, ChainError> {
        let v = self.post("/wallet/getnowblock", json!({})).await?;
        let id = v
            .pointer("/blockID")
            .and_then(Value::as_str)
            .ok_or_else(|| ChainError::Malformed("no blockID".into()))?;
        let num = v
            .pointer("/block_header/raw_data/number")
            .and_then(Value::as_i64)
            .ok_or_else(|| ChainError::Malformed("no block number".into()))?;

        let bytes = hex_decode(id)?;
        if bytes.len() != 32 {
            return Err(ChainError::Malformed("blockID is not 32 bytes".into()));
        }
        let mut ref_block_hash = [0u8; 32];
        ref_block_hash.copy_from_slice(&bytes);

        let now = now_ms();
        Ok(TxParams {
            ref_block_num: num as u64,
            ref_block_hash,
            timestamp: now,
            // A 60-second window: long enough to sign, short enough that a
            // stalled transaction expires instead of landing unexpectedly later.
            expiration: now + 60_000,
            fee_limit,
        })
    }

    /// TRX balance in sun. An un-activated address returns `{}`, which is zero,
    /// not an error.
    pub async fn trx_balance(&self, addr: Address) -> Result<i64, ChainError> {
        let v = self
            .post("/wallet/getaccount", json!({ "address": addr.to_hex() }))
            .await?;
        Ok(v.get("balance").and_then(Value::as_i64).unwrap_or(0))
    }

    /// TRC20 balance via a constant call to `balanceOf(address)`.
    pub async fn trc20_balance(
        &self,
        contract: Address,
        addr: Address,
    ) -> Result<u128, ChainError> {
        let mut param = vec![0u8; 32];
        param[12..].copy_from_slice(&addr.to_evm_bytes());
        let v = self
            .post(
                "/wallet/triggerconstantcontract",
                json!({
                    "owner_address": addr.to_hex(),
                    "contract_address": contract.to_hex(),
                    "function_selector": "balanceOf(address)",
                    "parameter": hex_encode(&param),
                }),
            )
            .await?;
        let raw = v
            .pointer("/constant_result/0")
            .and_then(Value::as_str)
            .ok_or_else(|| ChainError::Malformed("no constant_result".into()))?;
        Ok(be_u128(&hex_decode(raw)?))
    }

    /// Ask the chain how much energy this exact transfer needs.
    ///
    /// Hardcoding is measurably wrong: a transfer to an address that already
    /// holds the token costs ~1,984 energy, while one to an address with a zero
    /// balance costs ~29,650 because a storage slot must be created. Estimating
    /// low means the transaction fails *and still charges a fee*.
    pub async fn estimate_trc20_energy(
        &self,
        contract: Address,
        from: Address,
        calldata: &[u8],
    ) -> Result<EnergyEstimate, ChainError> {
        let v = self
            .post(
                "/wallet/triggerconstantcontract",
                json!({
                    "owner_address": from.to_hex(),
                    "contract_address": contract.to_hex(),
                    "function_selector": "transfer(address,uint256)",
                    // The selector is stripped: the node re-adds it.
                    "parameter": hex_encode(&calldata[4..]),
                }),
            )
            .await?;
        // `energy_penalty` is TRON's dynamic energy model: heavily used
        // contracts are surcharged on top of their base cost. It is charged in
        // addition to `energy_used`, and for USDT it is a large fraction of the
        // total, so reporting only the base would understate the fee badly.
        Ok(EnergyEstimate {
            base: v.get("energy_used").and_then(Value::as_i64).unwrap_or(0),
            penalty: v.get("energy_penalty").and_then(Value::as_i64).unwrap_or(0),
        })
    }

    /// What this account can spend before it has to burn TRX.
    ///
    /// TRON does not charge a flat fee. A transaction consumes bandwidth, and a
    /// contract call consumes energy; whatever the account cannot cover from
    /// its free allowance or its stake is paid for by burning TRX at the
    /// current price. So "the fee" is meaningless without knowing what the
    /// account already has.
    pub async fn account_resources(&self, addr: Address) -> Result<Resources, ChainError> {
        let v = self
            .post(
                "/wallet/getaccountresource",
                json!({ "address": addr.to_hex() }),
            )
            .await?;
        let g = |k: &str| v.get(k).and_then(Value::as_i64).unwrap_or(0);

        // Free bandwidth resets daily; staked bandwidth and energy come from
        // frozen TRX. Both are spendable before anything is burned.
        let free_net = (g("freeNetLimit") - g("freeNetUsed")).max(0);
        let staked_net = (g("NetLimit") - g("NetUsed")).max(0);
        Ok(Resources {
            energy_available: (g("EnergyLimit") - g("EnergyUsed")).max(0),
            energy_limit: g("EnergyLimit"),
            bandwidth_available: free_net + staked_net,
            bandwidth_limit: g("freeNetLimit") + g("NetLimit"),
        })
    }

    /// Current burn prices, straight from the chain.
    ///
    /// These are governance parameters and do change; hardcoding them makes
    /// every fee estimate quietly wrong after the next vote.
    pub async fn prices(&self) -> Result<Prices, ChainError> {
        let v = self.post("/wallet/getchainparameters", json!({})).await?;
        let params = v
            .get("chainParameter")
            .and_then(Value::as_array)
            .ok_or_else(|| ChainError::Malformed("no chainParameter".into()))?;
        let find = |key: &str| {
            params
                .iter()
                .find(|p| p.get("key").and_then(Value::as_str) == Some(key))
                .and_then(|p| p.get("value"))
                .and_then(Value::as_i64)
        };
        Ok(Prices {
            // Sun per unit of energy.
            sun_per_energy: find("getEnergyFee").unwrap_or(210),
            // Sun per byte of bandwidth.
            sun_per_bandwidth: find("getTransactionFee").unwrap_or(1000),
        })
    }

    /// Verify the configured USDT contract really is USDT on this network.
    pub async fn verify_usdt(&self, contract: Address) -> Result<(String, u8), ChainError> {
        let symbol = self.call_string(contract, "symbol()").await?;
        let decimals = self.call_u64(contract, "decimals()").await? as u8;
        Ok((symbol, decimals))
    }

    async fn call_raw(&self, contract: Address, selector: &str) -> Result<Vec<u8>, ChainError> {
        let v = self
            .post(
                "/wallet/triggerconstantcontract",
                json!({
                    "owner_address": contract.to_hex(),
                    "contract_address": contract.to_hex(),
                    "function_selector": selector,
                    "parameter": "",
                }),
            )
            .await?;
        let raw = v
            .pointer("/constant_result/0")
            .and_then(Value::as_str)
            .ok_or_else(|| ChainError::Malformed(format!("no result for {selector}")))?;
        hex_decode(raw)
    }

    async fn call_u64(&self, contract: Address, selector: &str) -> Result<u64, ChainError> {
        Ok(be_u128(&self.call_raw(contract, selector).await?) as u64)
    }

    /// ABI string return: offset(32) || length(32) || data.
    async fn call_string(&self, contract: Address, selector: &str) -> Result<String, ChainError> {
        let b = self.call_raw(contract, selector).await?;
        if b.len() < 64 {
            return Err(ChainError::Malformed("string return too short".into()));
        }
        let len = be_u128(&b[32..64]) as usize;
        let start: usize = 64;
        let end = start
            .checked_add(len)
            .filter(|e| *e <= b.len())
            .ok_or_else(|| ChainError::Malformed("string length exceeds the payload".into()))?;
        Ok(String::from_utf8_lossy(&b[start..end]).into_owned())
    }

    /// Broadcast a fully signed transaction.
    pub async fn broadcast(&self, raw_tx: &[u8]) -> Result<String, ChainError> {
        let v = self
            .post(
                "/wallet/broadcasthex",
                json!({ "transaction": hex_encode(raw_tx) }),
            )
            .await?;
        if v.get("result").and_then(Value::as_bool) == Some(true) {
            return Ok(v
                .get("txid")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string());
        }
        // Failure messages come back hex-encoded.
        let msg = v
            .get("message")
            .and_then(Value::as_str)
            .map(|m| {
                hex_decode(m)
                    .ok()
                    .map(|b| String::from_utf8_lossy(&b).into_owned())
                    .unwrap_or_else(|| m.to_string())
            })
            .unwrap_or_else(|| v.to_string());
        Err(ChainError::Broadcast(msg))
    }

    /// Native TRX transfers, newest first. Uses the v1 indexer, which a
    /// self-hosted full node will not have.
    pub async fn history_trx(&self, addr: Address, limit: u32) -> Result<Value, ChainError> {
        self.get(&format!(
            "/v1/accounts/{addr}/transactions?limit={limit}&only_confirmed=true&order_by=block_timestamp,desc"
        ))
        .await
    }

    pub async fn history_trc20(
        &self,
        addr: Address,
        contract: Address,
        limit: u32,
    ) -> Result<Value, ChainError> {
        self.get(&format!(
            "/v1/accounts/{addr}/transactions/trc20?limit={limit}&contract_address={contract}&only_confirmed=true"
        ))
        .await
    }
}

async fn tokio_sleep(d: Duration) {
    tokio::time::sleep(d).await;
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn hex_decode(s: &str) -> Result<Vec<u8>, ChainError> {
    let s = s.trim_start_matches("0x");
    if s.len() % 2 != 0 {
        return Err(ChainError::Malformed("odd-length hex".into()));
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|_| ChainError::Malformed("bad hex".into()))
        })
        .collect()
}

/// Big-endian bytes to u128, taking the low 16 bytes of a 32-byte ABI word.
/// Token balances never approach 2^128, so this cannot silently truncate in
/// practice — and it never goes through f64, which would lose precision above
/// ~9e15.
fn be_u128(b: &[u8]) -> u128 {
    let start = b.len().saturating_sub(16);
    let mut out = [0u8; 16];
    let slice = &b[start..];
    out[16 - slice.len()..].copy_from_slice(slice);
    u128::from_be_bytes(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn business_errors_are_never_retried() {
        // Retrying a deterministic failure only burns API quota.
        assert!(!ChainError::Business("bad".into()).retryable());
        assert!(!ChainError::Broadcast("SIGERROR".into()).retryable());
        assert!(!ChainError::Malformed("x".into()).retryable());
        assert!(!ChainError::NoHistoryApi.retryable());

        assert!(ChainError::RateLimited.retryable());
        assert!(ChainError::Transport("reset".into()).retryable());
        assert!(ChainError::Http {
            status: 503,
            body: String::new()
        }
        .retryable());
        assert!(!ChainError::Http {
            status: 400,
            body: String::new()
        }
        .retryable());
    }

    #[test]
    fn be_u128_decodes_abi_words() {
        let mut w = [0u8; 32];
        w[31] = 1;
        assert_eq!(be_u128(&w), 1);
        // 2,500,000 = 0x2625a0, the TRC20 amount from the vectors.
        let bytes =
            hex_decode("00000000000000000000000000000000000000000000000000000000002625a0").unwrap();
        assert_eq!(be_u128(&bytes), 2_500_000);
        assert_eq!(be_u128(&[]), 0);
    }

    #[test]
    fn hex_round_trips() {
        assert_eq!(hex_encode(&[0x41, 0xff, 0x00]), "41ff00");
        assert_eq!(hex_decode("41ff00").unwrap(), vec![0x41, 0xff, 0x00]);
        assert_eq!(hex_decode("0x41").unwrap(), vec![0x41]);
        assert!(hex_decode("abc").is_err());
        assert!(hex_decode("zz").is_err());
    }
}
