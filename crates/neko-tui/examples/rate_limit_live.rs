//! A burst like the one a wallet list sends, against live public nodes.
//!
//! Forty keyless TRON balance reads and twenty reads on one EVM public RPC,
//! all at once, each through a client built for that one request - which is
//! what the wallet does. Prints how many came back and how long it took.
//!
//! Run: cargo run --release -p neko-tui --example rate_limit_live
//! `EVM_CHAIN=bsc` picks BNB Chain's node instead of Ethereum's.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

const TRON: &[&str] = &[
    "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH",
    "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t",
    "TKzxdSv2FZKQrEqkKVgp5DcwEXBEKMg2Ax",
    "TNUC9Qb1rRpS5CbWLmNMxXBjyFoydXjWFR",
];
const EVM: &str = "0xd8dA6BF26964aF9D7eEd9e03E53415D37aA96045";

struct Tally {
    ok: usize,
    errors: BTreeMap<String, usize>,
    slowest: Duration,
}

impl Tally {
    fn new() -> Self {
        Tally {
            ok: 0,
            errors: BTreeMap::new(),
            slowest: Duration::ZERO,
        }
    }

    fn add(&mut self, r: Result<(), String>, took: Duration) {
        self.slowest = self.slowest.max(took);
        match r {
            Ok(()) => self.ok += 1,
            Err(e) => *self.errors.entry(e.chars().take(90).collect()).or_default() += 1,
        }
    }

    fn print(&self, name: &str, total: usize, wall: Duration) {
        println!(
            "{name:<10} ok {:>2}/{total}   failed {:>2}   wall {:>5.1}s   slowest {:>5.1}s",
            self.ok,
            total - self.ok,
            wall.as_secs_f64(),
            self.slowest.as_secs_f64()
        );
        for (e, n) in &self.errors {
            println!("             {n:>2} x {e}");
        }
    }
}

#[tokio::main]
async fn main() {
    let evm_chain = match std::env::var("EVM_CHAIN").as_deref() {
        Ok("bsc") => neko_evm::BSC,
        _ => neko_evm::ETHEREUM,
    };
    let t0 = Instant::now();

    let tron: Vec<_> = (0..40)
        .map(|i| {
            let addr = neko_tron::Address::parse(TRON[i % TRON.len()]).unwrap();
            tokio::spawn(async move {
                let start = Instant::now();
                let c = neko_tron::TronGrid::new(None, None);
                let r = c
                    .trx_balance(addr)
                    .await
                    .map(|_| ())
                    .map_err(|e| e.to_string());
                (r, start.elapsed(), t0.elapsed())
            })
        })
        .collect();
    let evm: Vec<_> = (0..20)
        .map(|_| {
            let addr = neko_hd::EvmAddress::parse(EVM).unwrap();
            tokio::spawn(async move {
                let start = Instant::now();
                let c = neko_evm::client::Rpc::new(evm_chain, None);
                let r = c.balance(addr).await.map(|_| ()).map_err(|e| e.to_string());
                (r, start.elapsed(), t0.elapsed())
            })
        })
        .collect();

    let (mut t, mut tw) = (Tally::new(), Duration::ZERO);
    for h in tron {
        let (r, took, done) = h.await.unwrap();
        t.add(r, took);
        tw = tw.max(done);
    }
    let (mut e, mut ew) = (Tally::new(), Duration::ZERO);
    for h in evm {
        let (r, took, done) = h.await.unwrap();
        e.add(r, took);
        ew = ew.max(done);
    }

    println!("TRON default {}", neko_tron::DEFAULT_URL);
    t.print("TRON", 40, tw);
    println!(
        "EVM  default {} (chain {})",
        evm_chain.default_rpc, evm_chain.chain_id
    );
    e.print("EVM", 20, ew);
}
