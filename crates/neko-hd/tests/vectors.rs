//! Conformance against `vectors/hd.json`.
//!
//! From the archive's own README: "Rust 端必须与这些逐字节吻合。差一个字节，
//! 交易被全网拒绝" — and worse, a wrong address is usually still a *valid*
//! address, so format checks cannot catch it. Funds land somewhere nobody holds
//! the key to, silently.
//!
//! The first vector is LedgerHQ's official test mnemonic, so this also pins us
//! to hardware-wallet behaviour.

use neko_hd::{address::Address, derive};
use serde::Deserialize;

const RAW: &str = include_str!("../../../vectors/hd.json");

#[derive(Deserialize)]
struct ValidAddr {
    base58: String,
    hex: String,
}

#[derive(Deserialize)]
struct AddressCodec {
    valid: Vec<ValidAddr>,
    invalid: Vec<String>,
}

#[derive(Deserialize)]
struct Vector {
    mnemonic: String,
    entropy: String,
    seed: String,
    master_xprv: String,
    hot_account_xprv: String,
    addresses: Vec<String>,
    priv_keys: Vec<String>,
    cold_addresses: Vec<String>,
}

#[derive(Deserialize)]
struct Vectors {
    #[serde(rename = "addressCodec")]
    address_codec: AddressCodec,
    path: String,
    vectors: Vec<Vector>,
}

/// The Ledger test mnemonic. Public, and the basis of the standard BIP44
/// vectors - which is exactly why it must never hold funds.
const LEDGER: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

const HOT: u32 = 0;
const COLD: u32 = 1;

fn v() -> Vectors {
    serde_json::from_str(RAW).expect("vectors/hd.json is malformed")
}
fn hx(s: &str) -> Vec<u8> {
    hex::decode(s).expect("bad hex in vectors")
}
fn seed_of(vec: &Vector) -> [u8; 64] {
    let mut s = [0u8; 64];
    s.copy_from_slice(&hx(&vec.seed));
    s
}

#[test]
fn derivation_path_matches_the_reference() {
    assert_eq!(v().path, "m/44'/195'/{branch}'/0/{index}");
    assert_eq!(derive::path_for(HOT, 0), "m/44'/195'/0'/0/0");
    assert_eq!(derive::path_for(COLD, 3), "m/44'/195'/1'/0/3");
}

/// Base58 <-> 21-byte hex must round-trip exactly.
#[test]
fn address_codec_round_trips() {
    for a in &v().address_codec.valid {
        let parsed = Address::parse(&a.base58).unwrap_or_else(|e| panic!("{}: {e}", a.base58));
        assert_eq!(parsed.to_hex(), a.hex, "hex mismatch for {}", a.base58);
        assert_eq!(parsed.to_string(), a.base58, "base58 round-trip failed");

        let from_bytes = Address::from_bytes(&hx(&a.hex)).unwrap();
        assert_eq!(from_bytes.to_string(), a.base58);
    }
}

/// Empty, truncated, non-base58, a Bitcoin address (wrong prefix), and a bad
/// checksum must all be rejected. A wrong-but-plausible address is the failure
/// mode that silently loses money.
#[test]
fn malformed_addresses_are_rejected() {
    for bad in &v().address_codec.invalid {
        assert!(
            Address::parse(bad).is_err(),
            "accepted invalid address {bad:?}"
        );
    }
}

/// The 20-byte ABI form drops the 0x41 prefix; re-adding it must be lossless.
#[test]
fn evm_form_round_trips() {
    for a in &v().address_codec.valid {
        let addr = Address::parse(&a.base58).unwrap();
        assert_eq!(Address::from_evm_bytes(&addr.to_evm_bytes()).unwrap(), addr);
        assert_eq!(
            hex::encode(addr.to_evm_bytes()),
            a.hex[2..],
            "ABI form keeps the prefix"
        );
    }
}

