//! The Aptos REST API.
//!
//! One host, asked for four things: an account's sequence number, its two
//! balances, what gas costs, and the submission itself. Balances come from
//! *view functions* rather than from reading storage, because APT's balance
//! now lives in two places at once - a legacy coin store and a fungible-asset
//! store - and `0x1::coin::balance` is the view that adds them up.

use serde_json::{json, Value};

use crate::address::AptosAddress;
use crate::error::AptosError;

pub struct Rest {
    base: String,
    http: reqwest::Client,
}

impl Rest {
    pub fn new(url: Option<&str>) -> Self {
        Rest {
            base: url
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
        &self.base
    }

    async fn get(&self, path: &str) -> Result<Value, AptosError> {
        let r = self
            .http
            .get(format!("{}/{path}", self.base))
            .send()
            .await
            .map_err(|e| AptosError::Rpc(e.to_string()))?;
        let status = r.status();
        let body = r.text().await.map_err(|e| AptosError::Rpc(e.to_string()))?;
        if !status.is_success() {
            return Err(AptosError::Rpc(short(&body)));
        }
        serde_json::from_str(&body).map_err(|_| AptosError::BadReply(short(&body)))
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value, AptosError> {
        let r = self
            .http
            .post(format!("{}/{path}", self.base))
            .json(&body)
            .send()
            .await
            .map_err(|e| AptosError::Rpc(e.to_string()))?;
        let status = r.status();
        let text = r.text().await.map_err(|e| AptosError::Rpc(e.to_string()))?;
        if !status.is_success() {
            return Err(AptosError::Rpc(short(&text)));
        }
        serde_json::from_str(&text).map_err(|_| AptosError::BadReply(short(&text)))
    }

    /// A view function's first return value, as a string.
    async fn view_one(&self, body: Value) -> Result<String, AptosError> {
        let v = self.post("view", body).await?;
        v.as_array()
            .and_then(|a| a.first())
            .map(|x| match x {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            })
            .ok_or_else(|| AptosError::BadReply(format!("view returned {v}")))
    }

    /// The next sequence number for this account, and zero for one that does
    /// not exist yet.
    ///
    /// A brand-new account has no resource on chain at all and the node
    /// answers 404. That is not an error here: an account is created by its
    /// first transaction, which is sent at sequence number zero.
    pub async fn sequence_number(&self, who: AptosAddress) -> Result<u64, AptosError> {
        match self.get(&format!("accounts/{who}")).await {
            Ok(v) => v
                .get("sequence_number")
                .and_then(Value::as_str)
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| AptosError::BadReply(format!("no sequence number in {v}"))),
            Err(AptosError::Rpc(e)) if e.contains("account_not_found") || e.contains("404") => {
                Ok(0)
            }
            Err(e) => Err(e),
        }
    }

    /// APT, in octas.
    ///
    /// Through `0x1::coin::balance`, which is the view that reports the total
    /// across both stores. Reading the fungible-asset store alone understates
    /// it for any account that still holds the legacy kind.
    pub async fn apt_balance(&self, who: AptosAddress) -> Result<u128, AptosError> {
        let s = self
            .view_one(json!({
                "function": "0x1::coin::balance",
                "type_arguments": ["0x1::aptos_coin::AptosCoin"],
                "arguments": [who.to_string()],
            }))
            .await?;
        s.parse()
            .map_err(|_| AptosError::BadReply(format!("balance {s:?} is not a number")))
    }

    /// A fungible asset's balance, in its own units.
    pub async fn fa_balance(
        &self,
        who: AptosAddress,
        metadata: AptosAddress,
    ) -> Result<u128, AptosError> {
        let s = self
            .view_one(json!({
                "function": "0x1::primary_fungible_store::balance",
                "type_arguments": ["0x1::fungible_asset::Metadata"],
                "arguments": [who.to_string(), metadata.to_string()],
            }))
            .await?;
        s.parse()
            .map_err(|_| AptosError::BadReply(format!("balance {s:?} is not a number")))
    }

    /// What a fungible asset calls itself, asked of the chain.
    ///
    /// Checked before a transfer is signed, for the same reason as everywhere
    /// else: the address is what decides which token moves, and the name is
    /// what proves the address is the one meant.
    pub async fn fa_symbol(&self, metadata: AptosAddress) -> Result<String, AptosError> {
        self.view_one(json!({
            "function": "0x1::fungible_asset::symbol",
            "type_arguments": ["0x1::fungible_asset::Metadata"],
            "arguments": [metadata.to_string()],
        }))
        .await
    }

    pub async fn fa_decimals(&self, metadata: AptosAddress) -> Result<u8, AptosError> {
        let s = self
            .view_one(json!({
                "function": "0x1::fungible_asset::decimals",
                "type_arguments": ["0x1::fungible_asset::Metadata"],
                "arguments": [metadata.to_string()],
            }))
            .await?;
        s.trim_matches('"')
            .parse()
            .map_err(|_| AptosError::BadReply(format!("decimals {s:?} is not a number")))
    }

    /// Octas per gas unit.
    pub async fn gas_unit_price(&self) -> Result<u64, AptosError> {
        let v = self.get("estimate_gas_price").await?;
        v.get("gas_estimate")
            .and_then(Value::as_u64)
            .ok_or_else(|| AptosError::BadReply(format!("no gas estimate in {v}")))
    }

    /// The chain id, so a transaction is never signed for the wrong network.
    pub async fn chain_id(&self) -> Result<u8, AptosError> {
        let v = self.get("").await?;
        v.get("chain_id")
            .and_then(Value::as_u64)
            .and_then(|n| u8::try_from(n).ok())
            .ok_or_else(|| AptosError::BadReply(format!("no chain id in {v}")))
    }

    /// Seconds since the epoch, from the node's own ledger.
    ///
    /// Aptos expires a transaction by wall clock, so the deadline has to be
    /// measured against the chain's clock rather than this machine's - a
    /// laptop a few minutes fast would build transactions that are already
    /// expired.
    pub async fn ledger_time_secs(&self) -> Result<u64, AptosError> {
        let v = self.get("").await?;
        let micros: u64 = v
            .get("ledger_timestamp")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| AptosError::BadReply(format!("no ledger timestamp in {v}")))?;
        Ok(micros / 1_000_000)
    }

