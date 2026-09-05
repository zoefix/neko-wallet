//! Transaction history for BNB Chain.
//!
//! A node's own RPC cannot answer "what has this address done" - that needs an
//! index, and building one means replaying the chain. So this talks to a
//! provider, and the choice of provider is not incidental:
//!
//! * Etherscan's V2 API withdrew free access for BNB Chain (along with
//!   Avalanche, Base and OP) in favour of paid plans, and the old
//!   `api.bscscan.com` is deprecated outright.
//! * `eth_getLogs` needs no provider, but **native BNB transfers emit no
//!   logs**, so it can only ever return token movements. A history that
//!   silently omits every BNB transfer is worse than none: it looks complete.
//!   Public endpoints also cap the block range at a couple of hours.
//!
//! BSCTrace, via NodeReal's MegaNode, is BNB Chain's own recommended
//! replacement, has a free tier, and its `nr_getAssetTransfers` covers the
//! `external` category - which is exactly the native transfers `eth_getLogs`
//! cannot see.
//!
//! What a token movement is called on screen is `EvmChain::stable_label`,
//! never a name out of a reply. Two reasons:
//!
//! * **A token can call itself anything.** Several in a typical address's
//!   history call themselves USDT in characters that are not the ones in
//!   USDT, and a name taken from a server is a name an attacker chose.
//! * **One holding, one name.** The balance screen and the history have to
//!   agree. Polygon's contract is named `USDT0` and is shown as USDT; Base's
//!   stablecoin is USDC and is shown as USDC.
//!
//! What the contract calls itself is still checked against the chain before a
//! transfer is signed - see `EvmChain::stable_symbol`. That check is about
//! whether this is the right contract; the label is about what to call it.

use neko_hd::EvmAddress;
use serde_json::{json, Value};

use crate::error::EvmError;

#[allow(dead_code)]
pub const DEFAULT_HOST: &str = "https://bsc-mainnet.nodereal.io/v1";
pub const SIGNUP_URL: &str = "https://nodereal.io";

/// One movement of value, as the provider reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transfer {
    pub hash: String,
    pub from: String,
    pub to: String,
    /// Minimal units. Never a float.
    pub amount: i128,
    pub symbol: String,
    pub decimals: u8,
    /// **Milliseconds** since the epoch, not seconds.
    ///
    /// Every provider states it differently - NodeReal in milliseconds,
    /// Etherscan and Blockscout in seconds - and the screen renders whatever
    /// arrives. A figure in seconds shown as milliseconds is January 1970,
    /// which has happened here once already, so the unit is part of the type's
    /// contract and each parser converts.
    pub block_ts: i64,
    pub success: bool,
}

pub struct Bsctrace {
    /// Decides the symbol and the precision of what comes back. USDT is six
    /// decimals on Ethereum and eighteen on BNB Chain.
    chain: crate::EvmChain,
    url: String,
    http: reqwest::Client,
}

impl Bsctrace {
    /// `api_key` is required: the endpoint carries it in the path, and there
    /// is no anonymous access.
    ///
    /// `None` for a chain NodeReal does not index. Returning a client that
    /// would post to a host that does not resolve reports "no transfer index
    /// for this chain" as a network failure, which reads as the wallet being
    /// broken rather than the feature being absent.
    pub fn new(chain: crate::EvmChain, api_key: &str) -> Option<Self> {
        Some(Bsctrace {
            chain,
            url: format!("{}/{api_key}", chain.history_host?),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        })
    }

