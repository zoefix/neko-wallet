//! The Bitcoin path, end to end, against mainnet.
//!
//! Everything the wallet does on this chain except broadcasting: derive, read
//! coins, estimate a fee, select, build and sign a real transaction, and read
//! history - through the same client the interface uses.

#[tokio::main]
async fn main() {
    let seed = neko_hd::derive::seed_from_mnemonic(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        "",
    )
    .unwrap();
    let addr = neko_hd::bitcoin::address_at(&seed, 0, 0, 0).unwrap();
    println!("derived    {addr}   (m/84'/0'/0'/0/0)");

    let client = neko_tui::chain::Client::for_chain(neko_core::ChainId::Bitcoin, None, None);
    let mine = neko_core::ChainAddress::Bitcoin(addr);
    println!("endpoint   {}", client.endpoint().unwrap_or("?"));

    match neko_tui::chain::wallet_assets(&client, mine).await {
        Ok(rows) => {
            for (sym, dec, amt) in rows {
                println!(
                    "  {sym:<5} {:>20}",
                    neko_core::Amount::new(amt, dec)
                        .to_display_string_trim(neko_tui::chain::BALANCE_FRAC)
                );
            }
        }
        Err(e) => println!("  balances failed: {e}"),
    }

    match neko_tui::chain::native_price(&client).await {
        Ok(p) => println!(
            "1 BTC   =  {} USDT  (quoted from BTCB on BNB Chain)",
            neko_core::Amount::new(p, neko_core::PRICE_SCALE).to_display_string_trim(2)
        ),
        Err(e) => println!("price failed: {e}"),
    }

    // Coin selection and a real signature, against this address's actual coins.
    let esplora = neko_btc::client::Esplora::new(None);
    let utxos = esplora.utxos(addr).await.unwrap_or_default();
    let rate = esplora.fee_rate(neko_btc::TARGET_BLOCKS).await.unwrap_or(1);
    println!("\nfee rate   {rate} sat/vB   coins held: {}", utxos.len());

    if utxos.is_empty() {
        println!("  (no coins at this address, so nothing to select)");
    } else {
        let to = neko_hd::BtcAddress::parse("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").unwrap();
        let total: u64 = utxos.iter().map(|u| u.value).sum();
        let amount = total / 2;
        match neko_btc::coins::select(&utxos, &to, amount, &addr, rate) {
            Ok(sel) => {
                println!(
                    "  spending {} of {} coins, fee {} sat, change {:?}",
                    sel.inputs.len(),
                    utxos.len(),
                    sel.fee,
                    sel.change
                );
                let key = neko_hd::bitcoin::private_key_at(&seed, 0, 0, 0).unwrap();
                let mut outs = vec![neko_btc::tx::output(&to, amount)];
                if let Some(c) = sel.change {
                    outs.push(neko_btc::tx::output(&addr, c));
                }
                let mut t = neko_btc::tx::Tx {
                    version: neko_btc::tx::VERSION,
                    inputs: sel.inputs.iter().map(neko_btc::tx::input).collect(),
                    outputs: outs,
                    locktime: 0,
                };
                match neko_btc::tx::sign_p2wpkh(&mut t, &sel.inputs, &key) {
                    Ok(()) => {
                        println!("  signed     txid {}", t.txid());
                        println!(
                            "  size       {} bytes, {} vB",
                            t.serialize().len(),
                            t.vsize()
                        );
                        println!(
                            "  fee check  inputs-outputs = {:?}",
                            t.fee(sel.input_total())
                        );
                        println!("  NOT broadcast.");
                    }
                    Err(e) => println!("  signing failed: {e}"),
                }
            }
            Err(e) => println!("  selection: {e}"),
        }
    }

    println!("\nhistory:");
    match neko_tui::chain::history(&client, mine, 8).await {
        Ok(rows) if rows.is_empty() => println!("  (none)"),
        Ok(rows) => {
            for e in rows.iter().take(8) {
                println!(
                    "  {} {:>16} BTC  {}  {}",
                    match e.direction {
                        neko_tron::Direction::In => "in ",
                        neko_tron::Direction::Out => "out",
                    },
                    neko_core::Amount::new(e.amount, e.decimals).to_display_string_trim(8),
                    &e.txid[..16.min(e.txid.len())],
                    if e.counterparty.is_empty() {
                        // Nothing was paid to anybody: a consolidation, or a
                        // spend whose whole value went to the miner.
                        "(no recipient)".to_string()
                    } else {
                        e.counterparty[..14.min(e.counterparty.len())].to_string()
                    }
                );
            }
        }
        Err(e) => println!("  history failed: {e}"),
    }
}