    /// Ask the chain what a transaction would cost, without sending it.
    ///
    /// Returns the gas units used. The simulation needs a signature-shaped
    /// field and ignores its contents, which is why zeros are acceptable here
    /// and only here.
    pub async fn simulate(&self, signed_bcs: &[u8]) -> Result<u64, AptosError> {
        let r = self
            .http
            .post(format!("{}/transactions/simulate", self.base))
            .header("content-type", "application/x.aptos.signed_transaction+bcs")
            .body(signed_bcs.to_vec())
            .send()
            .await
            .map_err(|e| AptosError::Rpc(e.to_string()))?;
        let status = r.status();
        let text = r.text().await.map_err(|e| AptosError::Rpc(e.to_string()))?;
        if !status.is_success() {
            return Err(AptosError::Rpc(short(&text)));
        }
        let v: Value =
            serde_json::from_str(&text).map_err(|_| AptosError::BadReply(short(&text)))?;
        let first = v.as_array().and_then(|a| a.first()).unwrap_or(&v);
        if first.get("success").and_then(Value::as_bool) == Some(false) {
            let why = first
                .get("vm_status")
                .and_then(Value::as_str)
                .unwrap_or("the simulation failed");
            return Err(AptosError::Rpc(why.to_string()));
        }
        first
            .get("gas_used")
            .and_then(Value::as_str)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| AptosError::BadReply(format!("no gas_used in {first}")))
    }

    /// Broadcast. The BCS body is the same bytes that were signed plus the
    /// authenticator, so nothing is re-encoded between signing and sending.
    pub async fn submit(&self, signed_bcs: &[u8]) -> Result<String, AptosError> {
        let r = self
            .http
            .post(format!("{}/transactions", self.base))
            .header("content-type", "application/x.aptos.signed_transaction+bcs")
            .body(signed_bcs.to_vec())
            .send()
            .await
            .map_err(|e| AptosError::Rpc(e.to_string()))?;
        let status = r.status();
        let text = r.text().await.map_err(|e| AptosError::Rpc(e.to_string()))?;
        if !status.is_success() {
            return Err(AptosError::Rpc(short(&text)));
        }
        let v: Value =
            serde_json::from_str(&text).map_err(|_| AptosError::BadReply(short(&text)))?;
        v.get("hash")
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| AptosError::BadReply(format!("no hash in {v}")))
    }
}

/// Keep an error readable. A node's failure body can be a page of JSON, and
/// the useful part is at the front.
fn short(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= 300 {
        return t.to_string();
    }
    t.chars().take(300).collect::<String>() + "…"
}
