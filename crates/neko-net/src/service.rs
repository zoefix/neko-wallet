//! One remote service: where it lives, which keys it takes, and the loop that
//! asks it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::classify::{self, Classifier, Verdict};
use crate::keys::{self, KeyRing};
use crate::retry::{self, Retry};
use crate::throttle::{self, Bucket, BucketId};

/// Whether a request may be repeated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Policy {
    /// A question. Asking twice costs nothing.
    Read,
    /// A transaction going out. Sent again only when it certainly never
    /// arrived: the connection was never made, or the node answered 429.
    ///
    /// A timeout, a reset or a 502 after the request went out are all
    /// consistent with the node having accepted it. Sending it again then is
    /// harmless to the chain, which rejects the duplicate, and harmful to the
    /// person, who is shown that rejection - "already known", "nonce too
    /// low", "sequence number too old" - for a transfer that went through.
    Submit,
}

#[derive(Clone)]
enum Body {
    Empty,
    Json(serde_json::Value),
    Text(String),
    Bytes(Vec<u8>, &'static str),
}

/// What to ask. The path is appended to whichever endpoint answers.
///
/// No `Debug`: a path can carry a key (Etherscan takes it in the query), and
/// a debug print is exactly how one ends up in a log.
#[derive(Clone)]
pub struct Request {
    post: bool,
    path: String,
    body: Body,
    headers: Vec<(&'static str, String)>,
    timeout: Option<Duration>,
    policy: Policy,
}

impl Request {
    pub fn get(path: impl Into<String>) -> Self {
        Request {
            post: false,
            path: path.into(),
            body: Body::Empty,
            headers: Vec::new(),
            timeout: None,
            policy: Policy::Read,
        }
    }

    pub fn post_json(path: impl Into<String>, body: &serde_json::Value) -> Self {
        Request {
            post: true,
            body: Body::Json(body.clone()),
            ..Request::get(path)
        }
    }

    /// A plain body with no content type, which is what Esplora's `/tx` wants.
    pub fn post_text(path: impl Into<String>, body: String) -> Self {
        Request {
            post: true,
            body: Body::Text(body),
            ..Request::get(path)
        }
    }

    pub fn post_bytes(path: impl Into<String>, body: Vec<u8>, content_type: &'static str) -> Self {
        Request {
            post: true,
            body: Body::Bytes(body, content_type),
            ..Request::get(path)
        }
    }

    pub fn header(mut self, name: &'static str, value: impl Into<String>) -> Self {
        self.headers.push((name, value.into()));
        self
    }

    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = Some(d);
        self
    }

    /// Mark this as a transaction submission. See [`Policy::Submit`].
    pub fn submit(mut self) -> Self {
        self.policy = Policy::Submit;
        self
    }
}

/// A reply, of any status. What it means is the caller's business; this
/// crate only decides whether to ask again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
    retry_after: Option<Duration>,
    rate_limited: bool,
}

impl Reply {
    pub fn new(status: u16, body: impl Into<String>) -> Self {
        Reply {
            status,
            body: body.into(),
            retry_after: None,
            rate_limited: false,
        }
    }

    pub fn with_retry_after(mut self, d: Duration) -> Self {
        self.retry_after = Some(d);
        self
    }

    pub fn is_success(&self) -> bool {
        (200..300).contains(&self.status)
    }

    /// The server's `Retry-After`, if it sent one.
    pub fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }

    /// Whether this is a rate limit - one that outlasted every retry, when it
    /// comes back from [`Service::send`] - so a caller can say "rate limited"
    /// rather than quote a status code.
    pub fn rate_limited(&self) -> bool {
        self.rate_limited || matches!(classify::by_status(self), Verdict::RateLimited { .. })
    }
}

/// No reply at all.
///
/// The text never contains a URL. Some services carry their key in the path
/// or the query, and an error message is the kind of thing that ends up on a
/// screen or in a bug report.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NetError {
    #[error("could not connect: {0}")]
    Connect(String),
    #[error("timed out")]
    Timeout,
    #[error("connection failed: {0}")]
    Transport(String),
    #[error("the request could not be built: {0}")]
    Request(String),
}

