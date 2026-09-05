//! The Optimism path, end to end, against mainnet.
//!
//! The figure to watch is the L1 fee. This chain charges for posting to
//! Ethereum *beside* L2 gas, so `gas x price` is not the whole cost - unlike
//! Arbitrum, which folds the same charge into the gas number. Both are
//! printed below so the difference is visible rather than asserted.

use neko_core::{ChainAddress, ChainId, TransferRequest};

#[tokio::main]
async fn main() {
    let seed = neko_hd::derive::seed_from_mnemonic(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        "",
    )
    .unwrap();
    let addr = match std::env::args().nth(1) {
        Some(a) => neko_hd::EvmAddress::parse(&a).unwrap(),
        None => neko_hd::derive::evm_address_at(&seed, 0, 0).unwrap(),
    };
    println!("derived    {addr}   (m/44'/60'/0'/0/0)");
    println!(
        "           the same address as Ethereum and BNB Chain: one coin type for all three\n"
    );

    let client = neko_tui::chain::Client::for_chain(ChainId::Optimism, None, None);
    assert_eq!(
        client.chain(),
        ChainId::Optimism,
        "the client lost its chain"
    );
    let mine = ChainAddress::parse(ChainId::Optimism, &addr.to_string()).unwrap();

    match neko_tui::chain::wallet_assets(&client, mine).await {
        Ok(rows) => {
            for (sym, dec, amt) in rows {
                println!(
                    "  {sym:<5} {:>24}   ({dec} decimals)",
                    neko_core::Amount::new(amt, dec)
                        .to_display_string_trim(neko_tui::chain::BALANCE_FRAC)
                );
            }
        }
        Err(e) => println!("  balances failed: {e}"),
    }

    match neko_tui::chain::native_price(&client).await {
        Ok(p) => println!(
            "\n1 ETH = {} USDT",
            neko_core::Amount::new(p, neko_core::PRICE_SCALE).to_display_string_trim(6)
        ),
        Err(e) => println!("\nprice failed: {e}"),
    }

    // Blockscout, on its own domain rather than under blockscout.com.
    // NodeReal answers RPC for this chain but has no index behind it.
    match neko_tui::chain::history(&client, mine, 5).await {
        Ok(rows) => {
            println!("\nhistory: {} rows", rows.len());
            for r in &rows {
                println!(
                    "  {:?} {:>6} {:>18}  {}  {}",
                    r.direction,
                    r.symbol,
                    neko_core::Amount::new(r.amount, r.decimals).to_display_string_trim(8),
                    &r.counterparty[..r.counterparty.len().min(14)],
                    r.block_ts
                );
            }
        }
        Err(e) => println!("\nhistory: {e}"),
    }

    // What a transfer would cost, through the real quote path, for both assets.
    let to = ChainAddress::parse(
        ChainId::Optimism,
        "0x742d35Cc6634C0532925a3b844Bc454e4438f44e",
    )
    .unwrap();
    for (label, asset, raw, decimals) in [
        (
            "ETH",
            ChainId::Optimism.native(),
            1_000_000_000_000_000i128,
            18u8,
        ),
        ("USDT", ChainId::Optimism.stable().unwrap(), 1_000_000, 6),
    ] {
        let req = TransferRequest {
            wallet_id: 1,
            from: mine,
            to,
            asset,
            amount: neko_core::Amount::new(raw, decimals),
        };
        print!("\n--- {label} --- ");
        match neko_tui::chain::quote(&client, &req).await {
            Ok(neko_tui::event::Quote::Evm {
                chain,
                params,
                l1_fee,
                sending_native,
                ..
            }) => {
                let expected = params.gas_limit as u128 * params.fees.expected_per_gas();
                let ceiling = params.gas_limit as u128 * params.fees.max_per_gas();
                println!(
                    "chain id {}  gas {}  type {}",
                    params.chain_id,
                    params.gas_limit,
                    if params.fees.is_eip1559() { 2 } else { 0 }
                );
                println!(
                    "  expected {} ETH   ceiling {} ETH",
                    neko_core::Amount::new(expected as i128, chain.native_decimals)
                        .to_display_string_trim(10),
                    neko_core::Amount::new(ceiling as i128, chain.native_decimals)
                        .to_display_string_trim(10)
                );
                // The part `gas x price` does not cover. Read from the
                // chain's own oracle during the quote. On Arbitrum this same
                // line prints zero, because there is no oracle to ask and the
                // cost is already inside the gas figure above.
                println!(
                    "  L1 fee   {} ETH   <- charged beside gas, not in it",
                    neko_core::Amount::new(l1_fee as i128, chain.native_decimals)
                        .to_display_string_trim(12)
                );
                // The flag that decides whether the amount is counted against
                // the coin balance alongside the fee. It read `false` for this
                // chain's ether until `is_native` stopped being a hand-written
                // list, so it is worth printing rather than trusting.
                println!("  sending the coin itself: {sending_native}");
            }
            Ok(_) => println!("the quote came back for the wrong chain"),
            Err(e) => println!("quote failed: {e}"),
        }
    }
}
