//! Several API keys for one service, used in turn.
//!
//! TronGrid limits each key separately, so two keys are twice the allowance -
//! but only if requests are actually spread across them, and only if a key
//! that has just been suspended is left alone until it is reinstated. Both
//! need memory that outlives a client, because a client lives for one request.
//!
//! A key is a credential. It is never a map key, never logged and never in a
//! `Debug` string: what identifies one is a fingerprint, keyed per process, so
//! even the fingerprint means nothing outside it.

use std::collections::hash_map::RandomState;
use std::collections::HashMap;
use std::hash::BuildHasher;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::throttle::{self, Bucket, BucketId};

/// A fingerprint of a secret, stable for the life of the process and useless
/// after it.
pub fn fingerprint(secret: &str) -> u64 {
    static STATE: OnceLock<RandomState> = OnceLock::new();
    STATE.get_or_init(RandomState::new).hash_one(secret)
}

/// The keys in a setting, in the order written.
///
/// Commas, semicolons and any whitespace separate them - the full-width forms
/// too, which are what a CJK input method types - so a list pasted from
/// anywhere reads the same. Repeats are dropped.
pub fn split(raw: &str) -> Vec<&str> {
    let separator = |c: char| {
        matches!(c, ',' | ';' | '\u{ff0c}' | '\u{ff1b}' | '\u{3001}') || c.is_whitespace()
    };
    let mut out: Vec<&str> = Vec::new();
    for k in raw.split(separator) {
        if !k.is_empty() && !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

struct Key {
    secret: String,
    fingerprint: u64,
}

/// Zero or more keys for one service.
pub struct KeyRing {
    keys: Vec<Key>,
    /// Shared by every ring holding the same keys, so a client built for one
    /// request starts where the last one stopped rather than at the first key.
    cursor: Arc<AtomicUsize>,
}

impl std::fmt::Debug for KeyRing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KeyRing({} keys)", self.keys.len())
    }
}