impl NetError {
    fn from_reqwest(e: reqwest::Error) -> Self {
        let e = e.without_url();
        let detail = root_cause(&e);
        if e.is_timeout() {
            NetError::Timeout
        } else if e.is_connect() {
            NetError::Connect(detail)
        } else if e.is_builder() {
            NetError::Request(detail)
        } else {
            NetError::Transport(detail)
        }
    }

    /// Whether the request certainly never reached the server.
    fn never_sent(&self) -> bool {
        matches!(self, NetError::Connect(_) | NetError::Request(_))
    }
}

fn root_cause(e: &(dyn std::error::Error + 'static)) -> String {
    let mut last = e;
    while let Some(next) = last.source() {
        last = next;
    }
    last.to_string()
}

#[derive(Clone)]
struct Endpoint {
    base: String,
    host: String,
    /// Whether this endpoint may be sent the service's keys. Only the one the
    /// user configured, or the built-in default that the keys are *for*.
    trusted: bool,
}

impl Endpoint {
    fn new(base: &str, trusted: bool) -> Self {
        let base = base.trim().trim_end_matches('/').to_string();
        let host = reqwest::Url::parse(&base)
            .ok()
            .and_then(|u| {
                let host = u.host_str()?.to_string();
                Some(match u.port() {
                    Some(p) => format!("{host}:{p}"),
                    None => host,
                })
            })
            .unwrap_or_else(|| base.clone());
        Endpoint {
            base,
            host,
            trusted,
        }
    }
}

struct Credential {
    header: &'static str,
    ring: KeyRing,
}

/// One remote service, as reached by one client.
///
/// Cheap to build: everything that has to outlast it - throttles, cooldowns,
/// where key rotation stopped - is process-wide.
pub struct Service {
    endpoints: Vec<Endpoint>,
    credential: Option<Credential>,
    /// A key carried in the URL itself, as NodeReal and Etherscan take it.
    /// Only its fingerprint is kept, to give it a throttle of its own.
    url_key: Option<u64>,
    classifier: Option<Classifier>,
    timeout: Option<Duration>,
    retry: Retry,
}

impl std::fmt::Debug for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The host only: NodeReal's key is part of its path.
        f.debug_struct("Service")
            .field("host", &self.endpoints[0].host)
            .field("endpoints", &self.endpoints.len())
            .finish_non_exhaustive()
    }
}

struct Route {
    endpoint: usize,
    key: Option<usize>,
    bucket: Arc<Bucket>,
}

impl Service {
    /// One endpoint and nothing else.
    pub fn new(base: &str) -> Self {
        Service {
            endpoints: vec![Endpoint::new(base, true)],
            credential: None,
            url_key: None,
            classifier: None,
            timeout: None,
            retry: Retry::default(),
        }
    }

    /// A chain node, configured or not.
    ///
    /// When the user configured one, it is the only server ever contacted:
    /// the README promises the wallet talks to the nodes you choose, and a
    /// silent fallback would break that promise exactly when your node is
    /// down. When nobody did, the built-in default is asked first and the
    /// built-in fallbacks after it, each passed over while it cools down.
    pub fn for_chain(configured: Option<&str>, default: &str, fallbacks: &[&str]) -> Self {
        match configured.map(str::trim).filter(|u| !u.is_empty()) {
            Some(url) => Service::new(url),
            None => {
                let mut s = Service::new(default);
                s.endpoints
                    .extend(fallbacks.iter().map(|f| Endpoint::new(f, false)));
                s
            }
        }
    }

    /// Send `header` with one of `keys` - a setting that may hold several -
    /// to the endpoints trusted with it, and to nothing else.
    pub fn with_keys(mut self, header: &'static str, keys: Option<&str>) -> Self {
        let ring = KeyRing::parse(keys);
        self.credential = (!ring.is_empty()).then_some(Credential { header, ring });
        self
    }

    /// For a key the URL already carries: throttle it on its own.
    pub fn keyed_by(mut self, key: &str) -> Self {
        self.url_key = Some(keys::fingerprint(key));
        self
    }

