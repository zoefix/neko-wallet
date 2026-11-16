//! The connection pool every chain shares.

use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tokio::runtime::{Handle, Id};

/// How long a whole request may take, unless the caller asks for longer.
pub const TIMEOUT: Duration = Duration::from_secs(30);

/// Short, so a dead fallback costs seconds rather than the whole timeout.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Clients kept for runtimes other than the current one. The wallet has one
/// runtime; a test binary has one per test, and those come and go.
const MAX_RUNTIMES: usize = 8;

type Pools = Mutex<Vec<(Option<Id>, reqwest::Client)>>;

fn pools() -> &'static Pools {
    static CLIENTS: OnceLock<Pools> = OnceLock::new();
    CLIENTS.get_or_init(Default::default)
}

/// The shared client.
///
/// One per tokio runtime, which in the wallet means one per process. A pooled
/// connection is driven by a task on the runtime that opened it, and a request
/// on another runtime that picks it up after the first has shut down fails as
/// though the network had - which is what every `#[tokio::test]` does when it
/// returns. Keying on the runtime keeps the pool shared wherever sharing is
/// sound.
pub fn client() -> reqwest::Client {
    let id = Handle::try_current().ok().map(|h| h.id());
    let mut clients = pools().lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, c)) = clients.iter().find(|(k, _)| *k == id) {
        return c.clone();
    }
    let c = build();
    if clients.len() >= MAX_RUNTIMES {
        clients.remove(0);
    }
    clients.push((id, c.clone()));
    c
}

fn build() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(TIMEOUT)
        .connect_timeout(CONNECT_TIMEOUT)
        .build()
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries_for(id: Id) -> usize {
        pools()
            .lock()
            .unwrap()
            .iter()
            .filter(|(k, _)| *k == Some(id))
            .count()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn one_runtime_builds_one_client() {
        let id = Handle::current().id();
        let _ = client();
        let _ = tokio::spawn(async { client() }).await.unwrap();
        let _ = client();
        assert_eq!(entries_for(id), 1);
    }
}