impl KeyRing {
    pub fn parse(raw: Option<&str>) -> Self {
        let keys: Vec<Key> = split(raw.unwrap_or(""))
            .into_iter()
            .map(|k| Key {
                secret: k.to_string(),
                fingerprint: fingerprint(k),
            })
            .collect();
        let ring_id = keys
            .iter()
            .fold(0u64, |acc, k| acc.rotate_left(7) ^ k.fingerprint);
        KeyRing {
            cursor: cursor(ring_id),
            keys,
        }
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub(crate) fn secret(&self, i: usize) -> &str {
        &self.keys[i].secret
    }

    /// The next key for `host`, round-robin, passing over any whose bucket is
    /// cooling down or has a queue longer than `spill`. Failing that, the
    /// least busy; failing that, the one that comes back first.
    pub(crate) fn pick(
        &self,
        host: &str,
        now: Instant,
        spill: Duration,
    ) -> Option<(usize, Arc<Bucket>)> {
        let n = self.keys.len();
        if n == 0 {
            return None;
        }
        let start = self.cursor.fetch_add(1, Ordering::Relaxed);
        let mut least_busy: Option<(Duration, usize, Arc<Bucket>)> = None;
        let mut soonest: Option<(Instant, usize, Arc<Bucket>)> = None;
        for step in 0..n {
            let i = (start + step) % n;
            let b = throttle::bucket(&BucketId::new(host, Some(self.keys[i].fingerprint)));
            if let Some(until) = b.cooling_until(now) {
                if soonest.as_ref().is_none_or(|(t, ..)| until < *t) {
                    soonest = Some((until, i, b));
                }
                continue;
            }
            let backlog = b.backlog(now);
            if backlog <= spill {
                return Some((i, b));
            }
            if least_busy.as_ref().is_none_or(|(d, ..)| backlog < *d) {
                least_busy = Some((backlog, i, b));
            }
        }
        least_busy
            .map(|(_, i, b)| (i, b))
            .or(soonest.map(|(_, i, b)| (i, b)))
    }

    pub(crate) fn fingerprint_of(&self, i: usize) -> u64 {
        self.keys[i].fingerprint
    }
}

fn cursor(ring_id: u64) -> Arc<AtomicUsize> {
    static CURSORS: OnceLock<Mutex<HashMap<u64, Arc<AtomicUsize>>>> = OnceLock::new();
    let mut map = CURSORS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    Arc::clone(map.entry(ring_id).or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SPILL: Duration = Duration::from_secs(1);

    #[test]
    fn a_setting_splits_into_keys() {
        assert_eq!(split("a"), ["a"]);
        assert_eq!(split(" a, b ,c\n d;e "), ["a", "b", "c", "d", "e"]);
        assert_eq!(split("a\u{ff0c}b\u{3001}c\u{ff1b}d"), ["a", "b", "c", "d"]);
        assert_eq!(split("a,,a, b"), ["a", "b"]);
        assert!(split(" , ").is_empty());
        assert!(KeyRing::parse(None).is_empty());
        assert_eq!(KeyRing::parse(Some("x, y")).len(), 2);
    }

    #[test]
    fn the_secret_never_reaches_debug_output() {
        let ring = KeyRing::parse(Some("super-secret-1,super-secret-2"));
        let shown = format!("{ring:?}");
        assert!(!shown.contains("secret"), "{shown}");
        assert_eq!(shown, "KeyRing(2 keys)");
        assert_ne!(fingerprint("super-secret-1"), fingerprint("super-secret-2"));
        assert_eq!(fingerprint("super-secret-1"), fingerprint("super-secret-1"));
    }

    /// Two rings built from the same setting - two clients, one request each -
    /// continue each other's rotation.
    #[test]
    fn rotation_survives_rebuilding_the_client() {
        let host = "rotation.test.invalid";
        let setting = "rot-k1,rot-k2,rot-k3";
        let mut seen = Vec::new();
        for _ in 0..6 {
            let ring = KeyRing::parse(Some(setting));
            let (i, _) = ring.pick(host, Instant::now(), SPILL).unwrap();
            seen.push(ring.secret(i).to_string());
        }
        // Whatever the starting point, every key is used twice and no key
        // twice in a row.
        for k in ["rot-k1", "rot-k2", "rot-k3"] {
            assert_eq!(seen.iter().filter(|s| *s == k).count(), 2, "{seen:?}");
        }
        assert!(seen.windows(2).all(|w| w[0] != w[1]), "{seen:?}");
    }

    #[tokio::test]
    async fn a_cooling_key_is_skipped_until_it_is_back() {
        let host = "skip.test.invalid";
        let ring = KeyRing::parse(Some("skip-a,skip-b"));
        let a = ring.keys.iter().position(|k| k.secret == "skip-a").unwrap();
        let bucket_a = throttle::bucket(&BucketId::new(host, Some(ring.fingerprint_of(a))));
        bucket_a
            .acquire()
            .await
            .rate_limited(Duration::from_millis(200), None);

        for _ in 0..6 {
            let (i, _) = ring.pick(host, Instant::now(), SPILL).unwrap();
            assert_eq!(ring.secret(i), "skip-b");
        }
        tokio::time::sleep(Duration::from_millis(220)).await;
        let used: Vec<_> = (0..4)
            .map(|_| {
                ring.secret(ring.pick(host, Instant::now(), SPILL).unwrap().0)
                    .to_string()
            })
            .collect();
        assert!(used.iter().any(|k| k == "skip-a"), "{used:?}");
    }

    #[tokio::test]
    async fn when_every_key_is_cooling_the_first_back_is_used() {
        let host = "allcool.test.invalid";
        let ring = KeyRing::parse(Some("cool-a,cool-b"));
        for (i, ms) in [(0usize, 5_000u64), (1, 1_000)] {
            throttle::bucket(&BucketId::new(host, Some(ring.fingerprint_of(i))))
                .acquire()
                .await
                .rate_limited(Duration::from_millis(ms), None);
        }
        for _ in 0..3 {
            let (i, _) = ring.pick(host, Instant::now(), SPILL).unwrap();
            assert_eq!(i, 1);
        }
    }
}
