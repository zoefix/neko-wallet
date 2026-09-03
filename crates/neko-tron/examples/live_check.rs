//! Read-only smoke test against a live TRON network.
//!
//! Run: cargo run --release -p neko-tron --example live_check

use neko_hd::Address;
use neko_tron::TronGrid;

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let c = TronGrid::new(None, std::env::var("TRONGRID_API_KEY").ok());
    println!("network: mainnet ({})", neko_tron::DEFAULT_URL);

    match c.tx_params(0).await {
        Ok(p) => println!(
            "  block ref  #{} hash ..{}",
            p.ref_block_num,
            hex_tail(&p.ref_block_hash)
        ),
        Err(e) => return println!("  FAILED to get a block reference: {e}"),
    }

    let usdt = neko_tron::usdt_address();
    match c.verify_usdt(usdt).await {
        Ok((sym, dec)) => println!("  USDT       {usdt} -> symbol={sym:?} decimals={dec}"),
        Err(e) => println!("  USDT       verification failed: {e}"),
    }

    // A well-known address with activity, just to exercise the read paths.
    let probe = Address::parse("TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH").unwrap();
    match c.trx_balance(probe).await {
        Ok(sun) => println!("  TRX bal    {} sun ({} TRX)", sun, sun as f64 / 1e6),
        Err(e) => println!("  TRX bal    failed: {e}"),
    }
    match c.trc20_balance(usdt, probe).await {
        Ok(b) => println!("  USDT bal   {b} (raw units)"),
        Err(e) => println!("  USDT bal   failed: {e}"),
    }
    match c.account_resources(probe).await {
        Ok(r) => println!(
            "  resources  energy {}/{}   bandwidth {}/{}",
            r.energy_available, r.energy_limit, r.bandwidth_available, r.bandwidth_limit
        ),
        Err(e) => println!("  resources  failed: {e}"),
    }
    match c.prices().await {
        Ok(p) => println!(
            "  prices     {} sun/energy   {} sun/byte",
            p.sun_per_energy, p.sun_per_bandwidth
        ),
        Err(e) => println!("  prices     failed: {e}"),
    }
    match c.history_trx(probe, 3).await {
        Ok(v) => {
            let n = v
                .get("data")
                .and_then(|d| d.as_array())
                .map(Vec::len)
                .unwrap_or(0);
            println!("  history    v1 API reachable, {n} recent transactions");
        }
        Err(e) => println!("  history    {e}"),
    }
}

fn hex_tail(b: &[u8; 32]) -> String {
    b[28..].iter().map(|x| format!("{x:02x}")).collect()
}
