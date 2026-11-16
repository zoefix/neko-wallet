//! Sui's GraphQL RPC.
//!
//! Asked for five things: what an address holds, which coin objects it can
//! spend, what gas costs, what a transaction would cost, and the submission
//! itself - plus the history, in [`Rpc::transfers`].
//!
//! JSON-RPC is gone from every public node: Sui's own fullnode answers
//! "Method not found" and points at gRPC or GraphQL, and PublicNode answers
//! that the endpoint "has been deprecated". GraphQL is the one of the two that
//! travels as JSON over the HTTP this wallet already speaks. Every query below
//! was run against `graphql.mainnet.sui.io` (server 1.83) before it was
//! written down. The schema it serves is the 2025 one - `simulateTransaction`
//! and `executeTransaction`, not the beta's `dryRunTransactionBlock` and
//! `executeTransactionBlock`.

use std::time::Duration;

use neko_net::{Request, Service};
use serde_json::{json, Value};

use crate::address::SuiAddress;
use crate::error::SuiError;
use crate::graphql::{self, cut, int};
use crate::tx::ObjectRef;

pub struct Rpc {
    net: Service,
}

/// One coin object and what it holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Coin {
    pub object: ObjectRef,
    pub balance: u128,
}

/// The service's own query timeout is 40 seconds; waiting a little longer
/// lets its answer arrive instead of a bare "timed out".
const QUERY_TIMEOUT: Duration = Duration::from_secs(45);

/// `executeTransaction` waits for finality, for up to the service's 74-second
/// mutation timeout. Hanging up sooner turns "it failed" or "it went through"
/// into "nobody knows", which is the worst of the three to show.
const EXECUTE_TIMEOUT: Duration = Duration::from_secs(80);

/// `Address.balance`. `totalBalance` is coin objects plus the address's
/// balance accumulator - everything the address holds of that type.
const BALANCE: &str = "query($owner: SuiAddress!, $type: String!) { \
    address(address: $owner) { balance(coinType: $type) { totalBalance } } }";

/// `Address.objects` filtered to `0x2::coin::Coin<T>`, one page at a time.
/// `contents.json` is the coin's Move value, `{"id": .., "balance": ".."}`;
/// `version` arrives as a number and `digest` in base58.
const COINS: &str = "query($owner: SuiAddress!, $type: String!, $first: Int!, $after: String) { \
    address(address: $owner) { objects(first: $first, after: $after, filter: {type: $type}) { \
    pageInfo { hasNextPage endCursor } \
    nodes { address version digest contents { json } } } } }";

/// The service's largest page.
const COIN_PAGE: usize = 50;

/// Pages read before giving up on listing the rest. Four hundred objects is
/// far past what a transfer can fold together, and the largest of them are
/// what a transfer spends.
const MAX_COIN_PAGES: usize = 8;

const GAS_PRICE: &str = "query { epoch { referenceGasPrice } }";

/// `simulateTransaction` takes the transaction as JSON, and already-built
/// bytes as `{"bcs": {"value": <base64>}}`. Checks are on by default, which
/// is what makes it the same proof the old dry run was.
const SIMULATE: &str = "query($tx: JSON!) { simulateTransaction(transaction: $tx) { \
    effects { status executionError { message } \
    gasEffects { gasSummary { computationCost storageCost storageRebate } } } } }";

const EXECUTE: &str = "mutation($tx: Base64!, $sigs: [Base64!]!) { \
    executeTransaction(transactionDataBcs: $tx, signatures: $sigs) { \
    effects { digest status executionError { message } } } }";

/// Every transaction that touched the address, as sender, sponsor or
/// recipient - one query for both directions.
///
/// `last` because the connection runs oldest to newest; the newest page comes
/// back in ascending order. `balanceChangesJson` rather than the paginated
/// `balanceChanges`, whose first page can end before this address's row: an
/// airdrop checked while writing this changed 502 balances in one
/// transaction. The page size is capped at 50 by the service.
const HISTORY: &str = "query($who: SuiAddress!, $last: Int!) { \
    transactions(last: $last, filter: {affectedAddress: $who}) { nodes { \
    digest sender { address } gasInput { gasSponsor { address } } \
    effects { status timestamp balanceChangesJson \
    gasEffects { gasSummary { computationCost storageCost storageRebate } } } } } }";

const MAX_PAGE: usize = 50;

impl Rpc {
    /// A configured `url` is the only node asked. Without one, the default is
    /// asked first and [`crate::endpoints`] stand behind it.
    pub fn new(url: Option<&str>) -> Self {
        Rpc {
            net: Service::for_chain(url, crate::DEFAULT_API, crate::endpoints::FALLBACKS)
                .classify(graphql::classify)
                .timeout(QUERY_TIMEOUT),
        }
    }