    async fn call(&self, params: Value) -> Result<Value, EvmError> {
        let body = json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "nr_getAssetTransfers",
            "params": [params],
        });
        let resp = self
            .http
            .post(&self.url)
            .json(&body)
            .send()
            .await
            .map_err(|e| EvmError::Network(e.to_string()))?;
        if !resp.status().is_success() {
            return Err(EvmError::Network(format!("HTTP {}", resp.status())));
        }
        let v: Value = resp
            .json()
            .await
            .map_err(|e| EvmError::BadReply(e.to_string()))?;
        if let Some(err) = v.get("error") {
            let msg = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("unknown error");
            // The one error worth naming, because the fix is a specific action
            // rather than "try again".
            if msg.contains("Unauthorized") {
                return Err(EvmError::Rpc(format!(
                    "the BNB Chain history key was rejected - check it, or get one free at {SIGNUP_URL}"
                )));
            }
            return Err(EvmError::Rpc(msg.to_string()));
        }
        v.get("result")
            .cloned()
            .ok_or_else(|| EvmError::BadReply("reply has no result".into()))
    }

    /// Everything this address sent or received, newest first.
    ///
    /// Two calls, because the provider filters on one direction at a time.
    /// A failure of either alone is not fatal - half a history beats none, and
    /// the caller is told nothing is missing only when nothing is.
    pub async fn transfers(
        &self,
        who: EvmAddress,
        token: EvmAddress,
        limit: usize,
    ) -> Result<Vec<Transfer>, EvmError> {
        let who_s = who.to_string();

        // No `contractAddresses` filter. It looks like the obvious way to ask
        // for one token, and it silently drops every *native* transfer with it
        // - a native transfer has no contract, so nothing matches. Coin
        // transfers were missing from history entirely because of this.
        //
        // Filtering here is also the stronger check. The reply carries a
        // contract address per row, and that is what a token is; `asset` is a
        // name the token itself chose, and this address's history contains
        // `꒤SDT` and `U឵S឵DΤ` - Unicode lookalikes of USDT, from contracts
        // nobody should be shown as USDT.
        let out_v = self.call(request("fromAddress", &who_s)).await;
        let in_v = self.call(request("toAddress", &who_s)).await;

        let mut all = Vec::new();
        let mut errors = Vec::new();
        for (r, outgoing) in [(out_v, true), (in_v, false)] {
            match r {
                Ok(v) => all.extend(parse_direction(&v, self.chain, token, outgoing)),
                Err(e) => errors.push(e),
            }
        }
        // Both directions failed: there is nothing to show and a reason to give.
        if all.is_empty() {
            if let Some(e) = errors.into_iter().next() {
                return Err(e);
            }
        }

        all.sort_by(|a, b| b.block_ts.cmp(&a.block_ts).then(b.hash.cmp(&a.hash)));
        // A transfer to yourself appears in both directions; keep both, since
        // that is genuinely two movements, but drop exact duplicates.
        all.dedup_by(|a, b| a.hash == b.hash && a.from == b.from && a.to == b.to);
        all.truncate(limit);
        Ok(all)
    }
}

/// Read the provider's reply, skipping anything malformed rather than failing
/// the whole page: one odd row must not hide a history.
pub fn parse(result: &Value, chain: crate::EvmChain, token: EvmAddress) -> Vec<Transfer> {
    let Some(rows) = result.get("transfers").and_then(Value::as_array) else {
        return Vec::new();
    };
    rows.iter()
        .filter_map(|t| parse_one(t, chain, token))
        .collect()
}

/// What is asked of the provider, for one direction.
///
/// No `contractAddresses`. See `transfers`: constraining it drops every native
/// transfer, because a native transfer has no contract to match.
pub fn request(direction: &str, address: &str) -> Value {
    json!({
        direction: address,
        "category": ["external", "20"],
        "withMetadata": true,
        // A zero-value transfer is a way to get an address into somebody's
        // history, so it is worth seeing rather than filtering away.
        "excludeZeroValue": false,
        "order": "desc",
    })
}

