//! What the history screen will show for one TON address.

use neko_core::{ChainAddress, ChainId};

#[tokio::main]
async fn main() {
    let who = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "EQAr-jlxrYwVBJIBJ7hoJYbcK7t0SA3_0ubMuNC4iY7Ng6Ur".into());
    let addr = neko_ton::TonAddress::parse(&who).unwrap();
    let client = ChainId::Ton;
    let client =
        neko_tui::chain::Client::for_chain(client, None, std::env::var("TONCENTER_API_KEY").ok());
    println!("address  {addr}\n");

    match neko_tui::chain::wallet_assets(&client, ChainAddress::Ton(addr)).await {
        Ok(rows) => {
            for (sym, dec, amt) in rows {
                println!(
                    "  balance {sym:<5} {}",
                    neko_core::Amount::new(amt, dec).to_display_string_trim(9)
                );
            }
        }
        Err(e) => println!("  balances failed: {e}"),
    }

    match neko_tui::chain::history(&client, ChainAddress::Ton(addr), 25).await {
        Ok(rows) => {
            println!("\n{} rows:", rows.len());
            for r in &rows {
                println!(
                    "  {:?}  {:>6} {:>16}   {}",
                    r.direction,
                    r.symbol,
                    neko_core::Amount::new(r.amount, r.decimals).to_display_string_trim(9),
                    &r.counterparty[..r.counterparty.len().min(48)]
                );
            }
        }
        Err(e) => println!("\nhistory failed: {e}"),
    }
}