    pub fn classify(mut self, c: Classifier) -> Self {
        self.classifier = Some(c);
        self
    }

    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = Some(d);
        self
    }

    pub fn retry(mut self, r: Retry) -> Self {
        self.retry = r;
        self
    }

    /// The endpoint asked first: the configured one, or the default.
    pub fn primary(&self) -> &str {
        &self.endpoints[0].base
    }

    fn verdict(&self, reply: &Reply) -> Verdict {
        self.classifier
            .and_then(|c| c(reply))
            .unwrap_or_else(|| classify::by_status(reply))
    }

    /// The first endpoint that is neither cooling down nor backed up, with a
    /// key if it takes one. Failing that the least busy; when every one is
    /// cooling, whichever comes back soonest.
    fn route(&self) -> Route {
        let now = Instant::now();
        let spill = self.retry.spill_after;
        let mut least_busy: Option<(Duration, Route)> = None;
        let mut soonest: Option<(Instant, Route)> = None;
        for (i, ep) in self.endpoints.iter().enumerate() {
            let keyed = self
                .credential
                .as_ref()
                .filter(|_| ep.trusted)
                .and_then(|c| c.ring.pick(&ep.host, now, spill));
            let route = match keyed {
                Some((k, bucket)) => Route {
                    endpoint: i,
                    key: Some(k),
                    bucket,
                },
                None => Route {
                    endpoint: i,
                    key: None,
                    bucket: throttle::bucket(&BucketId::new(&ep.host, self.url_key)),
                },
            };
            if let Some(until) = route.bucket.cooling_until(now) {
                if soonest.as_ref().is_none_or(|(t, _)| until < *t) {
                    soonest = Some((until, route));
                }
                continue;
            }
            let backlog = route.bucket.backlog(now);
            if backlog <= spill {
                return route;
            }
            if least_busy.as_ref().is_none_or(|(d, _)| backlog < *d) {
                least_busy = Some((backlog, route));
            }
        }
        least_busy
            .map(|(_, r)| r)
            .or(soonest.map(|(_, r)| r))
            .expect("a service has at least one endpoint")
    }

    async fn send_once(&self, route: &Route, req: &Request) -> Result<Reply, NetError> {
        let ep = &self.endpoints[route.endpoint];
        let url = format!("{}{}", ep.base, req.path);
        let http = crate::http::client();
        let mut rb = if req.post {
            http.post(url)
        } else {
            http.get(url)
        };
        rb = match &req.body {
            Body::Empty => rb,
            Body::Json(v) => rb.json(v),
            Body::Text(s) => rb.body(s.clone()),
            Body::Bytes(b, ct) => rb.header("content-type", *ct).body(b.clone()),
        };
        for (name, value) in &req.headers {
            rb = rb.header(*name, value);
        }
        if let (Some(c), Some(k)) = (&self.credential, route.key) {
            rb = rb.header(c.header, c.ring.secret(k));
        }
        if let Some(t) = req.timeout.or(self.timeout) {
            rb = rb.timeout(t);
        }
        let resp = rb.send().await.map_err(NetError::from_reqwest)?;
        let status = resp.status().as_u16();
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(retry::parse_retry_after);
        let body = resp.text().await.map_err(NetError::from_reqwest)?;
        Ok(Reply {
            status,
            body,
            retry_after,
            rate_limited: false,
        })
    }

    /// Ask, retrying what is worth retrying.
    ///
    /// Returns the last reply whatever its status once retries run out, so a
    /// caller's own reading of a 5xx or a 429 still applies; `Err` only when
    /// no reply came at all.
    pub async fn send(&self, req: Request) -> Result<Reply, NetError> {
        // Moving to another endpoint or key costs no wait, so each one gets
        // an attempt on top of the retries.
        let keys = self.credential.as_ref().map_or(0, |c| c.ring.len());
        let attempts = self.retry.attempts.max(1)
            + (self.endpoints.len() - 1) as u32
            + keys.saturating_sub(1) as u32;
        let alternatives = self.endpoints.len() > 1 || keys > 1;
        let mut backoffs = 0u32;
        let mut wait = Wait::Nothing;
        let mut previous: Option<BucketId> = None;
        let mut last = Err(NetError::Transport("no attempt was made".into()));

        let mut attempt = 0;
        while attempt < attempts {
            let route = self.route();
            let this = self.bucket_id(&route);
            let ready = route.bucket.cooling_until(Instant::now()).is_none();
            let switched = previous.as_ref().is_some_and(|p| *p != this) && ready;
            if !switched {
                match std::mem::replace(&mut wait, Wait::Nothing) {
                    Wait::Nothing => {}
                    Wait::Backoff => {
                        tokio::time::sleep(self.retry.backoff(backoffs)).await;
                        backoffs += 1;
                    }
                    Wait::For(d) => tokio::time::sleep(d).await,
                    // The bucket itself holds the request until then.
                    Wait::Paused => {}
                    // Longer than this wallet will make somebody wait.
                    Wait::TooLong => break,
                }
            }
            previous = Some(this);

            // Queued on a host that starts refusing, a request with somewhere
            // else to go goes there: that is not an attempt, and costs none.
            let permit = if ready && alternatives {
                match route.bucket.acquire_or_leave().await {
                    Some(p) => p,
                    None => continue,
                }
            } else {
                route.bucket.acquire().await
            };
            attempt += 1;
            let outcome = self.send_once(&route, &req).await;
            let (retryable, next) = match &outcome {
                Ok(reply) => match self.verdict(reply) {
                    Verdict::Final => {
                        permit.succeeded();
                        return outcome;
                    }
                    Verdict::RateLimited { retry_after } => {
                        let cooldown = retry_after
                            .unwrap_or(self.retry.rate_limit_cooldown)
                            .min(MAX_COOLDOWN);
                        permit.rate_limited(cooldown, retry_after.map(|d| self.retry.honour(d)));
                        // Refused at the door - a 429, or TronGrid's 403 -
                        // so nothing was submitted. A rate limit reported
                        // inside a 200 came from the application, which may
                        // already have acted on the request.
                        let at_the_door = matches!(reply.status, 403 | 429);
                        let next = match retry_after {
                            None => Wait::Backoff,
                            Some(d) if d > self.retry.max_retry_after => Wait::TooLong,
                            Some(_) => Wait::Paused,
                        };
                        (req.policy == Policy::Read || at_the_door, next)
                    }
                    Verdict::Unavailable => {
                        permit.failed(self.retry.failure_cooldown);
                        let next = match reply.retry_after {
                            None => Wait::Backoff,
                            Some(d) if d > self.retry.max_retry_after => Wait::TooLong,
                            Some(d) => Wait::For(d),
                        };
                        (req.policy == Policy::Read, next)
                    }
                },
                // A request that cannot be built will not build next time.
                Err(NetError::Request(_)) => return outcome,
                Err(e) => {
                    permit.failed(self.retry.failure_cooldown);
                    (req.policy == Policy::Read || e.never_sent(), Wait::Backoff)
                }
            };
            last = outcome.map(|mut r| {
                r.rate_limited = matches!(self.verdict(&r), Verdict::RateLimited { .. });
                r
            });
            if !retryable {
                break;
            }
            wait = next;
        }
        last
    }

    fn bucket_id(&self, route: &Route) -> BucketId {
        let ep = &self.endpoints[route.endpoint];
        let key = match (route.key, &self.credential) {
            (Some(k), Some(c)) => Some(c.ring.fingerprint_of(k)),
            _ => self.url_key,
        };
        BucketId::new(&ep.host, key)
    }
}

/// The longest an endpoint or key is passed over on a server's say-so.
const MAX_COOLDOWN: Duration = Duration::from_secs(600);

/// What has to happen before the same place is asked again.
#[derive(Debug, Clone, Copy)]
enum Wait {
    Nothing,
    Backoff,
    For(Duration),
    Paused,
    TooLong,
}

#[cfg(test)]
mod tests;
