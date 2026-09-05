//! Compare this wallet's BCS against the node's own encoder.
//!
//! Aptos exposes `/transactions/encode_submission`, which takes a transaction
//! as JSON and returns the exact bytes a signature must cover. Two independent
//! encoders agreeing on every byte is the only way to be sure this file is
//! right - BCS carries no types, so a wrong encoding is not malformed, it is a
//! different transaction that would be signed just as happily.

use neko_aptos::tx::{self, RawTransaction, TxParams};
use neko_aptos::AptosAddress;

#[tokio::main]
async fn main() {
    let api = neko_aptos::DEFAULT_API;
    let http = reqwest::Client::new();

    let sender = AptosAddress::parse(
        "0xeb663b681209e7087d681c5d3eed12aaa8e1915e7c87794542c3f96e94b3d3bf",
    )
    .unwrap();
    let to =
        AptosAddress::parse("0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
            .unwrap();

    let params = TxParams {
        sequence_number: 11,
        max_gas_amount: 2_000,
        gas_unit_price: 100,
        expiration_timestamp_secs: 1_900_000_000,
        chain_id: neko_aptos::CHAIN_ID,
    };

    for (label, payload, json_payload) in [
        (
            "APT transfer",
            tx::transfer_apt(to, 12_345_678),
            serde_json::json!({
                "type": "entry_function_payload",
                "function": "0x1::aptos_account::transfer",
                "type_arguments": [],
                "arguments": [to.to_string(), "12345678"],
            }),
        ),
        (
            "USDT (fungible asset) transfer",
            tx::transfer_fungible_asset(neko_aptos::usdt_metadata(), to, 1_000_000),
            serde_json::json!({
                "type": "entry_function_payload",
                "function": "0x1::primary_fungible_store::transfer",
                "type_arguments": ["0x1::fungible_asset::Metadata"],
                "arguments": [neko_aptos::USDT_METADATA, to.to_string(), "1000000"],
            }),
        ),
    ] {
        let raw = RawTransaction {
            sender,
            payload,
            params,
        };
        let ours = raw.signing_message();

        let body = serde_json::json!({
            "sender": sender.to_string(),
            "sequence_number": params.sequence_number.to_string(),
            "max_gas_amount": params.max_gas_amount.to_string(),
            "gas_unit_price": params.gas_unit_price.to_string(),
            "expiration_timestamp_secs": params.expiration_timestamp_secs.to_string(),
            "payload": json_payload,
        });
        let resp = http
            .post(format!("{api}/transactions/encode_submission"))
            .json(&body)
            .send()
            .await
            .unwrap();
        let text = resp.text().await.unwrap();
        let theirs_hex = text.trim().trim_matches('"');
        println!("--- {label} ---");
        if !theirs_hex.starts_with("0x") {
            println!("  node said: {}", &text[..text.len().min(300)]);
            continue;
        }
        let theirs = hex::decode(&theirs_hex[2..]).unwrap();
        println!("  ours   {} bytes", ours.len());
        println!("  node   {} bytes", theirs.len());
        if ours == theirs {
            println!("  IDENTICAL");
            println!("  hex: {}", hex::encode(&ours));
        } else {
            println!("  DIFFER");
            println!("   ours: {}", hex::encode(&ours));
            println!("   node: {}", hex::encode(&theirs));
            let n = ours.iter().zip(&theirs).take_while(|(a, b)| a == b).count();
            println!("   first difference at byte {n}");
        }
    }
}
