//! Esplora, over HTTPS.
//!
//! Bitcoin is the one chain here where the node cannot answer the question the
//! wallet asks. "What does this address hold" requires an index over the entire
//! chain that a node does not build unless configured to, so every wallet
//! either runs its own indexer or asks somebody else's. This asks somebody
//! else's, and the interface says whose and says it is configurable.
//!
//! What it does *not* do is let that server decide anything. It returns
//! outputs, a fee rate and a block height; which coins get spent and what they
//! are spent on is decided here, and the transaction is built and signed here.
//! A hostile server can make this wallet fail or quote a silly fee. It cannot
//! make it sign away coins.

use neko_hd::BtcAddress;
use serde_json::Value;

use crate::chain_consts;
use crate::error::BtcError;
use crate::tx::{OutPoint, Utxo};

pub struct Esplora {
    base: String,
    http: reqwest::Client,
}

impl Esplora {
    pub fn new(base: Option<&str>) -> Self {
        let base = base
            .filter(|u| !u.is_empty())
            .unwrap_or(chain_consts::DEFAULT_API)
            .trim_end_matches('/')
            .to_string();
        Esplora {
            base,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    pub fn endpoint(&self) -> &str {
        &self.base
    }

    async fn get(&self, path: &str) -> Result<String, BtcError> {
        let url = format!("{}{path}", self.base);
        let mut last = None;
        for attempt in 0..3u32 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(1 << (attempt - 1))).await;
            }
            let resp = match self.http.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    last = Some(BtcError::Network(e.to_string()));
                    continue;
                }
            };
            let status = resp.status();
            if status.is_server_error() || status.as_u16() == 429 {
                last = Some(BtcError::Network(format!("HTTP {status}")));
                continue;
            }
            let body = resp
                .text()
                .await
                .map_err(|e| BtcError::BadReply(e.to_string()))?;
            if !status.is_success() {
                // Esplora puts its reason in the body, and it is usually the
                // useful part - "sendrawtransaction RPC error: min relay fee
                // not met" says exactly what to change.
                return Err(BtcError::Rpc(body.trim().to_string()));
            }
            return Ok(body);
        }
        Err(last.unwrap_or_else(|| BtcError::Network("no attempt succeeded".into())))
    }

    async fn get_json(&self, path: &str) -> Result<Value, BtcError> {
        let body = self.get(path).await?;
        serde_json::from_str(&body).map_err(|e| BtcError::BadReply(e.to_string()))
    }

    /// Every coin this address can spend.
    ///
    /// The balance is the sum of these; there is no balance to ask for. An
    /// address that has never been paid returns an empty list, which is a
    /// balance of zero and not a failure.
    pub async fn utxos(&self, addr: BtcAddress) -> Result<Vec<Utxo>, BtcError> {
        let v = self.get_json(&format!("/address/{addr}/utxo")).await?;
        let script = addr.script_pubkey();
        let rows = v
            .as_array()
            .ok_or_else(|| BtcError::BadReply("utxo list is not an array".into()))?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let txid = r
                .get("txid")
                .and_then(Value::as_str)
                .ok_or_else(|| BtcError::BadReply("a utxo has no txid".into()))?;
            let vout = r
                .get("vout")
                .and_then(Value::as_u64)
                .ok_or_else(|| BtcError::BadReply("a utxo has no index".into()))?
                as u32;
            let value = r
                .get("value")
                .and_then(Value::as_u64)
                .ok_or_else(|| BtcError::BadReply("a utxo has no value".into()))?;
            let block_height = r
                .get("status")
                .filter(|s| s.get("confirmed").and_then(Value::as_bool) == Some(true))
                .and_then(|s| s.get("block_height"))
                .and_then(Value::as_u64);

            out.push(Utxo {
                outpoint: OutPoint::from_display_txid(txid, vout)?,
                value,
                // The script is ours by construction - we asked about our own
                // address - and is recorded here rather than taken from the
                // server, because the signature commits to it.
                script_pubkey: script.clone(),
                block_height,
            });
        }
        Ok(out)
    }

    /// The rate for confirmation within `blocks`.
    ///
    /// Kept fractional. Estimators quote figures like 1.12, and rounding that
    /// to a whole satoshi before multiplying makes a small transfer cost most
    /// of a percent again for nothing. Never below the relay floor either: a
    /// rate under it does not make a slow transaction, it makes one no node
    /// will forward, and Esplora does quote below it for distant targets.
    pub async fn fee_rate(&self, blocks: u32) -> Result<crate::coins::FeeRate, BtcError> {
        let v = self.get_json("/fee-estimates").await?;
        let obj = v
            .as_object()
            .ok_or_else(|| BtcError::BadReply("fee estimates are not an object".into()))?;
        // Esplora keys by target in blocks and does not always include the one
        // asked for, so take the nearest target at or before it - which errs
        // toward paying more and confirming sooner.
        let mut best: Option<(u32, f64)> = None;
        for (k, val) in obj {
            let Ok(target) = k.parse::<u32>() else {
                continue;
            };
            let Some(rate) = val.as_f64() else { continue };
            if target <= blocks && best.map(|(t, _)| target > t).unwrap_or(true) {
                best = Some((target, rate));
            }
        }
        let rate = best
            .map(|(_, r)| r)
            .or_else(|| obj.values().filter_map(Value::as_f64).next_back())
            .ok_or_else(|| BtcError::BadReply("no usable fee estimate".into()))?;
        Ok(crate::coins::FeeRate::from_sat_per_vb(rate))
    }

    pub async fn tip_height(&self) -> Result<u64, BtcError> {
        self.get("/blocks/tip/height")
            .await?
            .trim()
            .parse()
            .map_err(|_| BtcError::BadReply("tip height is not a number".into()))
    }

    /// Broadcast. Returns the txid, which we already knew - segwit fixed
    /// malleability, so the id does not depend on the signature.
    pub async fn broadcast(&self, raw: &[u8]) -> Result<String, BtcError> {
        let url = format!("{}/tx", self.base);
        let resp = self
            .http
            .post(&url)
            .body(hex::encode(raw))
            .send()
            .await
            .map_err(|e| BtcError::Network(e.to_string()))?;
        let status = resp.status();
        let body = resp
            .text()
            .await
            .map_err(|e| BtcError::BadReply(e.to_string()))?;
        if !status.is_success() {
            return Err(BtcError::Rpc(body.trim().to_string()));
        }
        Ok(body.trim().to_string())
    }

    /// Recent transactions touching this address, newest first.
    pub async fn address_txs(&self, addr: BtcAddress) -> Result<Value, BtcError> {
        self.get_json(&format!("/address/{addr}/txs")).await
    }
}