#[test]
fn mnemonic_to_entropy_round_trips() {
    for vec in &v().vectors {
        let e = derive::entropy_from_mnemonic(&vec.mnemonic).unwrap();
        assert_eq!(hex::encode(&*e), vec.entropy, "entropy mismatch");
        let m = derive::mnemonic_from_entropy(&hx(&vec.entropy)).unwrap();
        assert_eq!(*m, vec.mnemonic, "mnemonic mismatch");
    }
}

/// entropy -> phrase -> PBKDF2 seed. Using the entropy directly as the seed is
/// the tempting shortcut that produces a completely different wallet.
#[test]
fn bip39_seed_matches_the_reference() {
    for vec in &v().vectors {
        let seed = derive::seed_from_mnemonic(&vec.mnemonic, "").unwrap();
        assert_eq!(
            hex::encode(&seed[..]),
            vec.seed,
            "seed mismatch for {}",
            vec.mnemonic
        );
    }
}

#[test]
fn extended_keys_match_the_reference() {
    use bip32::Prefix;
    for vec in &v().vectors {
        let seed = seed_of(vec);
        let master = derive::master_from_seed(&seed).unwrap();
        assert_eq!(
            master.to_string(Prefix::XPRV).to_string(),
            vec.master_xprv,
            "master xprv"
        );

        let account = derive::derive_xprv(&seed, "m/44'/195'/0'").unwrap();
        assert_eq!(
            account.to_string(Prefix::XPRV).to_string(),
            vec.hot_account_xprv,
            "hot account xprv"
        );
    }
}

/// The one that actually matters: addresses and private keys, byte for byte.
#[test]
fn addresses_and_private_keys_match_the_reference() {
    for vec in &v().vectors {
        let seed = seed_of(vec);
        for (i, want) in vec.addresses.iter().enumerate() {
            let got = derive::address_at(&seed, HOT, i as u32).unwrap();
            assert_eq!(
                &got.to_string(),
                want,
                "hot address {i} for {}",
                vec.mnemonic
            );
        }
        for (i, want) in vec.priv_keys.iter().enumerate() {
            let got = derive::private_key_at(&seed, HOT, i as u32).unwrap();
            assert_eq!(hex::encode(*got), *want, "private key {i}");
            // The key must also produce its own address.
            let addr = derive::address_from_private_key(&got).unwrap();
            assert_eq!(
                addr.to_string(),
                vec.addresses[i],
                "key/address disagree at {i}"
            );
        }
        for (i, want) in vec.cold_addresses.iter().enumerate() {
            let got = derive::address_at(&seed, COLD, i as u32).unwrap();
            assert_eq!(&got.to_string(), want, "cold address {i}");
        }
    }
}

/// The hardened account level must actually isolate branches.
#[test]
fn hot_and_cold_branches_are_disjoint() {
    let vec = &v().vectors[0];
    let seed = seed_of(vec);
    let hot: Vec<String> = (0..5)
        .map(|i| derive::address_at(&seed, HOT, i).unwrap().to_string())
        .collect();
    let cold: Vec<String> = (0..5)
        .map(|i| derive::address_at(&seed, COLD, i).unwrap().to_string())
        .collect();
    for a in &hot {
        assert!(!cold.contains(a), "branch collision on {a}");
    }
}

/// Private keys are fixed 32 bytes with leading zeros preserved. A trimmed key
/// derives a different address.
#[test]
fn private_keys_are_fixed_width() {
    let vec = &v().vectors[0];
    let seed = seed_of(vec);
    for i in 0..5 {
        assert_eq!(derive::private_key_at(&seed, HOT, i).unwrap().len(), 32);
    }
}

#[test]
fn generated_mnemonics_are_valid_and_unique() {
    let mut seen = std::collections::HashSet::new();
    for _ in 0..20 {
        let m = derive::generate_mnemonic(12).unwrap();
        assert_eq!(m.split_whitespace().count(), 12);
        assert!(
            derive::validate_mnemonic(&m),
            "generated an invalid mnemonic: {}",
            *m
        );
        assert!(seen.insert(m.to_string()), "generated a duplicate mnemonic");
    }
    assert_eq!(
        derive::generate_mnemonic(24)
            .unwrap()
            .split_whitespace()
            .count(),
        24
    );
    assert!(
        derive::generate_mnemonic(13).is_err(),
        "only 12 and 24 words are valid"
    );
}

