//! Runs the exact path the history screen runs, so a regression shows up here
//! rather than as an empty list in the UI.
//!
//! Run: NODEREAL_API_KEY=... cargo run -p neko-tui --example bsc_history
#[tokio::main]
async fn main() {
    let key = std::env::var("NODEREAL_API_KEY").ok();
    let has = key.is_some();
    let client = neko_tui::chain::Client::for_chain(neko_core::ChainId::Bsc, None, key);
    let addr = neko_core::ChainAddress::parse(
        neko_core::ChainId::Bsc,
        "0x9858EfFD232B4033E47d90003D41EC34EcaEda94",
    )
    .unwrap();
    println!("key configured: {has}");
    match neko_tui::chain::history(&client, addr, 6).await {
        Ok(rows) => {
            println!("{} entries", rows.len());
            for r in &rows {
                println!(
                    "  {:?} {:>24} {:<5} dust={} {}",
                    r.direction,
                    neko_core::Amount::new(r.amount, r.decimals).to_display_string(),
                    r.symbol,
                    r.is_dust(),
                    &r.counterparty[..12]
                );
            }
        }
        Err(e) => println!("refused: {e}"),
    }
}
