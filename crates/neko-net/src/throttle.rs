//! Pacing requests to one host, before the host has to ask.
//!
//! A wallet list asks every chain about every wallet at once. Unpaced, that is
//! forty requests to TronGrid in the same millisecond, and a keyless client is
//! allowed a handful a second: the node answers 429 to most of them and the
//! screen fills with errors that were entirely self-inflicted.
//!
//! Each bucket - a host, or a host and one API key - limits how many requests
//! are in flight and how close together they may *start*. Both limits move:
//! a rate limit halves the one and doubles the other, and every few successes
//! give a little back, never past where they started. A bucket also remembers
//! when it was last refused, which is what failover and key rotation consult
//! to pass over it while it cools down.
//!
//! The state is process-wide on purpose. Clients are rebuilt for every
//! request, so anything a client held would be forgotten by the next one -
//! including the fact that this host said "slow down" a second ago.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use tokio::sync::Notify;

/// How hard one bucket may be pushed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    /// Requests in flight at once.
    pub in_flight: usize,
    /// The least time between two requests starting.
    pub spacing: Duration,
}

impl Limits {
    pub const fn new(in_flight: usize, spacing_ms: u64) -> Self {
        Limits {
            in_flight,
            spacing: Duration::from_millis(spacing_ms),
        }
    }

    /// How far a run of rate limits may widen the spacing.
    fn max_spacing(&self) -> Duration {
        (self.spacing * 8).clamp(Duration::from_secs(2), Duration::from_secs(10))
    }
}

/// Where a host's limits start.
///
/// From each operator's published or measured allowance, a little under it:
/// pacing that only works when nobody else on the same IP is asking is not
/// pacing. A key buys more, which is the reason to have one - and with several
/// keys each gets its own bucket, so rotation adds throughput rather than
/// sharing one allowance between them.
pub fn base_limits(host: &str, keyed: bool) -> Limits {
    let h = host.split(':').next().unwrap_or(host);
    let is = |d: &str| h == d || h.ends_with(&format!(".{d}"));
    match () {
        // 15 requests a second per key. Keyless it says `allowed_rps(3)` and
        // counts arrivals, which network jitter bunches up, so well under.
        _ if is("trongrid.io") && keyed => Limits::new(8, 75),
        _ if is("trongrid.io") => Limits::new(2, 400),
        // One request a second without a key, ten with one.
        _ if is("toncenter.com") && keyed => Limits::new(4, 110),
        _ if is("toncenter.com") => Limits::new(1, 1100),
        // 100 requests per 10 s per IP, 40 per 10 s for any one method.
        _ if h == "api.mainnet-beta.solana.com" => Limits::new(4, 150),
        _ if is("blockstream.info") || is("mempool.space") => Limits::new(3, 200),
        _ if h == "api.mainnet.aptoslabs.com" => Limits::new(2, 250),
        _ if is("aptoslabs.com") => Limits::new(4, 100),
        _ if is("api.etherscan.io") => Limits::new(2, 250),
        _ if is("routescan.io") => Limits::new(2, 250),
        _ if is("nodereal.io") => Limits::new(4, 100),
        _ if is("drpc.org") => Limits::new(4, 100),
        _ if is("publicnode.com") => Limits::new(6, 40),
        // Somebody's own node, or a public one with no published limit: start
        // generous and let the replies say otherwise.
        _ => Limits::new(8, 20),
    }
}

/// A host, or a host and the fingerprint of one key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BucketId {
    host: String,
    key: Option<u64>,
}

impl BucketId {
    pub fn new(host: &str, key: Option<u64>) -> Self {
        BucketId {
            host: host.to_ascii_lowercase(),
            key,
        }
    }
}

pub struct Bucket {
    base: Limits,
    state: Mutex<State>,
    /// A place in flight came free.
    freed: Notify,
    /// The bucket started cooling down: whoever is queued and has somewhere
    /// else to go should go there.
    cooled: Notify,
}

struct State {
    in_flight: usize,
    max_in_flight: usize,
    /// Callers waiting for a place in flight.
    queued: usize,
    spacing: Duration,
    next_start: Instant,
    cooling_until: Option<Instant>,
    limited_at: Option<Instant>,
    streak: u32,
}

/// Successes it takes to give back one step of what a rate limit took.
const RELAX_EVERY: u32 = 4;