/// One direction's reply, reduced to the transfers worth listing.
///
/// A transfer of nothing, *out* of this address, is not something this address
/// did. On an EVM chain a token transfer is a contract call carrying zero
/// native coin, and the provider reports it under both categories - so every
/// token transfer came back twice: once as itself, and once as "sent 0 ETH"
/// with the same timestamp.
///
/// Incoming zeroes are kept. A zero-value transfer *to* you is how an address
/// gets into your history, and the dust filter already hides those behind a key
/// rather than discarding them.
pub fn parse_direction(
    result: &Value,
    chain: crate::EvmChain,
    token: EvmAddress,
    outgoing: bool,
) -> Vec<Transfer> {
    parse(result, chain, token)
        .into_iter()
        .filter(|t| !outgoing || t.amount != 0)
        .collect()
}

/// The address a native transfer reports instead of a contract.
const NO_CONTRACT: &str = "0x0000000000000000000000000000000000000000";

fn parse_one(t: &Value, chain: crate::EvmChain, token: EvmAddress) -> Option<Transfer> {
    let category = t.get("category")?.as_str()?;
    // The precision comes from the chain, not from a constant: USDT is six
    // decimals on Ethereum and eighteen on BNB Chain. Anything that is not the
    // native coin or that token is skipped rather than shown with a made-up
    // scale.
    let contract = t
        .get("contractAddress")
        .and_then(Value::as_str)
        .unwrap_or(NO_CONTRACT);

    let (symbol, decimals) = match category {
        "external" | "internal" => (chain.native_symbol.to_string(), chain.native_decimals),
        // Matched on the contract, and the symbol comes from *our* constant
        // rather than from the row. A token can call itself anything, and
        // several in this address's history call themselves USDT in characters
        // that are not the ones in USDT.
        "20" if contract.eq_ignore_ascii_case(&token.to_string()) => {
            (chain.stable_label.to_string(), chain.stable_decimals)
        }
        _ => return None,
    };
    Some(Transfer {
        hash: t.get("hash")?.as_str()?.to_string(),
        from: t.get("from")?.as_str()?.to_string(),
        to: t.get("to")?.as_str()?.to_string(),
        amount: parse_value(t.get("value")?.as_str()?)?,
        symbol,
        decimals,
        // Seconds, and every other chain here reports milliseconds. Without
        // the conversion every EVM transfer was dated to January 1970.
        block_ts: t
            .get("blockTimeStamp")
            .and_then(Value::as_i64)
            .unwrap_or(0)
            .saturating_mul(1000),
        // Absent means the provider did not say; treating that as failure
        // would mark real transfers as failed.
        success: t
            .get("receiptsStatus")
            .and_then(Value::as_i64)
            .map(|s| s == 1)
            .unwrap_or(true),
    })
}

