//! What a reply means for the next attempt.
//!
//! The line that matters is between a node that is *busy* and a node that has
//! *decided*. A 429 or a 503 is the first kind and is worth asking again. A
//! JSON-RPC `error` object, TronGrid's HTTP-200 `Error` field, a 404 - those
//! are the second kind, they will be answered identically next time, and
//! asking again only spends quota that the next real request needed.
//!
//! Some nodes blur the line by reporting a rate limit *inside* a successful
//! reply, so a client can add its own reading of the body with a
//! [`Classifier`].

use std::time::Duration;

use serde_json::Value;

use crate::service::Reply;

/// How a reply is treated by the retry loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Hand it to the caller, whatever it says.
    Final,
    /// The server refused because of how often it is being asked. The endpoint
    /// or key cools down, the throttle widens, and the request is tried again
    /// - elsewhere if there is an elsewhere.
    RateLimited { retry_after: Option<Duration> },
    /// The server, or something in front of it, failed. Worth another attempt
    /// for a read, never for a submission.
    Unavailable,
}

/// A client's own reading of a reply, consulted before the default one.
/// `None` means "no opinion".
pub type Classifier = fn(&Reply) -> Option<Verdict>;

/// The reading every client gets: status codes, plus the 403 that TronGrid
/// sends when a key has gone over its limit.
pub fn by_status(reply: &Reply) -> Verdict {
    match reply.status {
        429 => Verdict::RateLimited {
            retry_after: reply.retry_after().or_else(|| hint_from_body(&reply.body)),
        },
        // "The key exceeds the frequency limit(15), and the query server is
        // suspended for 30s" - a rate limit wearing a 403. Any other 403 is a
        // real refusal and is final.
        403 if says_rate_limited(&reply.body) => Verdict::RateLimited {
            retry_after: reply.retry_after().or_else(|| hint_from_body(&reply.body)),
        },
        408 | 425 => Verdict::Unavailable,
        // 501 and 505 are statements about the request, not about load.
        501 | 505 => Verdict::Final,
        500..=599 => Verdict::Unavailable,
        _ => Verdict::Final,
    }
}

/// Whether a message is a node saying "too often".
///
/// Deliberately narrower than "contains *exceeded*": Solana's preflight says
/// "computational budget exceeded" and an EVM node says "exceeds block gas
/// limit", and both are decisions about the transaction.
pub fn says_rate_limited(text: &str) -> bool {
    let t = text.to_ascii_lowercase();
    [
        "rate limit",
        "rate-limit",
        "ratelimit",
        "too many requests",
        "request limit",
        "requests limit",
        "limit exceeded",
        "quota exceeded",
        "exceeded the quota",
        "exceeded the rate",
        "frequency limit",
        "capacity exceeded",
        "throttled",
    ]
    .iter()
    .any(|p| t.contains(p))
}

/// The JSON-RPC error codes that mean "too often" rather than "no".
///
/// -32005 is EIP-1474's "limit exceeded" and what Infura-style gateways send;
/// on Solana it is "node is unhealthy", which is no more a decision about the
/// request and is just as worth retrying. -32029 and 429 are what public
/// gateways use when they mirror the HTTP status into the body.
pub const RATE_LIMIT_CODES: [i64; 3] = [-32005, -32029, 429];

/// JSON-RPC bodies: a rate limit dressed as an `error` object, in a single
/// reply or anywhere in a batch. Any other `error` stays final, as before.
pub fn jsonrpc(reply: &Reply) -> Option<Verdict> {
    if !reply.is_success() {
        return None;
    }
    let v: Value = serde_json::from_str(&reply.body).ok()?;
    let limited = match &v {
        Value::Array(items) => items.iter().any(|i| error_is_rate_limit(i.get("error"))),
        one => error_is_rate_limit(one.get("error")),
    };
    limited.then_some(Verdict::RateLimited { retry_after: None })
}

