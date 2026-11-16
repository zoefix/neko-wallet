//! The retry loop against a node that misbehaves on cue.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::*;
use crate::testserver::{refused, serve, Answer};

/// Real waits, kept short.
fn quick() -> Retry {
    Retry {
        attempts: 4,
        base_delay: Duration::from_millis(10),
        max_delay: Duration::from_millis(40),
        max_retry_after: Duration::from_secs(2),
        rate_limit_cooldown: Duration::from_secs(30),
        failure_cooldown: Duration::from_secs(30),
        spill_after: Duration::from_secs(1),
    }
}

fn read() -> Request {
    Request::post_json("/wallet/getaccount", &json!({"address": "41"}))
}

fn submit() -> Request {
    Request::post_json("/wallet/broadcasthex", &json!({"transaction": "0a02"})).submit()
}

#[tokio::test]
async fn a_busy_node_is_asked_again_until_it_answers() {
    let s = serve(|_, n| match n {
        0 => Answer::status(503, "busy"),
        1 => Answer::status(429, "slow down"),
        _ => Answer::ok(r#"{"balance":1}"#),
    })
    .await;
    let r = Service::new(&s.url)
        .retry(quick())
        .send(read())
        .await
        .unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(s.hits(), 3);
    // Each attempt is the same request, path and body.
    for seen in s.seen() {
        assert_eq!(seen.path, "/wallet/getaccount");
        assert_eq!(seen.body, r#"{"address":"41"}"#);
    }
}

/// A decision is handed straight back. Asking again would be answered the
/// same way and spend quota a real request needed.
#[tokio::test]
async fn a_decision_is_never_asked_again() {
    for (status, body) in [
        (
            200,
            r#"{"Error":"class org.tron.core.exception.ContractValidateException"}"#,
        ),
        (
            200,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32000,"message":"nonce too low"}}"#,
        ),
        (400, "bad request"),
        (403, "forbidden"),
        (404, "not found"),
    ] {
        let s = serve(move |_, _| Answer::status(status, body)).await;
        let r = Service::new(&s.url)
            .classify(classify::jsonrpc)
            .retry(quick())
            .send(read())
            .await
            .unwrap();
        assert_eq!((r.status, r.body.as_str()), (status, body));
        assert!(!r.rate_limited());
        assert_eq!(s.hits(), 1, "{status} {body} was retried");
    }
}

#[tokio::test]
async fn retries_run_out_and_the_last_reply_is_returned() {
    let s = serve(|_, _| Answer::status(503, "down")).await;
    let r = Service::new(&s.url)
        .retry(quick())
        .send(read())
        .await
        .unwrap();
    assert_eq!(r.status, 503);
    assert_eq!(s.hits(), 4);

    let s = serve(|_, _| Answer::status(429, "slow down")).await;
    let r = Service::new(&s.url)
        .retry(quick())
        .send(read())
        .await
        .unwrap();
    assert!(r.rate_limited());
    assert_eq!(s.hits(), 4);
}

#[tokio::test]
async fn retry_after_is_waited_out() {
    let s = serve(|_, n| match n {
        0 => Answer::status(429, "").with_header("retry-after", "1"),
        _ => Answer::ok("{}"),
    })
    .await;
    let t0 = Instant::now();
    let r = Service::new(&s.url)
        .retry(quick())
        .send(read())
        .await
        .unwrap();
    assert_eq!(r.status, 200);
    let seen = s.seen();
    let gap = seen[1].at - seen[0].at;
    assert!(
        gap >= Duration::from_millis(950),
        "asked again after {gap:?}"
    );
    assert!(Instant::now() - t0 < Duration::from_secs(3));
}

/// TronGrid's suspension notice names its own wait, in the body of a 403.
#[tokio::test]
async fn a_wait_named_in_the_body_is_waited_out() {
    let s = serve(|_, n| match n {
        0 => Answer::status(
            403,
            r#"{"Error":"The key exceeds the frequency limit(15), and the query server is suspended for 1s"}"#,
        ),
        _ => Answer::ok("{}"),
    })
    .await;
    let r = Service::new(&s.url)
        .retry(quick())
        .send(read())
        .await
        .unwrap();
    assert_eq!(r.status, 200);
    let seen = s.seen();
    assert!(seen[1].at - seen[0].at >= Duration::from_millis(950));
}