#[test]
fn invalid_mnemonics_are_rejected() {
    for bad in [
        "",
        "abandon",
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon",
        "notaword abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
    ] {
        assert!(!derive::validate_mnemonic(bad), "accepted invalid mnemonic {bad:?}");
    }
}

/// A BIP39 passphrase (the "25th word") produces a completely different wallet.
/// Losing it makes the entropy alone useless, which is why it must be stored.
#[test]
fn bip39_passphrase_changes_the_wallet() {
    let vec = &v().vectors[0];
    let plain = derive::seed_from_mnemonic(&vec.mnemonic, "").unwrap();
    let with_pass = derive::seed_from_mnemonic(&vec.mnemonic, "trezor").unwrap();
    assert_ne!(plain[..], with_pass[..]);

    let a = derive::address_at(&plain, HOT, 0).unwrap();
    let b = derive::address_at(&with_pass, HOT, 0).unwrap();
    assert_ne!(a, b, "passphrase did not change the derived address");
}

// ── EVM chains (BNB Chain) ─────────────────────────────────────────────────

/// The canonical BIP44 vector for EVM chains: the Ledger test mnemonic at
/// `m/44'/60'/0'/0/0` is `0x9858EfFD232B4033E47d90003D41EC34EcaEda94` in every
/// wallet that implements the standard. Pinning it here means an address this
/// program shows can be checked against MetaMask, a Ledger, or any other
/// wallet - and a change that silently moved the derivation path would be
/// caught before it sent somebody's funds to an address they do not control.
///
/// (That mnemonic is public, so the addresses below are public too. The one
/// above has over ten thousand transactions on BNB Chain. Never fund it.)
#[test]
fn evm_addresses_match_the_standard_vector() {
    let seed = derive::seed_from_mnemonic(LEDGER, "").unwrap();
    let want = [
        "0x9858EfFD232B4033E47d90003D41EC34EcaEda94",
        "0x6Fac4D18c912343BF86fa7049364Dd4E424Ab9C0",
        "0xb6716976A3ebe8D39aCEB04372f22Ff8e6802D7A",
    ];
    for (i, w) in want.iter().enumerate() {
        let got = derive::evm_address_at(&seed, 0, i as u32).unwrap();
        assert_eq!(&got.to_string(), w, "EVM address at index {i}");
    }
}

/// One phrase, two chains, two different addresses - and the private keys
/// differ too. Users find this surprising, so it is worth a test that says it
/// out loud: a TRON address and an EVM address from the same wallet are not
/// the same account, and funds sent to one do not appear at the other.
#[test]
fn the_same_phrase_gives_different_keys_per_chain() {
    let seed = derive::seed_from_mnemonic(LEDGER, "").unwrap();
    let tron_key = derive::private_key_at(&seed, 0, 0).unwrap();
    let evm_key = derive::evm_private_key_at(&seed, 0, 0).unwrap();
    assert_ne!(*tron_key, *evm_key, "the two chains derived the same key");

    assert_eq!(
        derive::path_for_coin(derive::COIN_TYPE, 0, 0),
        "m/44'/195'/0'/0/0"
    );
    assert_eq!(
        derive::path_for_coin(derive::COIN_TYPE_EVM, 0, 0),
        "m/44'/60'/0'/0/0"
    );
}

/// The EVM address must be exactly the TRON address without its 0x41 prefix,
/// for the *same* key. The two encoders share a construction; this keeps them
/// from drifting.
#[test]
fn both_encodings_describe_the_same_twenty_bytes() {
    let seed = derive::seed_from_mnemonic(LEDGER, "").unwrap();
    let sk = derive::private_key_at(&seed, 0, 0).unwrap();
    let tron = derive::address_from_private_key(&sk).unwrap();
    let evm = derive::evm_address_from_private_key(&sk).unwrap();
    assert_eq!(evm.as_bytes(), &tron.to_evm_bytes());
}
