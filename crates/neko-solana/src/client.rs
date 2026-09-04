//! JSON-RPC against a Solana cluster.
//!
//! Same contract as the other two chains': the node supplies facts - a balance,
//! a blockhash, a rent figure - and never assembles a transaction. The bytes
//! are built and signed here, so a node that has been replaced can make this
//! wallet fail but not make it sign a transfer to somebody else.
//!
//! One Solana-specific rule shows up in the retry logic. A blockhash is only
//! good for about a minute, so a slow retry loop can hand back something that
//! has already expired. Fetching one is therefore cheap and late rather than
//! cached.

use neko_hd::SolanaAddress;
use serde_json::{json, Value};

use crate::chain_consts;
use crate::error::SolanaError;

/// SPL token accounts are a fixed 165 bytes; the rent for that size is what a
/// first transfer to a new holder has to cover.
pub const TOKEN_ACCOUNT_BYTES: usize = 165;

pub struct Rpc {
    url: String,
    http: reqwest::Client,
}

/// A blockhash, with the block height past which it is worthless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Blockhash {
    pub hash: [u8; 32],
    pub last_valid_block_height: u64,
}

/// What one account holds of one token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenBalance {
    pub amount: u64,
    /// Read from the chain, never assumed from a constant.
    pub decimals: u8,
}

impl Rpc {
    pub fn new(url: Option<&str>) -> Self {
        Rpc {
            url: url
                .filter(|u| !u.is_empty())
                .unwrap_or(chain_consts::DEFAULT_RPC)
                .to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    async fn call(&self, method: &str, params: Value) -> Result<Value, SolanaError> {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
        let mut last = None;
        for attempt in 0..3u32 {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(1 << (attempt - 1))).await;
            }
            let resp = match self.http.post(&self.url).json(&body).send().await {
                Ok(r) => r,
                Err(e) => {
                    last = Some(SolanaError::Network(e.to_string()));
                    continue;
                }
            };
            // 429 is the public endpoint's normal state under load, and it is
            // worth another attempt; the other 4xx are decisions.
            if resp.status().is_server_error() || resp.status().as_u16() == 429 {
                last = Some(SolanaError::Network(format!("HTTP {}", resp.status())));
                continue;
            }
            if !resp.status().is_success() {
                return Err(SolanaError::Network(format!("HTTP {}", resp.status())));
            }
            let v: Value = resp
                .json()
                .await
                .map_err(|e| SolanaError::BadReply(e.to_string()))?;
            if let Some(err) = v.get("error") {
                let msg = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                return Err(SolanaError::Rpc(msg.to_string()));
            }
            return v
                .get("result")
                .cloned()
                .ok_or_else(|| SolanaError::BadReply("reply has no result".into()));
        }
        Err(last.unwrap_or_else(|| SolanaError::Network("no attempt succeeded".into())))
    }

    /// Lamports held by an address.
    pub async fn balance(&self, addr: SolanaAddress) -> Result<u64, SolanaError> {
        let v = self
            .call(
                "getBalance",
                json!([addr.to_string(), {"commitment": "confirmed"}]),
            )
            .await?;
        v.get("value")
            .and_then(Value::as_u64)
            .ok_or_else(|| SolanaError::BadReply("getBalance returned no value".into()))
    }

    /// What an address holds of a mint.
    ///
    /// An address that has never held the token has no account for it, and that
    /// is `Ok(None)` - a fact about the account, not a failure to read it. The
    /// distinction matters because the two lead to different screens: one says
    /// "zero", the other says "sending here costs extra".
    pub async fn token_balance(
        &self,
        owner: SolanaAddress,
        mint: SolanaAddress,
    ) -> Result<Option<TokenBalance>, SolanaError> {
        let ata = crate::pda::associated_token_address(&owner, &mint)?;
        let v = self
            .call(
                "getTokenAccountBalance",
                json!([ata.to_string(), {"commitment": "confirmed"}]),
            )
            .await;
        let v = match v {
            Ok(v) => v,
            // The cluster reports a missing token account as an invalid-param
            // error rather than a null, so it has to be read out of the text.
            Err(SolanaError::Rpc(msg)) if msg.contains("could not find account") => {
                return Ok(None)
            }
            Err(e) => return Err(e),
        };
        let val = v
            .get("value")
            .ok_or_else(|| SolanaError::BadReply("no value".into()))?;
        let amount = val
            .get("amount")
            .and_then(Value::as_str)
            .and_then(|s| s.parse::<u64>().ok())
            .ok_or_else(|| SolanaError::BadReply("token amount is not a number".into()))?;
        let decimals = val
            .get("decimals")
            .and_then(Value::as_u64)
            .ok_or_else(|| SolanaError::BadReply("token account has no decimals".into()))?
            as u8;
        Ok(Some(TokenBalance { amount, decimals }))
    }