/// Amounts arrive as hex, sometimes padded to 32 bytes and sometimes minimal.
///
/// Refuses anything above 128 bits rather than wrapping: a wrapped amount
/// would be displayed as a real figure.
fn parse_value(s: &str) -> Option<i128> {
    let body = s.trim_start_matches("0x").trim_start_matches("0X");
    let body = body.trim_start_matches('0');
    if body.is_empty() {
        return Some(0);
    }
    if body.len() > 31 {
        return None;
    }
    i128::from_str_radix(body, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_survive_padding_and_refuse_overflow() {
        // The padded form the provider actually sends.
        assert_eq!(
            parse_value("0x000000000000000000000000000000000000000000000021ef7ec34f52880000"),
            Some(626_000_000_000_000_000_000)
        );
        // And the minimal form.
        assert_eq!(parse_value("0x232773380d7000"), Some(9_895_000_000_000_000));
        assert_eq!(parse_value("0x0"), Some(0));
        assert_eq!(parse_value("0x"), Some(0));
        // Over 128 bits: refused, never wrapped into a plausible number.
        assert_eq!(parse_value(&format!("0x1{}", "0".repeat(31))), None);
        assert_eq!(parse_value("0xzz"), None);
    }

    /// The real reply shape, taken from the provider.
    #[test]
    fn a_native_transfer_is_read_correctly() {
        let v = serde_json::json!({"transfers": [{
            "category": "external",
            "from": "0x9858effd232b4033e47d90003d41ec34ecaeda94",
            "to": "0x5eae506f855895a3d99c3e6863b3c01600301ffd",
            "value": "0x232773380d7000",
            "asset": "BNB",
            "hash": "0xcb01c531b6d1642dd8aebcb2d88f8ec884ebabb7f75a281e0f007ae27de3ea26",
            "blockTimeStamp": 1620515273,
            "receiptsStatus": 1
        }]});
        let got = parse(&v, crate::BSC, crate::BSC.stable_address());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].symbol, "BNB");
        assert_eq!(got[0].decimals, 18);
        assert_eq!(got[0].amount, 9_895_000_000_000_000);
        assert!(got[0].success);
        // Milliseconds. The provider reports seconds and every other chain
        // here reports milliseconds, so the conversion happens once, here -
        // without it every EVM transfer was dated to January 1970.
        assert_eq!(got[0].block_ts, 1_620_515_273_000);
    }

    #[test]
    fn a_token_transfer_is_read_correctly() {
        let v = serde_json::json!({"transfers": [{
            "category": "20",
            "from": "0x8fa75b899f47133df83667b7bb3bc36f1aac27f6",
            "to": "0x8894e0a0c962cb723c1976a4421c95949be2d4e3",
            "value": "0x000000000000000000000000000000000000000000000021ef7ec34f52880000",
            "asset": "USDT",
            "contractAddress": "0x55d398326f99059ff775485246999027b3197955",
            "hash": "0xf2fb91086074132e8d6814178c1d5ae69aaae370b66801f608bfba9f61ffc1c2",
            "blockTimeStamp": 1788469375,
            "receiptsStatus": 1
        }]});
        let got = parse(&v, crate::BSC, crate::BSC.stable_address());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].symbol, "USDT");
        // Eighteen here, six on TRON. The number travels with the transfer.
        assert_eq!(got[0].decimals, 18);
        assert_eq!(got[0].amount, 626_000_000_000_000_000_000);
    }

    /// The reply comes from a provider we do not control, so every shape of
    /// nonsense must be skipped rather than panic or become a wrong number.
    #[test]
    fn malformed_rows_are_skipped_not_fatal() {
        let v = serde_json::json!({"transfers": [
            {"category": "external"},                       // missing everything
            {"category": "nft", "hash": "0x1"},             // a category we do not show
            {"category": "20", "from": "0xa", "to": "0xb",
             "value": "0xnothex", "asset": "USDT", "hash": "0x2"},
            {"category": "external", "from": "0xa", "to": "0xb",
             "value": "0x1", "hash": "0x3", "blockTimeStamp": 5, "receiptsStatus": 0},
        ]});
        let got = parse(&v, crate::BSC, crate::BSC.stable_address());
        assert_eq!(got.len(), 1, "a good row was lost among bad ones");
        assert_eq!(got[0].hash, "0x3");
        assert!(!got[0].success, "a failed transfer was shown as successful");

        // A row with no transaction hash cannot be shown or looked up, so it
        // is skipped. Worth its own case: it is exactly what a truncated
        // reply looks like.
        assert!(parse(
            &serde_json::json!({"transfers": [{
                "category": "20", "from": "0xa", "to": "0xb",
                "value": "0x1", "asset": "USDT", "blockTimeStamp": 1
            }]}),
            crate::BSC,
            crate::BSC.stable_address()
        )
        .is_empty());

        assert!(parse(
            &serde_json::json!({}),
            crate::BSC,
            crate::BSC.stable_address()
        )
        .is_empty());
        assert!(parse(
            &serde_json::json!({"transfers": "nope"}),
            crate::BSC,
            crate::BSC.stable_address()
        )
        .is_empty());
    }
}

/// Two faults that between them hid the user's own coin transfer and dressed
/// somebody else's token up as USDT.
#[cfg(test)]
mod filtering {
    use super::*;

