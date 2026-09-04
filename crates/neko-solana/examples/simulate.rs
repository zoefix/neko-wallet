//! Ask the cluster to decode and run a transaction this crate built.
//!
//! The wire format has no field names: a byte in the wrong place yields a
//! *different* valid transaction rather than a parse error, so no amount of
//! reading the code proves the encoding. Simulation does - the cluster decodes
//! the bytes, resolves the accounts and executes the instruction, and reports
//! what it saw.

use neko_hd::SolanaAddress;
use neko_solana::{client::Rpc, tx};

#[tokio::main]
async fn main() {
    let url = std::env::var("SOLANA_RPC").ok();
    let rpc = Rpc::new(url.as_deref());

    // A funded mainnet account, used only as a source address in a simulation
    // that is never signed or sent.
    let from = SolanaAddress::parse("5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9").unwrap();
    let to = SolanaAddress::parse("9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM").unwrap();

    let blockhash = match rpc.latest_blockhash().await {
        Ok(b) => b,
        Err(e) => {
            println!("could not get a blockhash: {e}");
            return;
        }
    };
    println!(
        "blockhash        {}",
        bs58::encode(blockhash.hash).into_string()
    );

    for (label, ixs) in [
        (
            "1. plain SOL transfer",
            vec![tx::transfer_sol(from, to, 1_000_000)],
        ),
        (
            "2. SOL transfer with a compute budget",
            vec![
                tx::set_compute_unit_limit(neko_solana::COMPUTE_UNITS_SOL),
                tx::set_compute_unit_price(1_000),
                tx::transfer_sol(from, to, 1_000_000),
            ],
        ),
        ("3. USDT transfer (TransferChecked)", {
            let mint = neko_solana::usdt_mint();
            let src = neko_solana::associated_token_address(&from, &mint).unwrap();
            let dst = neko_solana::associated_token_address(&to, &mint).unwrap();
            vec![
                tx::set_compute_unit_limit(neko_solana::COMPUTE_UNITS_TOKEN),
                tx::transfer_token_checked(src, mint, dst, from, 1_000_000, 6),
            ]
        }),
        ("4. USDT transfer that opens the recipient's account", {
            let mint = neko_solana::usdt_mint();
            let src = neko_solana::associated_token_address(&from, &mint).unwrap();
            let dst = neko_solana::associated_token_address(&to, &mint).unwrap();
            vec![
                tx::set_compute_unit_limit(neko_solana::COMPUTE_UNITS_TOKEN_WITH_ATA),
                tx::create_associated_token_account(from, to, mint).unwrap(),
                tx::transfer_token_checked(src, mint, dst, from, 1_000_000, 6),
            ]
        }),
    ] {
        let msg = tx::Message::compile(&from, &ixs, blockhash.hash).unwrap();
        println!("\n{label}");
        println!(
            "  header           sigs={} ro_signed={} ro_unsigned={}  accounts={}",
            msg.num_required_signatures,
            msg.num_readonly_signed,
            msg.num_readonly_unsigned,
            msg.account_keys.len()
        );
        for (i, k) in msg.account_keys.iter().enumerate() {
            println!("    [{i}] {k}");
        }
        let signed = tx::Transaction {
            signatures: vec![[0u8; 64]],
            message: msg,
        };
        let raw = signed.serialize().unwrap();
        println!("  bytes            {}", raw.len());

        match simulate(&rpc, &raw).await {
            Ok(v) => println!("  cluster says     {v}"),
            Err(e) => println!("  cluster says     ERROR {e}"),
        }
    }
}

/// `sigVerify: false` because the signature above is zeros; the point is
/// whether the cluster can decode and execute the bytes.
async fn simulate(rpc: &Rpc, raw: &[u8]) -> Result<String, String> {
    let url = std::env::var("SOLANA_RPC").unwrap_or_else(|_| neko_solana::DEFAULT_RPC.to_string());
    let _ = rpc;
    let body = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "simulateTransaction",
        "params": [b64(raw), {"encoding": "base64", "sigVerify": false, "replaceRecentBlockhash": true}]
    });
    let resp: serde_json::Value = reqwest::Client::new()
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(e) = resp.get("error") {
        return Err(e.to_string());
    }
    let v = &resp["result"]["value"];
    let err = &v["err"];
    let mut out = format!(
        "err={}  units={}",
        if err.is_null() {
            "none".into()
        } else {
            err.to_string()
        },
        v["unitsConsumed"]
    );
    if let Some(logs) = v["logs"].as_array() {
        for l in logs {
            out.push_str("\n      | ");
            out.push_str(l.as_str().unwrap_or(""));
        }
    }
    Ok(out)
}

fn b64(input: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in input.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}
