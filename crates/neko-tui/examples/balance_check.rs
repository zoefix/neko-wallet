//! Calls the exact function the assets screen calls, so a regression in the
//! display path shows up here rather than as a blank column in the UI.
//!
//! Run: cargo run --release -p neko-tui --example balance_check

use neko_hd::{derive, Address};

const LEDGER_PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let seed = derive::seed_from_mnemonic(LEDGER_PHRASE, "").unwrap();
    let addr: Address = derive::address_at(&seed, 0, 0).unwrap();

    println!("network mainnet   address {addr}");
    let client = neko_tui::chain::client(None, std::env::var("TRONGRID_API_KEY").ok());

    match neko_tui::chain::balances(&client, addr, neko_tron::usdt_address()).await {
        Ok(rows) => {
            for (sym, bal) in rows {
                println!("  {sym:<6} {bal:>24}");
            }
        }
        Err(e) => println!("  failed: {e}"),
    }
}
