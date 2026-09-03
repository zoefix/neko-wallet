//! Generates a replacement for the third hd.json vector.
//!
//! The original third vector was the reference project's real Nile testnet
//! mnemonic — a wallet that has actually held funds on-chain. It cannot ship in
//! a public repository. This emits a freshly generated synthetic one.
//!
//! Run: cargo run --release -p neko-hd --example regen_vector

use bip32::Prefix;
use neko_hd::derive;

fn main() {
    let mnemonic = derive::generate_mnemonic(24).unwrap();
    let entropy = derive::entropy_from_mnemonic(&mnemonic).unwrap();
    let seed = derive::seed_from_mnemonic(&mnemonic, "").unwrap();

    let master = derive::master_from_seed(&seed).unwrap();
    let account = derive::derive_xprv(&seed, "m/44'/195'/0'").unwrap();

    let addrs = |branch: u32| -> Vec<String> {
        (0..5)
            .map(|i| derive::address_at(&seed, branch, i).unwrap().to_string())
            .collect()
    };
    let keys: Vec<String> = (0..5)
        .map(|i| hex::encode(*derive::private_key_at(&seed, 0, i).unwrap()))
        .collect();

    let out = serde_json::json!({
        "mnemonic": mnemonic.to_string(),
        "entropy": hex::encode(&*entropy),
        "seed": hex::encode(&seed[..]),
        "master_xprv": master.to_string(Prefix::XPRV).to_string(),
        "hot_account_xprv": account.to_string(Prefix::XPRV).to_string(),
        "addresses": addrs(0),
        "priv_keys": keys,
        "cold_addresses": addrs(1),
    });
    println!("{}", serde_json::to_string_pretty(&out).unwrap());
}
