//! The Solana path, end to end, against mainnet.
//!
//! Everything the wallet does on this chain except signing: derive, read
//! balances, price the coin, and read history - through the same client the
//! interface uses, not a shortcut around it.

#[tokio::main]
async fn main() {
    let seed = [42u8; 64];
    let addr = neko_hd::solana::address_at(&seed, 0).unwrap();
    println!("derived    {}  ({})", addr, neko_hd::solana::path_string(0));

    let client = neko_tui::chain::Client::for_chain(neko_core::ChainId::Solana, None, None);
    let mine = neko_core::ChainAddress::Solana(addr);

    match neko_tui::chain::wallet_assets(&client, mine).await {
        Ok(rows) => {
            for (sym, dec, amt) in rows {
                println!(
                    "  {sym:<5} {:>22}",
                    neko_core::Amount::new(amt, dec)
                        .to_display_string_trim(neko_tui::chain::BALANCE_FRAC)
                );
            }
        }
        Err(e) => println!("  balances failed: {e}"),
    }

    match neko_tui::chain::native_price(&client).await {
        Ok(p) => println!(
            "1 SOL   =  {} USDT",
            neko_core::Amount::new(p, neko_core::PRICE_SCALE).to_display_string_trim(4)
        ),
        Err(e) => println!("price failed: {e}"),
    }

    // A real address with real activity, so history has something to parse.
    let busy = neko_core::ChainAddress::Solana(
        neko_hd::SolanaAddress::parse("9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM").unwrap(),
    );
    println!("\nhistory (25 signatures asked for):");
    match neko_tui::chain::history(&client, busy, 25).await {
        Ok(rows) => {
            if rows.is_empty() {
                println!("  (none parsed)");
            }
            for e in rows.iter().take(25) {
                println!(
                    "  {} {:<5} {:>18}  {}  {}",
                    match e.direction {
                        neko_tron::Direction::In => "in ",
                        neko_tron::Direction::Out => "out",
                    },
                    e.symbol,
                    neko_core::Amount::new(e.amount, e.decimals).to_display_string_trim(6),
                    &e.txid[..16.min(e.txid.len())],
                    if e.counterparty.is_empty() {
                        "(several)".to_string()
                    } else {
                        e.counterparty[..12.min(e.counterparty.len())].to_string()
                    }
                );
            }
        }
        Err(e) => println!("  history failed: {e}"),
    }
}