impl Bucket {
    pub fn new(base: Limits) -> Self {
        Bucket {
            base,
            state: Mutex::new(State {
                in_flight: 0,
                max_in_flight: base.in_flight.max(1),
                queued: 0,
                spacing: base.spacing,
                next_start: Instant::now(),
                cooling_until: None,
                limited_at: None,
                streak: 0,
            }),
            freed: Notify::new(),
            cooled: Notify::new(),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Wait for a place in flight and a start slot.
    ///
    /// Slots are handed out in order, each `spacing` after the last, so a
    /// burst becomes a queue rather than a stampede.
    pub async fn acquire(self: &Arc<Self>) -> Permit {
        loop {
            if let Some(p) = self.wait(false).await {
                return p;
            }
        }
    }

    /// The same, for a caller with somewhere else to go: `None` as soon as
    /// this bucket starts cooling down, rather than a place in a queue behind
    /// a host that has just said no.
    ///
    /// Without this a burst commits to the default node before the first
    /// refusal arrives, and then waits out every pause the default asks for
    /// while the fallback sits idle.
    pub async fn acquire_or_leave(self: &Arc<Self>) -> Option<Permit> {
        self.wait(true).await
    }

    async fn wait(self: &Arc<Self>, may_leave: bool) -> Option<Permit> {
        loop {
            let freed = self.freed.notified();
            let cooled = self.cooled.notified();
            tokio::pin!(freed, cooled);
            // Registered before the state is read, so a refusal that lands in
            // between still wakes this waiter.
            cooled.as_mut().enable();
            let slot = {
                let mut s = self.lock();
                let now = Instant::now();
                if may_leave && s.cooling_until.is_some_and(|t| t > now) {
                    return None;
                }
                if s.in_flight < s.max_in_flight {
                    s.in_flight += 1;
                    let slot = s.next_start.max(now);
                    s.next_start = slot + s.spacing;
                    Some(slot)
                } else {
                    s.queued += 1;
                    None
                }
            };
            match slot {
                Some(slot) => {
                    // Made before the wait, so a caller that gives up while
                    // queued still gives its place back.
                    let permit = Permit {
                        bucket: Arc::clone(self),
                        started: slot,
                    };
                    if !may_leave {
                        tokio::time::sleep_until(slot.into()).await;
                        return Some(permit);
                    }
                    tokio::select! {
                        _ = tokio::time::sleep_until(slot.into()) => return Some(permit),
                        _ = cooled => return None,
                    }
                }
                None => {
                    let _queued = Queued(self);
                    if !may_leave {
                        freed.await;
                        continue;
                    }
                    tokio::select! {
                        _ = freed => {}
                        _ = cooled => return None,
                    }
                }
            }
        }
    }

    /// Roughly how long a request arriving now would wait before it could
    /// start: the slots already handed out, and one spacing for each caller
    /// queued for a place.
    pub fn backlog(&self, now: Instant) -> Duration {
        let s = self.lock();
        s.next_start.saturating_duration_since(now) + s.spacing * s.queued as u32
    }

    /// When this bucket stops cooling down, or `None` if it is not.
    pub fn cooling_until(&self, now: Instant) -> Option<Instant> {
        self.lock().cooling_until.filter(|t| *t > now)
    }

    /// The current limits, after adaptation.
    pub fn current(&self) -> Limits {
        let s = self.lock();
        Limits {
            in_flight: s.max_in_flight,
            spacing: s.spacing,
        }
    }

    fn release(&self) {
        let mut s = self.lock();
        s.in_flight = s.in_flight.saturating_sub(1);
        drop(s);
        self.freed.notify_one();
    }

    fn succeeded(&self, started: Instant) {
        let mut s = self.lock();
        // A request that set off before the refusal says nothing about
        // whether the refusal is over.
        if s.limited_at.is_none_or(|t| started >= t) {
            s.cooling_until = None;
        }
        s.streak += 1;
        if s.streak >= RELAX_EVERY {
            s.streak = 0;
            s.spacing = (s.spacing * 7 / 8).max(self.base.spacing);
            if s.max_in_flight < self.base.in_flight.max(1) {
                s.max_in_flight += 1;
                drop(s);
                self.freed.notify_one();
            }
        }
    }

    fn rate_limited(&self, cooldown: Duration, pause: Option<Duration>) {
        let now = Instant::now();
        let mut s = self.lock();
        s.streak = 0;
        s.limited_at = Some(now);
        s.cooling_until = Some(s.cooling_until.unwrap_or(now).max(now + cooldown));
        s.spacing = (s.spacing * 2)
            .max(Duration::from_millis(100))
            .min(self.base.max_spacing());
        s.max_in_flight = (s.max_in_flight / 2).max(1);
        // The server named a time: nobody on this bucket starts before it.
        if let Some(p) = pause {
            s.next_start = s.next_start.max(now + p);
        }
        drop(s);
        self.cooled.notify_waiters();
    }

    fn failed(&self, cooldown: Duration) {
        let now = Instant::now();
        let mut s = self.lock();
        s.streak = 0;
        s.cooling_until = Some(s.cooling_until.unwrap_or(now).max(now + cooldown));
        drop(s);
        self.cooled.notify_waiters();
    }
}

/// One caller counted as waiting for a place, until it stops waiting -
/// however it stops.
struct Queued<'a>(&'a Bucket);

impl Drop for Queued<'_> {
    fn drop(&mut self) {
        let mut s = self.0.lock();
        s.queued = s.queued.saturating_sub(1);
    }
}

