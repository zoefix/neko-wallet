//! How long to wait before asking again, and when to stop.

use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The retry schedule, and how long an endpoint or key sits out after failing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Retry {
    /// Attempts in all, the first included.
    pub attempts: u32,
    /// The first backoff; each later one doubles.
    pub base_delay: Duration,
    /// No single backoff is longer than this.
    pub max_delay: Duration,
    /// A server's `Retry-After` is honoured up to here and no further. A node
    /// that asks for five minutes gets this long, and a screen that would have
    /// waited five minutes gets an answer instead.
    pub max_retry_after: Duration,
    /// How long a rate-limited endpoint or key is passed over when the server
    /// did not say. TronGrid suspends a key for thirty seconds.
    pub rate_limit_cooldown: Duration,
    /// How long an endpoint that failed outright is passed over. Shorter: a
    /// timeout says less about the next request than a 429 does.
    pub failure_cooldown: Duration,
    /// How long a queue on one endpoint or key may grow before the next one
    /// takes the overflow. Pacing a burst under a node's limit is what keeps
    /// it from refusing; this is what keeps the pacing from making a
    /// screenful of balances wait in single file when there is a fallback.
    pub spill_after: Duration,
}

impl Default for Retry {
    fn default() -> Self {
        Retry {
            attempts: 4,
            base_delay: Duration::from_millis(500),
            max_delay: Duration::from_secs(8),
            max_retry_after: Duration::from_secs(30),
            rate_limit_cooldown: Duration::from_secs(30),
            failure_cooldown: Duration::from_secs(10),
            spill_after: Duration::from_secs(1),
        }
    }
}

impl Retry {
    /// The wait before attempt `n + 1`, after `n` failures that gave no hint.
    ///
    /// Exponential, with the upper half jittered: clients that failed together
    /// should not all come back in the same millisecond and fail together
    /// again.
    pub fn backoff(&self, n: u32) -> Duration {
        let full = self
            .base_delay
            .saturating_mul(1u32 << n.min(16))
            .min(self.max_delay);
        let half = full / 2;
        half + half.mul_f64(unit_random())
    }

    /// What a server's own hint comes to, after the cap.
    pub fn honour(&self, retry_after: Duration) -> Duration {
        retry_after.min(self.max_retry_after)
    }
}

/// A number in `[0, 1)`. Only spreads retries out, so it needs no more than
/// the standard library's per-process random keys.
fn unit_random() -> f64 {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(COUNTER.fetch_add(1, Ordering::Relaxed));
    (h.finish() >> 11) as f64 / (1u64 << 53) as f64
}

/// A `Retry-After` header: delta-seconds, or an HTTP date.
///
/// Fractions are accepted although the RFC has none, because some gateways
/// send them; a date in the past means now.
pub fn parse_retry_after(value: &str) -> Option<Duration> {
    let v = value.trim();
    if let Ok(secs) = v.parse::<f64>() {
        return (secs.is_finite() && secs >= 0.0).then(|| Duration::from_secs_f64(secs.min(1e9)));
    }
    let at = parse_http_date(v)?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some(Duration::from_secs(at.saturating_sub(now)))
}

/// Seconds since the epoch, from an IMF-fixdate (`Sun, 06 Nov 1994 08:49:37
/// GMT`) - the only form a server is allowed to send today.
fn parse_http_date(s: &str) -> Option<u64> {
    let mut parts = s.split_whitespace();
    let _weekday = parts.next()?;
    let day: u64 = parts.next()?.parse().ok()?;
    let month = match parts.next()? {
        "Jan" => 1,
        "Feb" => 2,
        "Mar" => 3,
        "Apr" => 4,
        "May" => 5,
        "Jun" => 6,
        "Jul" => 7,
        "Aug" => 8,
        "Sep" => 9,
        "Oct" => 10,
        "Nov" => 11,
        "Dec" => 12,
        _ => return None,
    };
    let year: i64 = parts.next()?.parse().ok()?;
    let mut hms = parts.next()?.split(':').map(|p| p.parse::<u64>().ok());
    let (h, m, sec) = (hms.next()??, hms.next()??, hms.next()??);
    if parts.next()? != "GMT" || !(1..=31).contains(&day) || h > 23 || m > 59 || sec > 60 {
        return None;
    }
    let days = days_from_civil(year, month, day as i64);
    u64::try_from(days * 86_400)
        .ok()
        .map(|d| d + h * 3600 + m * 60 + sec)
}

