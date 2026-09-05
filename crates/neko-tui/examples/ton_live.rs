//! The TON path, end to end, against mainnet.
//!
//! Balances, price and history come back through the same functions the screens
//! call. The transfer is quoted through `chain::quote` and then handed to the
//! node's own fee estimator, which has to parse the message and run the
//! contract to price it - so a message it prices is one the chain understood.

use neko_core::{Asset, ChainAddress, ChainId, NewWalletSpec, TransferRequest, VaultFile};

const PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

#[tokio::main]
async fn main() {
    let seed = neko_hd::derive::seed_from_mnemonic(PHRASE, "").unwrap();
    let key = neko_hd::ton::private_key_at(&seed, 0).unwrap();
    let pk = neko_hd::ton::public_key(&key);
    let _ = &seed;
    let addr = neko_ton::wallet::address_for(&pk).unwrap();
    println!("derived    {addr}   (m/44'/607'/0')");
    println!("           the address is the hash of the contract holding this key");

    let api_key = std::env::var("TONCENTER_API_KEY").ok();
    let client = neko_tui::chain::Client::for_chain(ChainId::Ton, None, api_key.clone());
    let node = neko_ton::client::Toncenter::new(None, api_key);

    // A throwaway vault holding the same phrase, so signing goes through the
    // real path rather than a copy of it here.
    let dir = tempfile::tempdir().unwrap();
    let mut session = VaultFile::at(dir.path().join("live.db"))
        .create(
            "live@example.com",
            "correct horse battery staple xyzzy",
            neko_vault::profile::LIGHT,
        )
        .unwrap();
    session
        .create_wallet(
            "live",
            NewWalletSpec::ImportMnemonic {
                phrase: PHRASE,
                passphrase: None,
            },
        )
        .unwrap();
    let wallet_id = session.list_wallets().unwrap()[0].id;
    println!("endpoint   {}", client.endpoint().unwrap_or("-"));
    let mine = ChainAddress::Ton(addr);

    match neko_tui::chain::wallet_assets(&client, mine).await {
        Ok(rows) => {
            println!("\nbalances:");
            for (sym, dec, amt) in rows {
                println!(
                    "  {sym:<5} {:>22}   ({dec} decimals)",
                    neko_core::Amount::new(amt, dec)
                        .to_display_string_trim(neko_tui::chain::BALANCE_FRAC)
                );
            }
        }
        Err(e) => println!("\nbalances failed: {e}"),
    }

    match neko_tui::chain::native_price(&client).await {
        Ok(p) => println!(
            "\n1 GRAM = {} USDT",
            neko_core::Amount::new(p, neko_core::PRICE_SCALE).to_display_string_trim(4)
        ),
        Err(e) => println!("\nprice failed: {e}"),
    }

    match neko_tui::chain::history(&client, mine, 5).await {
        Ok(rows) => {
            println!("\nhistory ({} rows):", rows.len());
            for r in rows.iter().take(5) {
                println!(
                    "  {:?} {:>18} {} <-> {}",
                    r.direction,
                    neko_core::Amount::new(r.amount, r.decimals).to_display_string_trim(9),
                    r.symbol,
                    &r.counterparty[..r.counterparty.len().min(20)]
                );
            }
        }
        Err(e) => println!("\nhistory failed: {e}"),
    }

    // A quote through the real path, for each asset, then the node's own
    // verdict on the message that quote produces.
    let to =
        neko_ton::TonAddress::parse("EQDVJucJT96vGh_bYm3e5uzenasiTOwA9orUHQiyhNsKmEcK").unwrap();
    for (label, asset, raw, decimals) in [
        ("GRAM", Asset::Gram, 1_000_000i128, 9u8),
        (
            "USDT",
            ChainId::Ton.usdt().unwrap(),
            1_000_000,
            neko_ton::USDT_DECIMALS,
        ),
    ] {
        let req = TransferRequest {
            wallet_id,
            from: mine,
            to: ChainAddress::Ton(to),
            asset,
            amount: neko_core::Amount::new(raw, decimals),
        };
        println!("\n--- {label} ---");
        let quote = match neko_tui::chain::quote(&client, &req).await {
            Ok(q) => q,
            Err(e) => {
                println!("  quote failed: {e}");
                continue;
            }
        };
        let neko_tui::event::Quote::Ton {
            ref params,
            fee,
            attached,
            gram_balance,
            ..
        } = quote
        else {
            println!("  quote came back for the wrong chain");
            continue;
        };
        println!(
            "  seqno {}   deploy {}   jetton wallet {}",
            params.seqno,
            params.deploy,
            params
                .jetton_wallet
                .map(|(_, w)| w.to_string())
                .unwrap_or_else(|| "-".into())
        );
        println!(
            "  fee {} GRAM   attached {} GRAM   balance {}",
            neko_core::Amount::new(fee as i128, 9).to_display_string_trim(9),
            neko_core::Amount::new(attached as i128, 9).to_display_string_trim(9),
            gram_balance
                .map(|b| neko_core::Amount::new(b as i128, 9).to_display_string_trim(9))
                .unwrap_or_else(|| "unknown".into())
        );

        // Signed through the vault, exactly as the app does it - not by
        // rebuilding the message here, which would let this pass while the
        // real path was wrong.
        let signed = match session.sign_transfer(&req, &quote.tx_params()) {
            Ok(s) => s,
            Err(e) => {
                println!("  signing failed: {e}");
                continue;
            }
        };
        println!("  message    {}", signed.id);
        println!("  {} bytes on the wire", signed.raw.len());

        // The node prices it, which means parsing the message and running the
        // contract. A message it prices is one the chain understood.
        let ext = neko_ton::boc::parse(&signed.raw).unwrap();
        let body = ext.refs().last().expect("an external message has a body");
        let init = (ext.refs().len() > 1).then(|| ext.refs()[0].clone());
        match node.estimate_fee(&addr, body, init.as_ref()).await {
            Ok(f) => println!(
                "  node priced it: {f} nanoton ({} GRAM)",
                neko_core::Amount::new(f as i128, 9).to_display_string_trim(9)
            ),
            Err(e) => println!("  the node refused to price it: {e}"),
        }
    }
}