/// A place in flight. Dropping it gives the place back; reporting how the
/// request went is what moves the limits.
pub struct Permit {
    bucket: Arc<Bucket>,
    started: Instant,
}

impl Permit {
    pub fn succeeded(self) {
        self.bucket.succeeded(self.started);
    }

    /// `cooldown` is how long failover passes this bucket over; `pause` is a
    /// wait the server itself asked for, which holds back every request here.
    pub fn rate_limited(self, cooldown: Duration, pause: Option<Duration>) {
        self.bucket.rate_limited(cooldown, pause);
    }

    pub fn failed(self, cooldown: Duration) {
        self.bucket.failed(cooldown);
    }
}

impl Drop for Permit {
    fn drop(&mut self) {
        self.bucket.release();
    }
}

/// The bucket for a host and key, shared by every client in the process.
pub fn bucket(id: &BucketId) -> Arc<Bucket> {
    static BUCKETS: OnceLock<Mutex<HashMap<BucketId, Arc<Bucket>>>> = OnceLock::new();
    let mut map = BUCKETS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    Arc::clone(
        map.entry(id.clone())
            .or_insert_with(|| Arc::new(Bucket::new(base_limits(&id.host, id.key.is_some())))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[tokio::test]
    async fn starts_are_spaced() {
        let b = Arc::new(Bucket::new(Limits::new(10, 40)));
        let t0 = Instant::now();
        let mut starts = Vec::new();
        for _ in 0..5 {
            let p = b.acquire().await;
            starts.push(Instant::now() - t0);
            p.succeeded();
        }
        // Slots are exactly one spacing apart; a timer that wakes late for one
        // and on time for the next can make a single gap look short, so the
        // guarantee is on each slot rather than on each gap.
        for (k, s) in starts.iter().enumerate() {
            assert!(
                *s >= Duration::from_millis(40 * k as u64),
                "start {k} too early: {starts:?}"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn in_flight_is_capped() {
        let b = Arc::new(Bucket::new(Limits::new(2, 0)));
        let now = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = (0..8)
            .map(|_| {
                let (b, now, peak) = (b.clone(), now.clone(), peak.clone());
                tokio::spawn(async move {
                    let p = b.acquire().await;
                    let n = now.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(n, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    now.fetch_sub(1, Ordering::SeqCst);
                    p.succeeded();
                })
            })
            .collect();
        for t in tasks {
            t.await.unwrap();
        }
        assert_eq!(peak.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn a_rate_limit_widens_and_successes_relax_back_to_base() {
        let base = Limits::new(4, 100);
        let b = Arc::new(Bucket::new(base));

        b.acquire()
            .await
            .rate_limited(Duration::from_secs(30), None);
        assert_eq!(b.current(), Limits::new(2, 200));
        b.acquire()
            .await
            .rate_limited(Duration::from_secs(30), None);
        assert_eq!(b.current(), Limits::new(1, 400));

        // Capped: a run of refusals does not grow the gap without bound.
        for _ in 0..10 {
            b.rate_limited(Duration::ZERO, None);
        }
        assert_eq!(b.current().spacing, base.max_spacing());
        assert_eq!(b.current().in_flight, 1);

        // Slowly back, and never past where it started.
        for _ in 0..RELAX_EVERY {
            b.succeeded(Instant::now());
        }
        let after_one_step = b.current();
        assert!(after_one_step.spacing < base.max_spacing());
        assert_eq!(after_one_step.in_flight, 2);
        for _ in 0..400 {
            b.succeeded(Instant::now());
        }
        assert_eq!(b.current(), base);
    }

    #[tokio::test]
    async fn a_named_wait_holds_back_the_whole_bucket() {
        let b = Arc::new(Bucket::new(Limits::new(4, 0)));
        b.acquire()
            .await
            .rate_limited(Duration::from_secs(1), Some(Duration::from_millis(150)));
        let t0 = Instant::now();
        b.acquire().await.succeeded();
        assert!(Instant::now() - t0 >= Duration::from_millis(140));
    }

    #[tokio::test]
    async fn cooling_ends_on_its_own_or_with_a_fresh_success() {
        let b = Arc::new(Bucket::new(Limits::new(4, 0)));
        let early = b.acquire().await;
        b.acquire().await.rate_limited(Duration::from_secs(5), None);
        assert!(b.cooling_until(Instant::now()).is_some());

        // A request that started before the refusal does not end it.
        early.succeeded();
        assert!(b.cooling_until(Instant::now()).is_some());

        // One that started after it does.
        b.acquire().await.succeeded();
        assert!(b.cooling_until(Instant::now()).is_none());

        b.acquire().await.failed(Duration::from_millis(50));
        assert!(b.cooling_until(Instant::now()).is_some());
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(b.cooling_until(Instant::now()).is_none());
    }

    /// Queued behind a host that has just refused, a caller with somewhere
    /// else to go leaves at once - whether it was waiting for a place or for
    /// its start slot - and gives its place back.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_refusal_releases_whoever_can_go_elsewhere() {
        let b = Arc::new(Bucket::new(Limits::new(2, 300)));
        let first = b.acquire().await;
        // In flight 2: this one holds a slot 300 ms out.
        let slotted = {
            let b = b.clone();
            tokio::spawn(async move { b.acquire_or_leave().await.is_some() })
        };
        // And this one is waiting for a place.
        let queued = {
            let b = b.clone();
            tokio::spawn(async move { b.acquire_or_leave().await.is_some() })
        };
        tokio::time::sleep(Duration::from_millis(50)).await;
        let t0 = Instant::now();
        first.rate_limited(Duration::from_secs(5), None);
        assert!(!slotted.await.unwrap(), "the slotted caller stayed");
        assert!(!queued.await.unwrap(), "the queued caller stayed");
        assert!(Instant::now() - t0 < Duration::from_millis(100));

        // Nothing is held: a caller with nowhere else to go gets a place.
        let none_left = tokio::time::timeout(Duration::from_secs(3), b.acquire()).await;
        assert!(none_left.is_ok(), "a place was never given back");
        // And one that could leave, does, while the bucket is cooling.
        assert!(b.acquire_or_leave().await.is_none());
    }

    /// The backlog is what a newcomer would wait: handed-out slots, then the
    /// queue. It is what decides when a fallback takes the overflow.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_backlog_counts_slots_and_the_queue() {
        let b = Arc::new(Bucket::new(Limits::new(1, 100)));
        assert_eq!(b.backlog(Instant::now()), Duration::ZERO);
        let held = b.acquire().await;
        // One slot handed out: the next start is a spacing away.
        let one = b.backlog(Instant::now());
        assert!(one > Duration::from_millis(50) && one <= Duration::from_millis(100));

        let waiting: Vec<_> = (0..3)
            .map(|_| {
                let b = b.clone();
                tokio::spawn(async move { b.acquire().await.succeeded() })
            })
            .collect();
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(b.backlog(Instant::now()) >= Duration::from_millis(300));

        drop(held);
        for w in waiting {
            w.await.unwrap();
        }
        // Everyone who queued has left the count.
        assert!(b.backlog(Instant::now()) <= Duration::from_millis(100));
    }

    #[test]
    fn known_hosts_start_where_they_should() {
        assert_eq!(base_limits("api.trongrid.io", false), Limits::new(2, 400));
        assert_eq!(base_limits("api.trongrid.io", true), Limits::new(8, 75));
        assert_eq!(base_limits("toncenter.com", false).in_flight, 1);
        assert_eq!(
            base_limits("ethereum-rpc.publicnode.com", false),
            Limits::new(6, 40)
        );
        // A suffix match is on a label boundary, not a substring.
        assert_eq!(base_limits("nottrongrid.io", false), Limits::new(8, 20));
        assert_eq!(base_limits("127.0.0.1:8080", false), Limits::new(8, 20));
    }
}