    pub fn endpoint(&self) -> &str {
        self.net.primary()
    }

    async fn query(&self, document: &str, variables: Value) -> Result<Value, SuiError> {
        let body = json!({"query": document, "variables": variables});
        self.request(Request::post_json("", &body)).await
    }

    async fn request(&self, req: Request) -> Result<Value, SuiError> {
        let reply = self
            .net
            .send(req)
            .await
            .map_err(|e| SuiError::Rpc(e.to_string()))?;
        if !reply.is_success() && !reply.body.trim_start().starts_with('{') {
            return Err(SuiError::Rpc(format!(
                "the node answered HTTP {}{}",
                reply.status,
                if reply.rate_limited() {
                    " (rate limited)"
                } else {
                    ""
                }
            )));
        }
        graphql::data(&reply.body)
    }

    /// The total held of one coin type, in its own units.
    pub async fn balance(&self, who: SuiAddress, coin_type: &str) -> Result<u128, SuiError> {
        let v = self
            .query(
                BALANCE,
                json!({"owner": who.to_string(), "type": coin_type}),
            )
            .await?;
        v.pointer("/address/balance/totalBalance")
            .and_then(int)
            .and_then(|n| u128::try_from(n).ok())
            .ok_or_else(|| {
                SuiError::BadReply(format!("no totalBalance in {}", cut(&v.to_string())))
            })
    }

    /// The coin objects themselves.
    ///
    /// A transfer spends objects rather than a balance, so this is what a
    /// quote actually needs. Capped, because a transaction that folds together
    /// hundreds of objects is refused by the chain and the honest answer is to
    /// say so.
    ///
    /// Only objects: SUI an address holds in its balance accumulator is part
    /// of [`Rpc::balance`] but not of this list, and a transaction built from
    /// coin objects cannot spend it.
    ///
    /// Every object, not the first page. The service lists them in no useful
    /// order, so one page of a balance spread over many objects can hold a
    /// sliver of it - and a transfer chosen from that page is refused as
    /// unaffordable when it is not. The caller picks the largest.
    pub async fn coins(&self, who: SuiAddress, coin_type: &str) -> Result<Vec<Coin>, SuiError> {
        let mut out = Vec::new();
        let mut after: Option<String> = None;
        for _ in 0..MAX_COIN_PAGES {
            let v = self
                .query(
                    COINS,
                    json!({
                        "owner": who.to_string(),
                        "type": format!("0x2::coin::Coin<{coin_type}>"),
                        "first": COIN_PAGE,
                        "after": after,
                    }),
                )
                .await?;
            out.extend(coins_from(&v)?);
            after = next_page(&v);
            if after.is_none() {
                break;
            }
        }
        Ok(out)
    }

    /// What one unit of computation costs, from the chain's reference price.
    pub async fn reference_gas_price(&self) -> Result<u64, SuiError> {
        let v = self.query(GAS_PRICE, json!({})).await?;
        v.pointer("/epoch/referenceGasPrice")
            .and_then(int)
            .and_then(|n| u64::try_from(n).ok())
            .ok_or_else(|| SuiError::BadReply(format!("no gas price in {}", cut(&v.to_string()))))
    }

    /// Run a transaction without committing it, and report what it would cost.
    ///
    /// This is also what proves the bytes are right: a dry run that returns a
    /// cost has been parsed by the chain's own decoder.
    pub async fn dry_run(&self, data: &[u8]) -> Result<DryRun, SuiError> {
        let v = self
            .query(SIMULATE, json!({"tx": {"bcs": {"value": b64(data)}}}))
            .await?;
        dry_run_from(&v)
    }

