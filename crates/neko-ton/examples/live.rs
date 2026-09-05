//! Build a real message and have a node validate it.
//!
//! There is no `simulateTransaction` here, but `estimateFee` has to parse the
//! message and run the contract to price it - so a malformed one fails, and a
//! priced one is one the chain understood.

use std::sync::Arc;
use zeroize::Zeroizing;

use neko_ton::{address::TonAddress, client::Toncenter, message, wallet};

#[tokio::main]
async fn main() {
    let api = Toncenter::new(std::env::var("TON_API").ok().as_deref(), None);
    println!("endpoint  {}", api.endpoint());

    // Our own wallet, from the phrase every BIP publishes vectors for.
    let seed = neko_hd::derive::seed_from_mnemonic(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        "",
    )
    .unwrap();
    let sk = neko_hd::ton::private_key_at(&seed, 0).unwrap();
    let pk = neko_hd::ton::public_key(&sk);
    let mine = wallet::address_for(&pk).unwrap();
    println!("derived   {}  ({})", mine, neko_hd::ton::path_string(0));
    println!("raw       {}", mine.to_raw_string());

    match api.wallet_state(&mine).await {
        Ok(s) => println!(
            "state     balance {} nanoton  seqno {}  deployed {}",
            s.balance, s.seqno, s.deployed
        ),
        Err(e) => println!("state     failed: {e}"),
    }

    // A jetton wallet address, resolved by the token's own contract.
    let master = neko_ton::usdt_master();
    match api.jetton_wallet(&mine, &master).await {
        Ok(w) => {
            println!("USDT wallet {}", w.to_raw_string());
            match api.jetton_balance(&w).await {
                Ok(b) => println!("USDT balance {b}"),
                Err(e) => println!("USDT balance failed: {e}"),
            }
        }
        Err(e) => println!("jetton wallet failed: {e}"),
    }

    // Now the encoding check, against a wallet that exists so the node has a
    // contract to run.
    let real =
        TonAddress::parse("0:5661bcb42ba847235760ce9aaa2dfff103eb7365db06e5df053120bacb77ddfd")
            .unwrap();
    let seqno = match api.seqno(&real).await {
        Ok(n) => n,
        Err(e) => {
            println!("\nseqno failed: {e}");
            return;
        }
    };
    let to = TonAddress::parse("EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs").unwrap();
    let inner = message::internal_message(&to, 1_000_000, true, None).unwrap();
    let body = message::signing_body(
        wallet::DEFAULT_SUBWALLET_ID,
        now() + message::VALID_FOR_SECS,
        seqno,
        message::MODE_ORDINARY,
        inner,
    )
    .unwrap();
    // The signature is not checked by `estimateFee`; the encoding is.
    let signed = message::signed_body(body, &Zeroizing::new([1u8; 32])).unwrap();

    println!("\nmessage for a live wallet (seqno {seqno})");
    println!(
        "  body cells  {} bits, {} refs",
        signed.bits(),
        signed.refs().len()
    );
    println!(
        "  boc bytes   {}",
        neko_ton::boc::serialize(&signed).unwrap().len()
    );
    match api.estimate_fee(&real, &signed, None).await {
        Ok(f) => println!(
            "  node priced it: {f} nanoton  ({} GRAM)",
            neko_core_fmt(f, 9)
        ),
        Err(e) => println!("  node REJECTED it: {e}"),
    }

    // A jetton transfer, which is a message to our own jetton wallet.
    let jw = api.jetton_wallet(&real, &master).await;
    if let Ok(jw) = jw {
        let body =
            neko_ton::jetton::transfer_body(1_000_000, &to, &real, neko_ton::JETTON_FORWARD_AMOUNT)
                .unwrap();
        let inner =
            message::internal_message(&jw, neko_ton::JETTON_TRANSFER_ATTACHED, true, Some(body))
                .unwrap();
        let b = message::signing_body(
            wallet::DEFAULT_SUBWALLET_ID,
            now() + message::VALID_FOR_SECS,
            seqno,
            message::MODE_ORDINARY,
            inner,
        )
        .unwrap();
        let signed = message::signed_body(b, &Zeroizing::new([1u8; 32])).unwrap();
        println!("\nUSDT transfer message");
        println!("  to jetton wallet {}", jw.to_raw_string());
        match api.estimate_fee(&real, &signed, None).await {
            Ok(f) => println!(
                "  node priced it: {f} nanoton ({} GRAM)",
                neko_core_fmt(f, 9)
            ),
            Err(e) => println!("  node REJECTED it: {e}"),
        }
    }

    // And a deploying message, which is what an unused wallet's first transfer
    // looks like: the contract travels with it.
    let init: Arc<neko_ton::cell::Cell> = wallet::state_init(
        wallet::code().unwrap(),
        wallet::initial_data(&pk, wallet::DEFAULT_SUBWALLET_ID).unwrap(),
    )
    .unwrap();
    let body2 = message::signing_body(
        wallet::DEFAULT_SUBWALLET_ID,
        now() + message::VALID_FOR_SECS,
        0,
        message::MODE_ORDINARY,
        message::internal_message(&to, 1_000_000, false, None).unwrap(),
    )
    .unwrap();
    let signed2 = message::signed_body(body2, &sk).unwrap();
    let ext = message::external_message(&mine, Some(init.clone()), signed2.clone()).unwrap();
    println!("\ndeploying message for our own (undeployed) wallet");
    println!(
        "  external    {} bits, {} refs",
        ext.bits(),
        ext.refs().len()
    );
    println!(
        "  boc bytes   {}",
        neko_ton::boc::serialize(&ext).unwrap().len()
    );
    match api.estimate_fee(&mine, &signed2, Some(&init)).await {
        Ok(f) => println!(
            "  node priced it: {f} nanoton ({} GRAM)",
            neko_core_fmt(f, 9)
        ),
        Err(e) => println!("  node REJECTED it: {e}"),
    }
}

fn now() -> u32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as u32
}

fn neko_core_fmt(v: u128, d: u32) -> String {
    let s = 10u128.pow(d);
    format!("{}.{:0width$}", v / s, v % s, width = d as usize)
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}
