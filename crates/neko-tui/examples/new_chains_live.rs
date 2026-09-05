//! Every chain added in this batch, end to end against mainnet.
//!
//! Balances, price, history and a real quote for both assets, through the same
//! code the wallet uses. Nothing is signed and nothing is broadcast.

use neko_core::{ChainAddress, ChainId, TransferRequest};

#[tokio::main]
async fn main() {
    let who = std::env::args().nth(1).unwrap_or_else(|| {
        "0xF977814e90dA44bFA03b6295A0616a897441aceC".to_string()
    });
    let addr = neko_hd::EvmAddress::parse(&who).unwrap();

    for chain in [
        ChainId::Avalanche,
        ChainId::HyperEvm,
        ChainId::Mantle,
        ChainId::Linea,
        ChainId::ZkSyncEra,
        ChainId::Scroll,
    ] {
        let evm = chain.evm().unwrap();
        println!("\n===== {} (chain id {}) =====", chain.label(), evm.chain_id);
        let client = neko_tui::chain::Client::for_chain(chain, None, None);
        assert_eq!(client.chain(), chain, "the client lost its chain");
        let mine = ChainAddress::parse(chain, &addr.to_string()).unwrap();

        match neko_tui::chain::wallet_assets(&client, mine).await {
            Ok(rows) => {
                for (sym, dec, amt) in rows {
                    println!(
                        "  {sym:<6} {:>22}  ({dec} dp)",
                        neko_core::Amount::new(amt, dec)
                            .to_display_string_trim(neko_tui::chain::BALANCE_FRAC)
                    );
                }
            }
            Err(e) => println!("  balances failed: {e}"),
        }

        match neko_tui::chain::native_price(&client).await {
            Ok(p) => println!(
                "  1 {} = {} USDT",
                evm.native_symbol,
                neko_core::Amount::new(p, neko_core::PRICE_SCALE).to_display_string_trim(6)
            ),
            Err(e) => println!("  price: {e}"),
        }

        match neko_tui::chain::history(&client, mine, 3).await {
            Ok(rows) => {
                println!("  history: {} rows", rows.len());
                for r in &rows {
                    println!(
                        "    {:?} {:>6} {:>16}  {}",
                        r.direction,
                        r.symbol,
                        neko_core::Amount::new(r.amount, r.decimals).to_display_string_trim(6),
                        r.block_ts
                    );
                }
            }
            Err(e) => println!("  history: {e}"),
        }

        let to = ChainAddress::parse(chain, "0x742d35Cc6634C0532925a3b844Bc454e4438f44e").unwrap();
        for asset in chain.assets() {
            let raw: i128 = if asset.is_native() { 1_000_000_000_000 } else { 1_000 };
            let req = TransferRequest {
                wallet_id: 1,
                from: mine,
                to,
                asset,
                amount: neko_core::Amount::new(raw, asset.decimals()),
            };
            print!("  quote {:<6} ", asset.symbol());
            match neko_tui::chain::quote(&client, &req).await {
                Ok(neko_tui::event::Quote::Evm {
                    params, l1_fee, sending_native, ..
                }) => println!(
                    "gas {:<8} type {}  L1 {:<14} native={}",
                    params.gas_limit,
                    if params.fees.is_eip1559() { 2 } else { 0 },
                    l1_fee,
                    sending_native
                ),
                Ok(_) => println!("wrong chain came back"),
                Err(e) => println!("failed: {e}"),
            }
        }
    }
}