fn error_is_rate_limit(err: Option<&Value>) -> bool {
    let Some(err) = err else { return false };
    if let Some(code) = err.get("code").and_then(Value::as_i64) {
        if RATE_LIMIT_CODES.contains(&code) {
            return true;
        }
    }
    err.get("message")
        .and_then(Value::as_str)
        .or_else(|| err.as_str())
        .is_some_and(says_rate_limited)
}

/// A wait the server wrote into its body instead of a header: TronGrid's
/// "suspended for 30s".
pub fn hint_from_body(body: &str) -> Option<Duration> {
    let lower = body.to_ascii_lowercase();
    let at = lower.find("suspended for ")? + "suspended for ".len();
    let digits: String = lower[at..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok().map(Duration::from_secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(status: u16, body: &str) -> Reply {
        Reply::new(status, body)
    }

    #[test]
    fn statuses() {
        assert!(matches!(
            by_status(&reply(429, "")),
            Verdict::RateLimited { .. }
        ));
        assert_eq!(by_status(&reply(503, "")), Verdict::Unavailable);
        assert_eq!(by_status(&reply(502, "")), Verdict::Unavailable);
        assert_eq!(by_status(&reply(501, "")), Verdict::Final);
        assert_eq!(by_status(&reply(200, "")), Verdict::Final);
        assert_eq!(by_status(&reply(400, "")), Verdict::Final);
        assert_eq!(by_status(&reply(404, "")), Verdict::Final);
        // A 403 is a refusal unless it says it is a rate limit.
        assert_eq!(by_status(&reply(403, "forbidden")), Verdict::Final);
    }

    /// TronGrid's own wording, from a key that went over its limit.
    #[test]
    fn trongrid_403_is_a_rate_limit_with_its_own_wait() {
        let body = r#"{"Error":"The key exceeds the frequency limit(15), and the query server is suspended for 30s"}"#;
        assert_eq!(
            by_status(&reply(403, body)),
            Verdict::RateLimited {
                retry_after: Some(Duration::from_secs(30))
            }
        );
        // A header, when there is one, outranks the body.
        let r = reply(429, body).with_retry_after(Duration::from_secs(2));
        assert_eq!(
            by_status(&r),
            Verdict::RateLimited {
                retry_after: Some(Duration::from_secs(2))
            }
        );
    }

    #[test]
    fn jsonrpc_rate_limits_are_recognised() {
        for body in [
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"limit exceeded"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32029,"message":"x"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":429,"message":"x"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"Too Many Requests"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32603,"message":"daily request limit reached"}}"#,
            r#"[{"id":0,"result":1},{"id":1,"error":{"code":-32000,"message":"rate limited"}}]"#,
        ] {
            assert!(
                matches!(
                    jsonrpc(&reply(200, body)),
                    Some(Verdict::RateLimited { .. })
                ),
                "{body}"
            );
        }
    }

    /// The node's decisions stay decisions.
    #[test]
    fn jsonrpc_business_errors_stay_final() {
        for body in [
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"nonce too low"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":3,"message":"execution reverted: transfer amount exceeds balance"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"exceeds block gas limit"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32002,"message":"Transaction simulation failed: Computational budget exceeded"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"result":"0x1"}"#,
            r#"{"Error":"class org.tron.core.exception.ContractValidateException"}"#,
            "not json",
        ] {
            assert_eq!(jsonrpc(&reply(200, body)), None, "{body}");
        }
        // Only successful replies are read; the status speaks for the rest.
        assert_eq!(jsonrpc(&reply(500, r#"{"error":{"code":429}}"#)), None);
    }

    #[test]
    fn body_hints() {
        assert_eq!(
            hint_from_body("the query server is suspended for 30s"),
            Some(Duration::from_secs(30))
        );
        assert_eq!(
            hint_from_body("Suspended For 5 seconds"),
            Some(Duration::from_secs(5))
        );
        assert_eq!(hint_from_body("suspended for a while"), None);
        assert_eq!(hint_from_body(""), None);
    }
}