    /// Whether an address already has an account for this mint.
    ///
    /// The answer decides whether a transfer costs a fee or a fee plus two
    /// millionths of a SOL in rent, so it is asked before every token transfer
    /// rather than guessed from whether a balance came back.
    pub async fn has_token_account(
        &self,
        owner: SolanaAddress,
        mint: SolanaAddress,
    ) -> Result<bool, SolanaError> {
        let ata = crate::pda::associated_token_address(&owner, &mint)?;
        let v = self
            .call(
                "getAccountInfo",
                json!([ata.to_string(), {"commitment": "confirmed", "encoding": "base64"}]),
            )
            .await?;
        Ok(!v.get("value").map(Value::is_null).unwrap_or(true))
    }

    /// A mint's on-chain symbol is not in the mint account, but its precision
    /// is - and precision is the number that silently multiplies a transfer by
    /// a million if it is wrong.
    pub async fn mint_decimals(&self, mint: SolanaAddress) -> Result<u8, SolanaError> {
        let v = self
            .call(
                "getAccountInfo",
                json!([mint.to_string(), {"commitment": "confirmed", "encoding": "jsonParsed"}]),
            )
            .await?;
        v.get("value")
            .and_then(|v| v.get("data"))
            .and_then(|d| d.get("parsed"))
            .and_then(|p| p.get("info"))
            .and_then(|i| i.get("decimals"))
            .and_then(Value::as_u64)
            .map(|d| d as u8)
            .ok_or_else(|| SolanaError::BadReply(format!("{mint} does not look like an SPL mint")))
    }

    /// A blockhash to sign against.
    ///
    /// Valid for roughly a minute. Callers fetch this immediately before
    /// signing, never at quote time.
    pub async fn latest_blockhash(&self) -> Result<Blockhash, SolanaError> {
        let v = self
            .call("getLatestBlockhash", json!([{"commitment": "confirmed"}]))
            .await?;
        let val = v
            .get("value")
            .ok_or_else(|| SolanaError::BadReply("no value".into()))?;
        let s = val
            .get("blockhash")
            .and_then(Value::as_str)
            .ok_or_else(|| SolanaError::BadReply("no blockhash".into()))?;
        let raw = bs58::decode(s)
            .into_vec()
            .map_err(|_| SolanaError::BadReply("blockhash is not base58".into()))?;
        let hash: [u8; 32] = raw
            .try_into()
            .map_err(|_| SolanaError::BadReply("blockhash is not 32 bytes".into()))?;
        Ok(Blockhash {
            hash,
            last_valid_block_height: val
                .get("lastValidBlockHeight")
                .and_then(Value::as_u64)
                .unwrap_or(0),
        })
    }

    /// The rent a token account has to be funded with, asked rather than
    /// assumed - it is a cluster parameter and has changed before.
    pub async fn token_account_rent(&self) -> Result<u64, SolanaError> {
        let v = self
            .call(
                "getMinimumBalanceForRentExemption",
                json!([TOKEN_ACCOUNT_BYTES]),
            )
            .await?;
        v.as_u64()
            .ok_or_else(|| SolanaError::BadReply("rent is not a number".into()))
    }

    /// A priority fee that recent blocks actually accepted.
    ///
    /// Solana drops rather than queues: a transaction with too low a priority
    /// during congestion is not slow, it never lands, and the blockhash expires.
    /// The median of what got included is a better guess than zero.
    pub async fn priority_fee(&self, accounts: &[SolanaAddress]) -> Result<u64, SolanaError> {
        let keys: Vec<String> = accounts.iter().map(|a| a.to_string()).collect();
        let v = self
            .call("getRecentPrioritizationFees", json!([keys]))
            .await?;
        let mut fees: Vec<u64> = v
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|e| e.get("prioritizationFee").and_then(Value::as_u64))
                    .collect()
            })
            .unwrap_or_default();
        if fees.is_empty() {
            return Ok(0);
        }
        fees.sort_unstable();
        Ok(fees[fees.len() / 2])
    }

    /// Broadcast. Returns the signature, which is also the transaction id.
    pub async fn send(&self, raw: &[u8]) -> Result<String, SolanaError> {
        let encoded = base64(raw);
        let v = self
            .call(
                "sendTransaction",
                json!([encoded, {"encoding": "base64", "preflightCommitment": "confirmed", "maxRetries": 3}]),
            )
            .await?;
        v.as_str()
            .map(str::to_string)
            .ok_or_else(|| SolanaError::BadReply("sendTransaction returned no signature".into()))
    }
}

/// Standard base64. Written out rather than pulled in as a dependency: it is
/// twenty lines, and the alternative is another crate in the signing path.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4648's own vectors, including every padding case.
    #[test]
    fn base64_matches_the_rfc() {
        for (input, want) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"fooba", "Zm9vYmE="),
            (b"foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(input), want, "encoding {input:?}");
        }
        // The bytes that exercise the last two alphabet entries.
        assert_eq!(base64(&[0xfb, 0xff, 0xfe]), "+//+");
    }
}