    fn usdt() -> EvmAddress {
        crate::ETHEREUM.stable_address()
    }

    fn row(category: &str, contract: &str, asset: &str, value: &str) -> Value {
        serde_json::json!({
            "category": category,
            "contractAddress": contract,
            "asset": asset,
            "from": "0xa41811cf4d41e306310cb82b47258c22b80475cc",
            "to": "0x74224e8d997f1c438cbfd2ce147c8bbdcd5fa0c8",
            "value": value,
            "hash": "0x83187425445cbfde9cacdf1cfe3ff3acd9cab055a57174fdb27cb5b10f337210",
            "blockTimeStamp": 1_788_575_867i64,
            "receiptsStatus": 1,
        })
    }

    /// Asking the provider to filter by contract looks like the obvious way to
    /// want one token, and it drops every native transfer with it - a native
    /// transfer has no contract, so nothing matches. Coin transfers were
    /// missing from history entirely.
    #[test]
    fn the_request_does_not_constrain_the_contract() {
        let r = request("fromAddress", "0xabc");
        assert_eq!(r["fromAddress"], "0xabc");
        assert_eq!(r["category"], serde_json::json!(["external", "20"]));
        assert!(
            r.get("contractAddresses").is_none(),
            "constraining the contract server-side hides every native transfer"
        );
    }

    /// The transfer the user actually made, as the provider reported it.
    #[test]
    fn a_native_transfer_survives() {
        let v = serde_json::json!({"transfers": [row(
            "external",
            "0x0000000000000000000000000000000000000000",
            "ETH",
            "0x06d492a3e1a134",
        )]});
        let got = parse(&v, crate::ETHEREUM, usdt());
        assert_eq!(got.len(), 1, "the coin transfer was dropped");
        assert_eq!(got[0].symbol, "ETH");
        assert_eq!(got[0].decimals, 18);
        assert_eq!(got[0].amount, 1_922_576_140_050_740);
        assert_eq!(
            got[0].block_ts, 1_788_575_867_000,
            "seconds, not milliseconds"
        );
    }

    /// A token is what its contract is, not what it calls itself.
    ///
    /// These two names are from this wallet's own history: `꒤SDT` and
    /// `U឵S឵DΤ` are Unicode lookalikes of USDT from contracts that are not
    /// Tether's. Shown as USDT they would make a scam transfer indistinguishable
    /// from a real one.
    #[test]
    fn a_token_that_calls_itself_usdt_is_not_usdt() {
        for (contract, name) in [
            ("0xde7a933accd1a2e7d8c6f5ab4b77afd74b6f34f3", "\u{a4a4}SDT"),
            ("0xde7a933accd1a2e7d8c6f5ab4b77afd74b6f34f3", "USDT"),
            ("0x1111111111111111111111111111111111111111", "USDT"),
        ] {
            let v = serde_json::json!({"transfers": [row("20", contract, name, "0x47b760")]});
            assert!(
                parse(&v, crate::ETHEREUM, usdt()).is_empty(),
                "{name:?} from {contract} was accepted as USDT"
            );
        }

        // ...and the real one is, whatever it calls itself.
        let v = serde_json::json!({"transfers": [row(
            "20",
            "0xdAC17F958D2ee523a2206206994597C13D831ec7",
            "anything at all",
            "0x47b760",
        )]});
        let got = parse(&v, crate::ETHEREUM, usdt());
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].symbol, "USDT", "the name comes from us, not the row");
        assert_eq!(got[0].decimals, 6, "six on Ethereum, eighteen on BNB Chain");
        assert_eq!(got[0].amount, 4_700_000);
    }

    /// Case is not part of an address, and providers disagree about it.
    #[test]
    fn the_contract_match_ignores_case() {
        let v = serde_json::json!({"transfers": [row(
            "20",
            "0xdac17f958d2ee523a2206206994597c13d831ec7",
            "USDT",
            "0x1",
        )]});
        assert_eq!(parse(&v, crate::ETHEREUM, usdt()).len(), 1);
    }
}