/// Past the cap there is no point waiting: the answer is "rate limited", now.
#[tokio::test]
async fn a_wait_past_the_cap_is_not_waited() {
    let s = serve(|_, _| Answer::status(429, "").with_header("retry-after", "120")).await;
    let t0 = Instant::now();
    let r = Service::new(&s.url)
        .retry(quick())
        .send(read())
        .await
        .unwrap();
    assert!(r.rate_limited());
    assert_eq!(s.hits(), 1);
    assert!(Instant::now() - t0 < Duration::from_secs(1));
}

/// Default first, fallback when it refuses, and the default passed over until
/// its cooldown ends.
#[tokio::test]
async fn failover_goes_in_order_and_remembers_who_refused() {
    let primary = serve(|_, _| Answer::status(429, "slow down")).await;
    let fallback = serve(|_, _| Answer::ok("{}")).await;
    let retry = Retry {
        rate_limit_cooldown: Duration::from_millis(400),
        ..quick()
    };
    let svc = || Service::for_chain(None, &primary.url, &[&fallback.url]).retry(retry);

    let t0 = Instant::now();
    assert_eq!(svc().send(read()).await.unwrap().status, 200);
    assert_eq!((primary.hits(), fallback.hits()), (1, 1));
    // Switching costs no backoff.
    assert!(Instant::now() - t0 < Duration::from_millis(300));

    // A new client - as the wallet builds for every request - still knows.
    assert_eq!(svc().send(read()).await.unwrap().status, 200);
    assert_eq!((primary.hits(), fallback.hits()), (1, 2));

    tokio::time::sleep(Duration::from_millis(450)).await;
    assert_eq!(svc().send(read()).await.unwrap().status, 200);
    assert_eq!(
        (primary.hits(), fallback.hits()),
        (2, 3),
        "the default is asked again once it has cooled down"
    );
}

#[tokio::test]
async fn an_unreachable_default_fails_over() {
    let dead = refused().await;
    let fallback = serve(|_, _| Answer::ok("{}")).await;
    let r = Service::for_chain(None, &dead, &[&fallback.url])
        .retry(quick())
        .send(read())
        .await
        .unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(fallback.hits(), 1);
}

/// A configured node is the only node, however badly it is doing.
#[tokio::test]
async fn a_configured_node_is_never_bypassed() {
    let mine = serve(|_, _| Answer::status(503, "down")).await;
    let default = serve(|_, _| Answer::ok("{}")).await;
    let fallback = serve(|_, _| Answer::ok("{}")).await;
    let svc = Service::for_chain(Some(&mine.url), &default.url, &[&fallback.url]).retry(quick());
    assert_eq!(svc.primary(), mine.url);
    assert_eq!(svc.send(read()).await.unwrap().status, 503);
    assert_eq!(mine.hits(), 4);
    assert_eq!((default.hits(), fallback.hits()), (0, 0));
}

#[tokio::test]
async fn keys_rotate_and_a_limited_key_sits_out() {
    let s = serve(|req, _| match req.header("x-api-key") {
        Some("rot-bad") => Answer::status(429, "slow down"),
        Some(_) => Answer::ok("{}"),
        None => Answer::status(401, "no key"),
    })
    .await;
    let setting = "rot-good-1, rot-bad\nrot-good-2";
    for _ in 0..9 {
        let r = Service::new(&s.url)
            .with_keys("x-api-key", Some(setting))
            .retry(quick())
            .send(read())
            .await
            .unwrap();
        assert_eq!(r.status, 200);
    }
    let used: Vec<String> = s
        .seen()
        .iter()
        .map(|r| r.header("x-api-key").unwrap().to_string())
        .collect();
    let count = |k: &str| used.iter().filter(|u| *u == k).count();
    assert_eq!(
        count("rot-bad"),
        1,
        "a suspended key was asked again: {used:?}"
    );
    assert!(
        count("rot-good-1") >= 4 && count("rot-good-2") >= 4,
        "{used:?}"
    );
}

