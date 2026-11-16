//! The envelope Sui's GraphQL RPC answers in, and how its failures read.
//!
//! GraphQL reports almost every failure inside an HTTP 200: a malformed
//! transaction, an exceeded query limit and an unknown field all arrive as an
//! `errors` array beside a `null` (or partial) `data`. The status code says
//! nothing, so the body is what decides.
//!
//! Checked against `graphql.mainnet.sui.io` (server 1.83): a bad argument is
//! `BAD_USER_INPUT`, a query over the service's limits is
//! `RESOURCE_EXHAUSTED`, and neither is worth asking again. The server itself
//! defines no rate-limit code - the public endpoint throttles at its gateway
//! with a plain 429 - so a rate limit inside the body is recognised by its
//! wording, for the configurable endpoints run by others.

use neko_net::{Reply, Verdict};
use serde_json::Value;

use crate::error::SuiError;

/// Codes a GraphQL gateway uses for "too often" when it reports it in the
/// body rather than the status.
const RATE_LIMIT_CODES: [&str; 3] = ["RATE_LIMITED", "TOO_MANY_REQUESTS", "THROTTLED"];

/// A rate limit dressed as a GraphQL error. Every other error stays final:
/// the same query would be answered the same way.
pub fn classify(reply: &Reply) -> Option<Verdict> {
    if !reply.is_success() {
        return None;
    }
    let v: Value = serde_json::from_str(&reply.body).ok()?;
    let limited = v.get("errors")?.as_array()?.iter().any(error_is_rate_limit);
    limited.then_some(Verdict::RateLimited { retry_after: None })
}

fn error_is_rate_limit(e: &Value) -> bool {
    let code = e
        .get("extensions")
        .and_then(|x| x.get("code"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    RATE_LIMIT_CODES.contains(&code)
        || e.get("message")
            .and_then(Value::as_str)
            .is_some_and(neko_net::classify::says_rate_limited)
}

/// The `data` of a reply, or the first error it carries.
///
/// A reply with both is an error: partial data from a query that failed
/// halfway is not something to show as a balance or a history.
pub fn data(text: &str) -> Result<Value, SuiError> {
    let v: Value = serde_json::from_str(text).map_err(|_| SuiError::BadReply(cut(text)))?;
    if let Some(first) = v
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
    {
        let msg = first
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("the node refused the request");
        return Err(SuiError::Rpc(cut(msg)));
    }
    match v.get("data") {
        Some(d) if !d.is_null() => Ok(d.clone()),
        // A node configured before the move to GraphQL answers in JSON-RPC's
        // envelope, and saying so is more use than quoting it.
        _ if v.get("jsonrpc").is_some() => Err(SuiError::Rpc(format!(
            "this is a JSON-RPC node, and Sui is now read over GraphQL: {}",
            cut(v
                .pointer("/error/message")
                .and_then(Value::as_str)
                .unwrap_or_default())
        ))),
        _ => Err(SuiError::BadReply(cut(text))),
    }
}

/// A number the schema may send as a string (`BigInt`) or a number
/// (`UInt53`) - both appear in the same reply.
pub fn int(v: &Value) -> Option<i128> {
    match v {
        Value::String(s) => s.parse().ok(),
        Value::Number(n) => n.as_i64().map(i128::from),
        _ => None,
    }
}

pub fn cut(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= 300 {
        return t.to_string();
    }
    t.chars().take(300).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(body: &str) -> Reply {
        Reply::new(200, body)
    }

    #[test]
    fn rate_limits_in_the_body_are_recognised() {
        for body in [
            r#"{"data":null,"errors":[{"message":"x","extensions":{"code":"RATE_LIMITED"}}]}"#,
            r#"{"data":null,"errors":[{"message":"x","extensions":{"code":"TOO_MANY_REQUESTS"}}]}"#,
            r#"{"data":null,"errors":[{"message":"Too Many Requests"}]}"#,
            r#"{"errors":[{"message":"daily request limit reached"}]}"#,
        ] {
            assert!(
                matches!(classify(&reply(body)), Some(Verdict::RateLimited { .. })),
                "{body}"
            );
        }
    }

    /// The node's decisions stay decisions, including the one that sounds
    /// like a limit: `RESOURCE_EXHAUSTED` is about the shape of the query,
    /// and asking again with the same shape is refused again.
    #[test]
    fn other_errors_stay_final() {
        for body in [
            r#"{"data":null,"errors":[{"message":"Exceeded the maximum number (21) of queries that require dedicated access to a backing store in a single request.","extensions":{"code":"RESOURCE_EXHAUSTED"}}]}"#,
            r#"{"data":null,"errors":[{"message":"Invalid BCS-encoded TransactionData: unexpected end of input","extensions":{"code":"BAD_USER_INPUT"}}]}"#,
            r#"{"data":{"epoch":{"referenceGasPrice":"100"}}}"#,
            "not json",
        ] {
            assert_eq!(classify(&reply(body)), None, "{body}");
        }
        // Only successful replies are read; the status speaks for the rest.
        assert_eq!(
            classify(&Reply::new(
                503,
                r#"{"errors":[{"message":"rate limited"}]}"#
            )),
            None
        );
    }

    #[test]
    fn an_error_beside_partial_data_is_an_error() {
        let body = r#"{"data":{"objects":{"nodes":[]}},"errors":[{"message":"Exceeded the maximum number (21)","extensions":{"code":"RESOURCE_EXHAUSTED"}}]}"#;
        let e = data(body).unwrap_err().to_string();
        assert!(e.contains("Exceeded the maximum number"), "{e}");
        assert!(matches!(data("{}"), Err(SuiError::BadReply(_))));
        assert!(matches!(data("<html>"), Err(SuiError::BadReply(_))));
        assert_eq!(data(r#"{"data":{"a":1}}"#).unwrap()["a"], 1);
    }

    /// What the old default, PublicNode's JSON-RPC, answers a GraphQL query -
    /// the reply a stale configured URL will produce. The gRPC address it
    /// names is cut, so no URL outside the allowlist appears in the source.
    #[test]
    fn a_json_rpc_node_is_named_as_one() {
        let body = r#"{"jsonrpc":"2.0","error":{"code":-32000,"message":"Sui JSON-RPC endpoint has been deprecated. Please use the gRPC endpoint instead"},"id":null}"#;
        let e = data(body).unwrap_err().to_string();
        assert!(
            e.contains("JSON-RPC node") && e.contains("deprecated"),
            "{e}"
        );
    }

    #[test]
    fn numbers_arrive_both_ways() {
        assert_eq!(int(&Value::from("243001381957")), Some(243_001_381_957));
        assert_eq!(int(&Value::from("-5456568")), Some(-5_456_568));
        assert_eq!(int(&Value::from(1_076_301_771u64)), Some(1_076_301_771));
        assert_eq!(int(&Value::Null), None);
    }
}