/// Days since 1970-01-01 in the proleptic Gregorian calendar (Howard
/// Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_with_jitter_and_stops_at_the_cap() {
        let r = Retry::default();
        for n in 0..12 {
            let full = (Duration::from_millis(500) * (1 << n)).min(Duration::from_secs(8));
            for _ in 0..50 {
                let d = r.backoff(n);
                assert!(
                    d >= full / 2 && d <= full,
                    "attempt {n}: {d:?} outside [{:?}, {full:?}]",
                    full / 2
                );
            }
        }
        // Jittered, not constant.
        let samples: std::collections::HashSet<_> = (0..20).map(|_| r.backoff(3)).collect();
        assert!(samples.len() > 1);
        // A huge attempt number neither overflows nor exceeds the cap.
        assert!(r.backoff(u32::MAX) <= Duration::from_secs(8));
    }

    #[test]
    fn retry_after_seconds_and_fractions() {
        assert_eq!(parse_retry_after("30"), Some(Duration::from_secs(30)));
        assert_eq!(parse_retry_after(" 0 "), Some(Duration::ZERO));
        assert_eq!(parse_retry_after("1.5"), Some(Duration::from_millis(1500)));
        assert_eq!(parse_retry_after("-1"), None);
        assert_eq!(parse_retry_after("soon"), None);
        assert_eq!(parse_retry_after(""), None);
        assert_eq!(parse_retry_after("NaN"), None);
    }

    #[test]
    fn retry_after_dates() {
        assert_eq!(
            parse_http_date("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(784_111_777)
        );
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(
            parse_http_date("Tue, 29 Feb 2028 12:00:00 GMT"),
            Some(1_835_438_400)
        );
        // A date already past is "now", not an error and not a huge wait.
        assert_eq!(
            parse_retry_after("Sun, 06 Nov 1994 08:49:37 GMT"),
            Some(Duration::ZERO)
        );
        // Ninety seconds ahead, give or take the second the test takes.
        let ahead = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            + 90;
        let date = format_http_date(ahead);
        let got = parse_retry_after(&date).unwrap();
        assert!((88..=90).contains(&got.as_secs()), "{date} -> {got:?}");
        for bad in [
            "Sun, 06 Foo 1994 08:49:37 GMT",
            "Sun, 06 Nov 1994 25:00:00 GMT",
            "06 Nov 1994",
        ] {
            assert_eq!(parse_http_date(bad), None, "{bad}");
        }
    }

    #[test]
    fn retry_after_is_capped() {
        let r = Retry::default();
        assert_eq!(r.honour(Duration::from_secs(5)), Duration::from_secs(5));
        assert_eq!(r.honour(Duration::from_secs(300)), Duration::from_secs(30));
        assert_eq!(
            r.honour(parse_retry_after("86400").unwrap()),
            r.max_retry_after
        );
    }

    fn format_http_date(secs: u64) -> String {
        // Inverse of days_from_civil, for the test only.
        let days = (secs / 86_400) as i64;
        let rem = secs % 86_400;
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = yoe + era * 400 + i64::from(m <= 2);
        const MONTHS: [&str; 12] = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        format!(
            "Mon, {d:02} {} {y} {:02}:{:02}:{:02} GMT",
            MONTHS[(m - 1) as usize],
            rem / 3600,
            rem % 3600 / 60,
            rem % 60
        )
    }
}