/// The keys belong to the default host. A built-in fallback is somebody else
/// entirely and never sees them, not even when the default is refusing.
#[tokio::test]
async fn keys_never_reach_a_fallback() {
    let primary = serve(|_, _| Answer::status(429, "slow down")).await;
    let fallback = serve(|_, _| Answer::ok("{}")).await;
    let r = Service::for_chain(None, &primary.url, &[&fallback.url])
        .with_keys("tron-pro-api-key", Some("fb-key-1,fb-key-2"))
        .retry(quick())
        .send(read())
        .await
        .unwrap();
    assert_eq!(r.status, 200);
    assert!(primary
        .seen()
        .iter()
        .all(|r| r.header("tron-pro-api-key").is_some()));
    assert!(fallback.hits() >= 1);
    for seen in fallback.seen() {
        assert_eq!(seen.header("tron-pro-api-key"), None);
        assert!(!format!("{seen:?}").contains("fb-key"), "{seen:?}");
    }

    // And a configured node is the user's own, so it does get them.
    let mine = serve(|_, _| Answer::ok("{}")).await;
    Service::for_chain(Some(&mine.url), &primary.url, &[&fallback.url])
        .with_keys("tron-pro-api-key", Some("fb-key-1"))
        .send(read())
        .await
        .unwrap();
    assert_eq!(mine.seen()[0].header("tron-pro-api-key"), Some("fb-key-1"));
}

