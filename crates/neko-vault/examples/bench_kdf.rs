//! Measures Argon2id on this machine. The profile table is frozen once shipped,
//! so measure before committing to it.
//!
//! Run with: cargo run --release -p neko-vault --example bench_kdf

use neko_vault::{header::FileHeader, keys, profile};
use std::time::Instant;

fn main() {
    match neko_vault::calibrate::recommend(None) {
        Ok(c) => println!(
            "calibration picks: {} (estimated {:.0?}, budget {:.0?})\n",
            c.profile.name,
            c.estimated,
            neko_vault::calibrate::DEFAULT_BUDGET
        ),
        Err(e) => println!("calibration failed: {e}\n"),
    }

    println!(
        "{:<10} {:>10} {:>4} {:>4} {:>12}",
        "profile", "mem", "t", "p", "median"
    );
    for p in profile::ALL {
        let header = FileHeader::new(*p).unwrap();
        let mut times = Vec::new();
        for _ in 0..3 {
            let t0 = Instant::now();
            keys::stretch("zoe@example.com", "correct horse battery staple", &header).unwrap();
            times.push(t0.elapsed());
        }
        times.sort();
        println!(
            "{:<10} {:>8} MiB {:>4} {:>4} {:>10.0?}",
            p.name,
            p.params.mem_kib / 1024,
            p.params.iters,
            p.params.par,
            times[1]
        );
    }
}
