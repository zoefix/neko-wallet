//! Quotes both chains and prices a sample portfolio, using the same code the
//! wallet list uses.
//!
//! Run: cargo run -p neko-tui --example valuation
#[tokio::main]
async fn main() {
    let mut prices = neko_core::Prices::default();
    for chain in neko_core::CHAINS {
        let c = neko_tui::chain::Client::for_chain(chain, None, None);
        match neko_tui::chain::native_price(&c).await {
            Ok(p) => {
                println!(
                    "1 {:<4} = {:>12} USDT",
                    chain.native_symbol(),
                    neko_core::Amount::new(p, neko_core::PRICE_SCALE).to_display_string()
                );
                prices.set_native(chain, p, 0);
            }
            Err(e) => println!("{}: {e}", chain.native_symbol()),
        }
    }
    let holdings = [
        (neko_core::ChainId::Tron, "TRX", 8_655_008i128, 6u8),
        (neko_core::ChainId::Tron, "USDT", 15_880_000, 6),
        (neko_core::ChainId::Bsc, "BNB", 50_000_000_000_000_000, 18),
        (neko_core::ChainId::Bsc, "USDT", 0, 18),
    ];
    println!("\n8.655008 TRX + 15.88 USDT + 0.05 BNB:");
    match neko_core::value::total(holdings, &prices) {
        Some(v) => println!("  = {} USDT", v.to_display_string_max(2)),
        None => println!("  = ? (a holding could not be priced)"),
    }
}