/// The broadcast that timed out may well have been broadcast. It is not sent
/// again - not to the same node and not to a fallback.
#[tokio::test]
async fn a_submission_is_not_resent_after_a_timeout() {
    let slow = serve(|_, _| Answer::Hang(Duration::from_secs(3))).await;
    let fallback = serve(|_, _| Answer::ok(r#"{"result":true}"#)).await;
    let svc = Service::for_chain(None, &slow.url, &[&fallback.url]).retry(quick());

    let err = svc
        .send(submit().timeout(Duration::from_millis(200)))
        .await
        .unwrap_err();
    assert_eq!(err, NetError::Timeout);
    assert_eq!(slow.hits(), 1);
    assert_eq!(fallback.hits(), 0, "the transaction went out twice");
}

/// The same failure on a read is retried, which is the contrast that makes
/// the test above mean something.
#[tokio::test]
async fn a_read_is_retried_after_a_timeout() {
    let slow = serve(|_, _| Answer::Hang(Duration::from_secs(3))).await;
    let r = Service::new(&slow.url)
        .retry(quick())
        .send(read().timeout(Duration::from_millis(100)))
        .await;
    assert_eq!(r.unwrap_err(), NetError::Timeout);
    assert_eq!(slow.hits(), 4);
}

#[tokio::test]
async fn a_submission_is_not_resent_after_a_server_error() {
    let s = serve(|_, _| Answer::status(502, "bad gateway")).await;
    let r = Service::new(&s.url)
        .retry(quick())
        .send(submit())
        .await
        .unwrap();
    assert_eq!(r.status, 502);
    assert_eq!(s.hits(), 1);
}

/// Two cases where it certainly never arrived: nobody answered the
/// connection, or the door said 429.
#[tokio::test]
async fn a_submission_is_retried_when_it_never_arrived() {
    let dead = refused().await;
    let fallback = serve(|_, _| Answer::ok(r#"{"result":true}"#)).await;
    let r = Service::for_chain(None, &dead, &[&fallback.url])
        .retry(quick())
        .send(submit())
        .await
        .unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(fallback.hits(), 1);

    let s = serve(|_, n| match n {
        0 => Answer::status(429, "slow down"),
        _ => Answer::ok(r#"{"result":true}"#),
    })
    .await;
    let r = Service::new(&s.url)
        .retry(quick())
        .send(submit())
        .await
        .unwrap();
    assert_eq!(r.status, 200);
    assert_eq!(s.hits(), 2);
}

/// A rate limit inside a 200 came from the application, which may have acted
/// on the transaction before refusing. Reads retry it; submissions do not.
#[tokio::test]
async fn a_rate_limit_inside_a_reply_is_retried_only_for_reads() {
    let limited = r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32005,"message":"limit exceeded"}}"#;
    let s = serve(move |_, n| match n {
        0 => Answer::ok(limited),
        _ => Answer::ok(r#"{"jsonrpc":"2.0","id":1,"result":"0x1"}"#),
    })
    .await;
    let svc = Service::new(&s.url)
        .classify(classify::jsonrpc)
        .retry(quick());
    let r = svc.send(read()).await.unwrap();
    assert!(r.body.contains("0x1"));
    assert_eq!(s.hits(), 2);

    let s = serve(move |_, _| Answer::ok(limited)).await;
    let r = Service::new(&s.url)
        .classify(classify::jsonrpc)
        .retry(quick())
        .send(submit())
        .await
        .unwrap();
    assert!(r.rate_limited());
    assert_eq!(s.hits(), 1);
}

/// A default with a queue behind it hands the overflow to a fallback before
/// anybody has been refused - and a service with nowhere else to go waits
/// its turn instead.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_backed_up_default_spills_over_to_a_fallback() {
    let primary = serve(|_, _| Answer::ok("{}")).await;
    let fallback = serve(|_, _| Answer::ok("{}")).await;
    let host = primary.url.trim_start_matches("http://").to_string();
    let bucket = throttle::bucket(&BucketId::new(&host, None));

    // Every place on the default taken, and five more callers queued behind.
    let mut held = Vec::new();
    for _ in 0..bucket.current().in_flight {
        held.push(bucket.acquire().await);
    }
    let queued: Vec<_> = (0..5)
        .map(|_| {
            let url = primary.url.clone();
            tokio::spawn(async move { Service::new(&url).send(read()).await.unwrap().status })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(bucket.backlog(Instant::now()) >= Duration::from_millis(100));

    let retry = Retry {
        spill_after: Duration::from_millis(50),
        ..quick()
    };
    for _ in 0..3 {
        let r = Service::for_chain(None, &primary.url, &[&fallback.url])
            .retry(retry)
            .send(read())
            .await
            .unwrap();
        assert_eq!(r.status, 200);
    }
    assert_eq!((primary.hits(), fallback.hits()), (0, 3));

    drop(held);
    for q in queued {
        assert_eq!(q.await.unwrap(), 200);
    }
    assert_eq!(primary.hits(), 5);
}

/// A burst reaches the node paced, not all at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_is_paced() {
    let s = serve(|_, _| Answer::ok("{}")).await;
    // A loopback host gets the generous defaults: eight in flight, starts
    // twenty milliseconds apart.
    let host = s.url.trim_start_matches("http://").to_string();
    assert_eq!(
        throttle::base_limits(&host, false),
        throttle::Limits::new(8, 20)
    );
    let tasks: Vec<_> = (0..10)
        .map(|_| {
            let url = s.url.clone();
            tokio::spawn(async move { Service::new(&url).send(read()).await.unwrap().status })
        })
        .collect();
    for t in tasks {
        assert_eq!(t.await.unwrap(), 200);
    }
    let mut at: Vec<Instant> = s.seen().iter().map(|r| r.at).collect();
    at.sort();
    let span = at[9] - at[0];
    assert!(
        span >= Duration::from_millis(160),
        "ten requests within {span:?}"
    );
}

#[tokio::test]
async fn no_url_ever_appears_in_an_error() {
    let dead = refused().await;
    let svc = Service::new(&format!("{dead}/v1/secret-key-in-path?apikey=secret")).retry(quick());
    let err = svc.send(Request::get("")).await.unwrap_err();
    assert!(matches!(err, NetError::Connect(_)), "{err:?}");
    let text = err.to_string();
    assert!(
        !text.contains("secret") && !text.contains("127.0.0.1"),
        "{text}"
    );
}

#[tokio::test]
async fn a_dropped_caller_gives_its_place_back() {
    let s = serve(|_, _| Answer::Hang(Duration::from_secs(5))).await;
    let host = s.url.trim_start_matches("http://").to_string();
    let bucket = throttle::bucket(&BucketId::new(&host, None));
    let before = bucket.current().in_flight;
    let done = Arc::new(AtomicBool::new(false));
    let d = Arc::clone(&done);
    let url = s.url.clone();
    let task = tokio::spawn(async move {
        let _ = Service::new(&url).send(read()).await;
        d.store(true, Ordering::SeqCst);
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    task.abort();
    let _ = task.await;
    assert!(!done.load(Ordering::SeqCst));
    // All places free again: as many as the limit can be taken at once.
    let permits = take_all(&bucket, before).await;
    assert_eq!(permits.len(), before);
}

async fn take_all(bucket: &Arc<Bucket>, n: usize) -> Vec<crate::throttle::Permit> {
    let mut out = Vec::new();
    for _ in 0..n {
        let p = tokio::time::timeout(Duration::from_secs(2), bucket.acquire())
            .await
            .expect("a place was never given back");
        out.push(p);
    }
    out
}
