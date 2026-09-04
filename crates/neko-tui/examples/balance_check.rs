//! Calls the exact function the assets screen calls, so a regression in the
//! display path shows up here rather than as a blank column in the UI.
//!
//! Run: cargo run --release -p neko-tui --example balance_check

use neko_hd::derive;

const LEDGER_PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let seed = derive::seed_from_mnemonic(LEDGER_PHRASE, "").unwrap();
    let addr = neko_core::ChainAddress::Tron(derive::address_at(&seed, 0, 0).unwrap());

    println!("network mainnet   address {addr}");
    let client = neko_tui::chain::Client::for_chain(
        neko_core::ChainId::Tron,
        None,
        std::env::var("TRONGRID_API_KEY").ok(),
    );

    match neko_tui::chain::wallet_assets(&client, addr).await {
        Ok(rows) => {
            for (sym, dec, amt) in rows {
                let bal = neko_core::Amount::new(amt, dec)
                    .to_display_string_trim(neko_tui::chain::BALANCE_FRAC);
                println!("  {sym:<6} {bal:>24}");
            }
        }
        Err(e) => println!("  failed: {e}"),
    }
}