    /// Transfers in both directions, newest first.
    ///
    /// One query, filtered on every transaction that *affected* the address
    /// rather than the ones it sent: a node can list what an account signed
    /// and nothing it received, and losing what you *received* is the
    /// specific failure this wallet has shipped before.
    pub async fn transfers(
        &self,
        who: SuiAddress,
        limit: usize,
    ) -> Result<Vec<crate::history::Transfer>, SuiError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let data = self
            .query(
                HISTORY,
                json!({"who": who.to_string(), "last": limit.min(MAX_PAGE)}),
            )
            .await?;
        let mut out = crate::history::parse(&json!({ "data": data }), who);
        out.truncate(limit);
        Ok(out)
    }

    /// Broadcast, and wait for the chain to have executed it.
    ///
    /// Never sent twice - see [`neko_net::Policy::Submit`]. Waiting for
    /// finality makes a timeout here likelier than anywhere else, and it is
    /// exactly the case where the transaction may well have gone through.
    pub async fn execute(&self, data: &[u8], signature: &[u8]) -> Result<String, SuiError> {
        let body = json!({
            "query": EXECUTE,
            "variables": {"tx": b64(data), "sigs": [b64(signature)]},
        });
        let req = Request::post_json("", &body)
            .submit()
            .timeout(EXECUTE_TIMEOUT);
        let v = self.request(req).await?;
        digest_from(&v)
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

/// The coin list out of a [`COINS`] reply.
fn coins_from(v: &Value) -> Result<Vec<Coin>, SuiError> {
    // An address the indexer has never seen owns nothing.
    if v.get("address").is_some_and(Value::is_null) {
        return Ok(Vec::new());
    }
    let rows = v
        .pointer("/address/objects/nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| SuiError::BadReply(format!("no coin list in {}", cut(&v.to_string()))))?;
    rows.iter().map(parse_coin).collect()
}

/// The cursor for the page after this one, if there is one.
fn next_page(v: &Value) -> Option<String> {
    let info = v.pointer("/address/objects/pageInfo")?;
    if info.get("hasNextPage").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    info.get("endCursor")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// The cost out of a [`SIMULATE`] reply, or why it would fail.
fn dry_run_from(v: &Value) -> Result<DryRun, SuiError> {
    let effects = v
        .pointer("/simulateTransaction/effects")
        .filter(|e| !e.is_null())
        .ok_or_else(|| SuiError::BadReply(format!("no effects in {}", cut(&v.to_string()))))?;
    if let Some(why) = failure(effects) {
        return Err(SuiError::Rpc(why));
    }
    let g = effects
        .pointer("/gasEffects/gasSummary")
        .filter(|g| !g.is_null())
        .ok_or_else(|| SuiError::BadReply("no gasSummary".into()))?;
    let num = |k: &str| -> u64 {
        g.get(k)
            .and_then(int)
            .and_then(|n| u64::try_from(n).ok())
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

/// The digest out of an [`EXECUTE`] reply.
///
/// A transaction that executed and *failed* is on chain and has charged its
/// gas, so it is reported as a failure that names its digest rather than as a
/// payment.
fn digest_from(v: &Value) -> Result<String, SuiError> {
    let effects = v
        .pointer("/executeTransaction/effects")
        .filter(|e| !e.is_null())
        .ok_or_else(|| SuiError::BadReply(format!("no effects in {}", cut(&v.to_string()))))?;
    let digest = effects
        .get("digest")
        .and_then(Value::as_str)
        .ok_or_else(|| SuiError::BadReply(format!("no digest in {}", cut(&v.to_string()))))?;
    if let Some(why) = failure(effects) {
        return Err(SuiError::Rpc(format!(
            "transaction {digest} executed and failed: {why}"
        )));
    }
    Ok(digest.to_string())
}

/// Why a transaction's effects say it failed, or `None` when they say
/// `SUCCESS`. An effects block with no status at all is not a success.
fn failure(effects: &Value) -> Option<String> {
    let status = effects.get("status").and_then(Value::as_str);
    if status == Some("SUCCESS") {
        return None;
    }
    let why = effects
        .pointer("/executionError/message")
        .and_then(Value::as_str)
        .or(status)
        .unwrap_or("no status");
    Some(cut(why))
}

fn parse_coin(v: &Value) -> Result<Coin, SuiError> {
    let id = v
        .get("address")
        .and_then(Value::as_str)
        .ok_or_else(|| SuiError::BadReply("a coin with no id".into()))?;
    let version = v
        .get("version")
        .and_then(int)
        .and_then(|n| u64::try_from(n).ok())
        .ok_or_else(|| SuiError::BadReply("a coin with no version".into()))?;
    let digest = v
        .get("digest")
        .and_then(Value::as_str)
        .ok_or_else(|| SuiError::BadReply("a coin with no digest".into()))?;
    let balance = v
        .pointer("/contents/json/balance")
        .and_then(int)
        .and_then(|n| u128::try_from(n).ok())
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

/// Base64, which is how GraphQL carries transaction bytes and signatures.
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

    /// Two USDC coins as `Address.objects` listed them on mainnet. The second
    /// digest is one character short because it starts with a zero byte.
    #[test]
    fn coin_objects_are_read_from_the_listing() {
        let v: Value = serde_json::from_str(
            r#"{"address":{"objects":{"nodes":[
                {"address":"0xfc8e0471392b8ac6a94e78f7afa3fa834cea59a9573e4d15870cacaf4f58cf7c",
                 "version":1076301636,"digest":"5EihoJVw25DvBSZ33vM3zCrre3kpR2gJPT1iKmUHWEjX",
                 "contents":{"json":{"id":"0xfc8e0471392b8ac6a94e78f7afa3fa834cea59a9573e4d15870cacaf4f58cf7c","balance":"2683167"}}},
                {"address":"0xe6c4112b07102b8e92bfd16d43c713a1443bc002172349d4b1d945212e7366df",
                 "version":1076301319,"digest":"1U6KUxcHgrwneVMdtZ78Z5tWScwCyThprLHjKirNtod",
                 "contents":{"json":{"id":"0xe6c4112b07102b8e92bfd16d43c713a1443bc002172349d4b1d945212e7366df","balance":"279006"}}}
            ]}}}"#,
        )
        .unwrap();
        let coins = coins_from(&v).unwrap();
        assert_eq!(coins.len(), 2);
        assert_eq!(coins[0].balance, 2_683_167);
        assert_eq!(coins[0].object.version, 1_076_301_636);
        assert_eq!(coins[0].object.id[0], 0xfc);
        assert_eq!(coins[1].object.digest[0], 0, "a leading '1' is a zero byte");
        assert_eq!(coins_from(&json!({"address": null})).unwrap(), vec![]);

        // The cursor is followed only while the service says there is more.
        let page = |more: bool| {
            json!({"address": {"objects": {
                "pageInfo": {"hasNextPage": more, "endCursor": "Y3Vyc29y"},
                "nodes": []}}})
        };
        assert_eq!(next_page(&page(true)).as_deref(), Some("Y3Vyc29y"));
        assert_eq!(next_page(&page(false)), None);
        assert_eq!(next_page(&json!({"address": null})), None);
        assert!(coins_from(&json!({"address": {}})).is_err());
    }

    /// A 0.001 SUI transfer, simulated on mainnet: the net cost subtracts the
    /// rebate.
    #[test]
    fn a_simulation_yields_its_cost() {
        let v: Value = serde_json::from_str(
            r#"{"simulateTransaction":{"effects":{"status":"SUCCESS","executionError":null,
                "gasEffects":{"gasSummary":{"computationCost":100000,"storageCost":1976000,"storageRebate":987901}}}}}"#,
        )
        .unwrap();
        assert_eq!(
            dry_run_from(&v).unwrap(),
            DryRun {
                computation: 100_000,
                storage: 1_976_000,
                rebate: 987_901,
                net: 1_088_099,
            }
        );
    }

    /// The same transfer for more than the coin holds. It is priced, and it is
    /// still a refusal - the reason is what gets shown.
    #[test]
    fn a_simulation_that_fails_is_a_refusal() {
        let v: Value = serde_json::from_str(
            r#"{"simulateTransaction":{"effects":{"status":"FAILURE",
                "executionError":{"message":"Error in 1st command, Insufficient coin balance for operation."},
                "gasEffects":{"gasSummary":{"computationCost":100000,"storageCost":988000,"storageRebate":987901}}}}}"#,
        )
        .unwrap();
        let e = dry_run_from(&v).unwrap_err().to_string();
        assert!(e.contains("Insufficient coin balance"), "{e}");
        assert!(dry_run_from(&json!({"simulateTransaction": {"effects": null}})).is_err());
    }

    /// Execution answers with effects. These two follow the schema's
    /// `ExecutionResult`; a real success was never broadcast to capture one.
    #[test]
    fn execution_reports_the_digest_or_why_it_failed() {
        let ok = json!({"executeTransaction": {"effects": {
            "digest": "5nHf1qUjnSVbL7d1xa6Hq7SnWLG9x36tWbPpohVV2Gue",
            "status": "SUCCESS", "executionError": null}}});
        assert_eq!(
            digest_from(&ok).unwrap(),
            "5nHf1qUjnSVbL7d1xa6Hq7SnWLG9x36tWbPpohVV2Gue"
        );
        let failed = json!({"executeTransaction": {"effects": {
            "digest": "5nHf1qUjnSVbL7d1xa6Hq7SnWLG9x36tWbPpohVV2Gue",
            "status": "FAILURE",
            "executionError": {"message": "Error in 1st command, Insufficient coin balance for operation."}}}});
        let e = digest_from(&failed).unwrap_err().to_string();
        assert!(e.contains("5nHf1qUj") && e.contains("Insufficient"), "{e}");
    }

    /// What mainnet answered when this wallet's bytes went out under an
    /// all-zero signature: refused, in the `errors` array of an HTTP 200.
    #[test]
    fn a_refused_submission_carries_the_nodes_reason() {
        let body = r#"{"data":null,"errors":[{"message":"Invalid argument: Invalid user signature: Required Signature from 0xa5979f4b26f3e9423bf96dee502dcee177fbff6f4a62b292261d43037d98e2b3 is absent [\"0xd8908c165dee785924e7421a0fd0418a19d5daeec395fd505a92a0fd3117e428\"]","locations":[{"line":1,"column":45}],"path":["executeTransaction"],"extensions":{"code":"BAD_USER_INPUT"}}]}"#;
        let e = graphql::data(body).unwrap_err().to_string();
        assert!(e.contains("Invalid user signature"), "{e}");
        assert_eq!(graphql::classify(&neko_net::Reply::new(200, body)), None);
    }
}