/// One token transfer must produce one row.
#[cfg(test)]
mod one_row_per_transfer {
    use super::*;

    const MINE: &str = "0xa41811cf4d41e306310cb82b47258c22b80475cc";
    const THEM: &str = "0x74224e8d997f1c438cbfd2ce147c8bbdcd5fa0c8";
    const HASH: &str = "0x09bb3994d69ac3e1000000000000000000000000000000000000000000000000";

    fn row(
        category: &str,
        contract: &str,
        asset: &str,
        value: &str,
        from: &str,
        to: &str,
    ) -> Value {
        serde_json::json!({
            "category": category, "contractAddress": contract, "asset": asset,
            "from": from, "to": to, "value": value, "hash": HASH,
            "blockTimeStamp": 1_788_571_067i64, "receiptsStatus": 1,
        })
    }

    /// Exactly what the provider returns for one USDT transfer: the token
    /// movement, and the transaction that carried it with no coin in it.
    #[test]
    fn a_token_transfer_is_not_also_a_zero_coin_transfer() {
        let v = serde_json::json!({"transfers": [
            row("external", "0x0000000000000000000000000000000000000000", "ETH", "0x0", MINE, THEM),
            row("20", "0xdAC17F958D2ee523a2206206994597C13D831ec7", "USDT", "0x47b760", MINE, THEM),
        ]});
        let usdt = crate::ETHEREUM.stable_address();

        // Unfiltered, both rows are there - which is what was on screen.
        assert_eq!(parse(&v, crate::ETHEREUM, usdt).len(), 2);

        let got = parse_direction(&v, crate::ETHEREUM, usdt, true);
        assert_eq!(got.len(), 1, "the transfer was listed twice");
        assert_eq!(got[0].symbol, "USDT");
        assert_eq!(got[0].amount, 4_700_000);
    }

    /// A real coin transfer is not a zero, and must survive untouched.
    #[test]
    fn a_real_coin_transfer_is_kept() {
        let v = serde_json::json!({"transfers": [row(
            "external", "0x0000000000000000000000000000000000000000",
            "ETH", "0x06d492a3e1a134", MINE, THEM,
        )]});
        let got = parse_direction(&v, crate::ETHEREUM, crate::ETHEREUM.stable_address(), true);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].amount, 1_922_576_140_050_740);
    }

    /// The other direction keeps its zeroes: a zero-value transfer *to* you is
    /// how somebody gets their address into your history, and the dust filter
    /// is what decides whether to show it - not this.
    #[test]
    fn an_incoming_zero_is_kept_for_the_dust_filter() {
        let v = serde_json::json!({"transfers": [row(
            "20", "0xdAC17F958D2ee523a2206206994597C13D831ec7",
            "USDT", "0x0", THEM, MINE,
        )]});
        let got = parse_direction(&v, crate::ETHEREUM, crate::ETHEREUM.stable_address(), false);
        assert_eq!(
            got.len(),
            1,
            "a poisoning attempt was discarded rather than hidden"
        );
        assert_eq!(got[0].amount, 0);
    }
}

#[cfg(test)]
mod unit_agreement {
    use super::*;
    use serde_json::json;

