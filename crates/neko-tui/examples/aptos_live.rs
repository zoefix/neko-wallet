//! The Aptos path, end to end against mainnet.

use neko_core::{ChainAddress, ChainId, TransferRequest};

#[tokio::main]
async fn main() {
    let who = std::env::args().nth(1).unwrap_or_else(|| {
        "0xbf40935e0e6f0515cddf3e884ea7ebf1488f1292e0313f080c88102d8e0564f4".into()
    });
    let mine = ChainAddress::parse(ChainId::Aptos, &who).unwrap();
    println!("address {mine}");

    let client = neko_tui::chain::Client::for_chain(ChainId::Aptos, None, None);
    assert_eq!(client.chain(), ChainId::Aptos);

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
        Ok(p) => println!("  1 APT = {p}"),
        Err(e) => println!("  price: {e}"),
    }

    match neko_tui::chain::history(&client, mine, 4).await {
        Ok(rows) => {
            println!("  history: {} rows", rows.len());
            for r in &rows {
                println!(
                    "    {:?} {:>6} {:>18}  version {}",
                    r.direction,
                    r.symbol,
                    neko_core::Amount::new(r.amount, r.decimals).to_display_string_trim(8),
                    r.txid
                );
            }
        }
        Err(e) => println!("  history: {e}"),
    }

    let to = ChainAddress::parse(
        ChainId::Aptos,
        "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    )
    .unwrap();
    for asset in ChainId::Aptos.assets() {
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
            Ok(neko_tui::event::Quote::Aptos {
                params,
                apt_balance,
                sending_native,
                ..
            }) => println!(
                "seq {}  price {} octas  ceiling {} units  native={}  balance={:?}",
                params.sequence_number,
                params.gas_unit_price,
                params.max_gas_amount,
                sending_native,
                apt_balance
            ),
            Ok(_) => println!("the quote came back for the wrong chain"),
            Err(e) => println!("failed: {e}"),
        }
    }
}
