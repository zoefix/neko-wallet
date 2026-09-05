//! The Sui path, end to end against mainnet.

use neko_core::{ChainAddress, ChainId, TransferRequest};

#[tokio::main]
async fn main() {
    let who = std::env::args().nth(1).unwrap_or_else(|| {
        "0x77776760c06997206b13fa76f127aa016d24a645f04fce516be153ece0bddf23".into()
    });
    let mine = ChainAddress::parse(ChainId::Sui, &who).unwrap();
    println!("address {mine}");

    let client = neko_tui::chain::Client::for_chain(ChainId::Sui, None, None);
    assert_eq!(client.chain(), ChainId::Sui);

    match neko_tui::chain::wallet_assets(&client, mine).await {
        Ok(rows) => {
            for (sym, dec, amt) in rows {
                println!(
                    "  {sym:<5} {:>22}  ({dec} dp)",
                    neko_core::Amount::new(amt, dec)
                        .to_display_string_trim(neko_tui::chain::BALANCE_FRAC)
                );
            }
        }
        Err(e) => println!("  balances failed: {e}"),
    }
    match neko_tui::chain::native_price(&client).await {
        Ok(p) => println!("  1 SUI = {p}"),
        Err(e) => println!("  price: {e}"),
    }
    match neko_tui::chain::history(&client, mine, 4).await {
        Ok(rows) => {
            println!("  history: {} rows", rows.len());
            for r in &rows {
                println!(
                    "    {:?} {:>5} {:>16}  {}",
                    r.direction,
                    r.symbol,
                    neko_core::Amount::new(r.amount, r.decimals).to_display_string_trim(9),
                    r.block_ts
                );
            }
        }
        Err(e) => println!("  history: {e}"),
    }

    let to = ChainAddress::parse(
        ChainId::Sui,
        "0x0000000000000000000000000000000000000000000000000000000000000002",
    )
    .unwrap();
    for asset in ChainId::Sui.assets() {
        let raw: i128 = if asset.is_native() { 1_000_000 } else { 1_000 };
        let req = TransferRequest {
            wallet_id: 1,
            from: mine,
            to,
            asset,
            amount: neko_core::Amount::new(raw, asset.decimals()),
        };
        print!("  quote {:<5} ", asset.symbol());
        match neko_tui::chain::quote(&client, &req).await {
            Ok(neko_tui::event::Quote::Sui {
                fee,
                coins_spent,
                sui_balance,
                sending_native,
                params,
                ..
            }) => println!(
                "fee {fee} MIST after rebate  {coins_spent} coin objects  budget {}  native={sending_native}  balance={sui_balance:?}",
                params.budget
            ),
            Ok(_) => println!("the quote came back for the wrong chain"),
            Err(e) => println!("failed: {e}"),
        }
    }
}