    /// Every provider must hand back the same instant in the same unit.
    ///
    /// They do not state it the same way: NodeReal writes milliseconds,
    /// Etherscan and Blockscout write seconds. `Transfer::block_ts` is
    /// milliseconds and the screen renders it directly, so a parser that
    /// forwards seconds dates every row to January 1970. That shipped once,
    /// for NodeReal, and was reintroduced the day a second provider was added
    /// - which is what this test is for.
    #[test]
    fn every_provider_reports_the_same_instant_in_milliseconds() {
        const WHEN: i64 = 1_782_513_790;
        const MS: i64 = WHEN * 1_000;
        let from = "0x1111111111111111111111111111111111111111";
        let to = "0x2222222222222222222222222222222222222222";

        let blockscout = crate::blockscout::parse_coins(
            &json!({"items": [{
                "timestamp": "2026-06-26T22:43:10.000000Z",
                "hash": "0xa", "from": {"hash": from}, "to": {"hash": to},
                "value": "1", "status": "ok",
            }]}),
            crate::POLYGON,
        );
        let etherscan = crate::etherscan::parse_coins(
            &json!([{
                "hash": "0xa", "from": from, "to": to,
                "value": "1", "timeStamp": WHEN.to_string(), "isError": "0",
            }]),
            crate::POLYGON,
        );
        let nodereal = parse_direction(
            &json!({"transfers": [{
                "hash": "0xa", "from": from, "to": to,
                "value": "0x1", "category": "external",
                "blockTimeStamp": WHEN, "asset": "POL",
            }]}),
            crate::POLYGON,
            crate::POLYGON.stable_address(),
            false,
        );

        for (name, rows) in [
            ("blockscout", &blockscout),
            ("etherscan", &etherscan),
            ("nodereal", &nodereal),
        ] {
            assert_eq!(rows.len(), 1, "{name} parsed nothing");
            assert_eq!(
                rows[0].block_ts, MS,
                "{name} reported {} - seconds where milliseconds were expected \
                 puts every row in 1970",
                rows[0].block_ts
            );
        }
    }
}

#[cfg(test)]
mod token_naming {
    use super::*;
    use serde_json::json;

    /// No provider's idea of a token's name reaches the screen.
    ///
    /// A row is matched on the contract address, so a hostile token cannot be
    /// mistaken for USDT. But the *name* still used to be copied out of the
    /// reply on two of the three paths, which did two things: it let a server
    /// choose a string the user reads, and it showed one holding as USDT on
    /// the balance screen and USDT0 in its own history.
    #[test]
    fn the_token_label_never_comes_from_the_reply() {
        // A name in characters that are not the ones in USDT, plus the real
        // contract address, on every provider's shape.
        const HOSTILE: &str = "U\u{0405}DT";
        let usdt = crate::POLYGON.stable_address();
        let contract = usdt.to_string();
        let from = "0x1111111111111111111111111111111111111111";
        let to = "0x2222222222222222222222222222222222222222";

        let blockscout = crate::blockscout::parse_tokens(
            &json!({"items": [{
                "timestamp": "2026-06-26T22:43:10.000000Z",
                "transaction_hash": "0xa",
                "from": {"hash": from}, "to": {"hash": to},
                "total": {"decimals": "6", "value": "1"},
                "token": {"address_hash": contract, "decimals": "6", "symbol": HOSTILE},
            }]}),
            crate::POLYGON,
            usdt,
        );
        let etherscan = crate::etherscan::parse_tokens(
            &json!([{
            "hash": "0xa", "from": from, "to": to, "value": "1",
            "tokenSymbol": HOSTILE, "tokenDecimal": "6", "timeStamp": "1782513790",
            }]),
            crate::POLYGON,
        );
        let nodereal = parse(
            &json!({"transfers": [{
                "hash": "0xa", "from": from, "to": to, "value": "0x1",
                "category": "20", "contractAddress": contract,
                "blockTimeStamp": 1_782_513_790i64, "asset": HOSTILE,
            }]}),
            crate::POLYGON,
            usdt,
        );

        for (name, rows) in [
            ("blockscout", &blockscout),
            ("etherscan", &etherscan),
            ("nodereal", &nodereal),
        ] {
            assert_eq!(rows.len(), 1, "{name} parsed nothing");
            assert_eq!(
                rows[0].symbol,
                crate::POLYGON.stable_label,
                "{name} put a name the server chose on the screen"
            );
            assert_ne!(rows[0].symbol, HOSTILE);
            // And the precision is still the token's own, not the label's.
            assert_eq!(rows[0].decimals, 6, "{name}");
        }
    }
}
