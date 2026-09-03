//! Cross-language conformance against `vectors/crypto.json`.
//!
//! These vectors were produced by the Go implementation of TronVault and
//! independently reproduced by its TypeScript client. If any assertion here
//! fails, this build produces ciphertext the reference implementations cannot
//! read (or vice versa) — which for a wallet means unrecoverable funds.
//!
//! Note on labels: the vectors use the OLD `tronvault/...` HKDF labels. They are
//! kept here purely as *test inputs* proving the HKDF-SHA512 primitive matches.
//! neko-wallet's own labels (`neko/...`) live in `neko_crypto::info`.

use neko_crypto::{aad::Aad, aead, kdf, kdf_hkdf, Argon2idParams};
use serde::Deserialize;

const RAW: &str = include_str!("../../../vectors/crypto.json");

#[derive(Deserialize)]
struct AadVariant {
    table: String,
    column: String,
    #[serde(rename = "rowID")]
    row_id: i64,
    #[serde(rename = "keyVer")]
    key_ver: u32,
    extra: String,
    encoded: String,
}

#[derive(Deserialize)]
struct Kdf {
    iters: u32,
    #[serde(rename = "keyLen")]
    key_len: u32,
    #[serde(rename = "memKiB")]
    mem_kib: u32,
    par: u8,
}

#[derive(Deserialize)]
struct Vectors {
    #[serde(rename = "aadVariants")]
    aad_variants: Vec<AadVariant>,
    kdf: Kdf,
    #[serde(rename = "kdfEncoded")]
    kdf_encoded: String,
    kek: String,
    #[serde(rename = "kekV2")]
    kek_v2: String,
    key: String,
    mk: String,
    password: String,
    pepper: String,
    salt: String,
    sealed: String,
    #[serde(rename = "sealedAADTable")]
    sealed_aad_table: String,
    #[serde(rename = "sealedAADColumn")]
    sealed_aad_column: String,
    subkeys: std::collections::BTreeMap<String, String>,
    #[serde(rename = "hardenedHMAC")]
    hardened_hmac: String,
}

fn v() -> Vectors {
    serde_json::from_str(RAW).expect("vectors/crypto.json is malformed")
}
fn hx(s: &str) -> Vec<u8> {
    hex::decode(s).expect("bad hex in vectors")
}
fn params(k: &Kdf) -> Argon2idParams {
    Argon2idParams {
        mem_kib: k.mem_kib,
        iters: k.iters,
        par: k.par,
        key_len: k.key_len,
    }
}

/// The single most likely place to be off by a byte.
#[test]
fn aad_encoding_matches_reference() {
    for (i, a) in v().aad_variants.iter().enumerate() {
        let extra = hx(&a.extra);
        let got = Aad {
            table: &a.table,
            column: &a.column,
            row_id: a.row_id,
            key_ver: a.key_ver,
            extra: &extra,
        }
        .encode();
        assert_eq!(
            hex::encode(&got),
            a.encoded,
            "AAD variant {i} ({}/{}) diverges from the reference encoding",
            a.table,
            a.column
        );
    }
}

/// `{table:"ab",column:"c"}` and `{table:"a",column:"bc"}` must not collide.
#[test]
fn aad_length_prefixes_prevent_collision() {
    assert_ne!(
        Aad::new("ab", "c", 1, 1).encode(),
        Aad::new("a", "bc", 1, 1).encode()
    );
}

#[test]
fn kdf_params_encoding_is_frozen() {
    let vec = v();
    assert_eq!(hex::encode(params(&vec.kdf).encode()), vec.kdf_encoded);
}

/// Proves our Argon2id (argon2 0.6 + rayon) produces standard output.
#[test]
fn argon2id_matches_reference() {
    let vec = v();
    let got = kdf::derive_key(vec.password.as_bytes(), &hx(&vec.salt), params(&vec.kdf)).unwrap();
    assert_eq!(hex::encode(&*got), vec.kek, "Argon2id output diverges");
}

/// End-to-end proof against a real ciphertext produced by the Go implementation.
#[test]
fn xchacha_opens_reference_ciphertext() {
    let vec = v();
    let mut extra = params(&vec.kdf).encode().to_vec();
    extra.extend_from_slice(&hx(&vec.salt));
    let aad = Aad {
        table: &vec.sealed_aad_table,
        column: &vec.sealed_aad_column,
        row_id: 1,
        key_ver: 1,
        extra: &extra,
    };
    let pt = aead::open(&hx(&vec.key), &hx(&vec.sealed), aad)
        .expect("could not open the reference ciphertext");
    assert_eq!(hex::encode(&*pt), vec.mk, "recovered plaintext is not MK");
}

/// A ciphertext bound to one cell must not open under another cell's AAD.
#[test]
fn aad_binding_prevents_row_and_column_swap() {
    let vec = v();
    let mut extra = params(&vec.kdf).encode().to_vec();
    extra.extend_from_slice(&hx(&vec.salt));
    let (key, sealed) = (hx(&vec.key), hx(&vec.sealed));

    let cases: [(&str, Aad); 5] = [
        (
            "row",
            Aad {
                table: "vault",
                column: "wrapped_mk",
                row_id: 2,
                key_ver: 1,
                extra: &extra,
            },
        ),
        (
            "column",
            Aad {
                table: "vault",
                column: "other",
                row_id: 1,
                key_ver: 1,
                extra: &extra,
            },
        ),
        (
            "table",
            Aad {
                table: "other",
                column: "wrapped_mk",
                row_id: 1,
                key_ver: 1,
                extra: &extra,
            },
        ),
        (
            "key_ver",
            Aad {
                table: "vault",
                column: "wrapped_mk",
                row_id: 1,
                key_ver: 2,
                extra: &extra,
            },
        ),
        ("extra", Aad::new("vault", "wrapped_mk", 1, 1)),
    ];
    for (name, aad) in cases {
        assert!(
            aead::open(&key, &sealed, aad).is_err(),
            "{name} swap was accepted"
        );
    }
}

#[test]
fn hkdf_sha512_matches_reference() {
    let vec = v();
    let mk = hx(&vec.mk);
    for (label, want) in &vec.subkeys {
        let got = kdf_hkdf::derive_key32(&mk, label).unwrap();
        assert_eq!(hex::encode(&*got), *want, "HKDF subkey `{label}` diverges");
    }
}

#[test]
fn hmac_sha256_matches_reference() {
    let vec = v();
    let got = kdf_hkdf::hmac_sha256(
        &hx(&vec.pepper),
        &[b"tronvault/vault-harden-v1", &hx(&vec.kek)],
    );
    assert_eq!(hex::encode(got), vec.hardened_hmac);
}

/// Exercises derive_from_ikm (variable-length IKM) against a known answer.
#[test]
fn derive_from_ikm_matches_reference() {
    let vec = v();
    let mut ikm = hx(&vec.kek);
    ikm.extend_from_slice(&hx(&vec.hardened_hmac));
    let got = kdf_hkdf::derive_from_ikm(&ikm, "tronvault/vault-kek-v2", 32).unwrap();
    assert_eq!(hex::encode(&*got), vec.kek_v2);
}

#[test]
fn seal_open_roundtrip_and_tamper_detection() {
    let key = neko_crypto::random(32).unwrap();
    let aad = Aad::new("wallets", "mnemonic_ct", 42, 1);
    let msg = b"abandon abandon abandon about";

    let sealed = aead::seal(&key, msg, aad).unwrap();
    assert_eq!(&aead::open(&key, &sealed, aad).unwrap()[..], msg);

    for pos in [0usize, aead::NONCE_LEN, sealed.len() - 1] {
        let mut bad = sealed.clone();
        bad[pos] ^= 0xFF;
        assert!(
            aead::open(&key, &bad, aad).is_err(),
            "tamper at {pos} not detected"
        );
    }
    let other = neko_crypto::random(32).unwrap();
    assert!(
        aead::open(&other, &sealed, aad).is_err(),
        "wrong key accepted"
    );
}

#[test]
fn nonces_do_not_repeat() {
    let key = neko_crypto::random(32).unwrap();
    let aad = Aad::new("t", "c", 1, 1);
    let mut seen = std::collections::HashSet::new();
    for _ in 0..1000 {
        let s = aead::seal(&key, b"x", aad).unwrap();
        assert!(seen.insert(s[..aead::NONCE_LEN].to_vec()), "nonce reuse");
    }
}

#[test]
fn weak_kdf_params_are_rejected() {
    let ok = Argon2idParams {
        mem_kib: 65_536,
        iters: 2,
        par: 4,
        key_len: 32,
    };
    assert!(ok.validate().is_ok());
    for bad in [
        Argon2idParams {
            mem_kib: 1024,
            ..ok
        },
        Argon2idParams { iters: 1, ..ok },
        Argon2idParams { par: 0, ..ok },
        Argon2idParams { key_len: 16, ..ok },
    ] {
        assert!(bad.validate().is_err(), "weak params accepted: {bad:?}");
    }
}

/// Every neko label must derive a distinct key, and none may equal MK.
#[test]
fn neko_subkeys_are_distinct() {
    use neko_crypto::info;
    let mk = neko_crypto::random(32).unwrap();
    let labels = [
        info::FILE_KEY,
        info::KEK,
        info::DATA_KEY,
        info::BLIND_INDEX,
        info::VERIFIER,
        info::WALLET_SEED,
    ];
    let mut seen = std::collections::HashSet::new();
    for l in labels {
        let k = kdf_hkdf::derive_key32(&mk, l).unwrap();
        assert_ne!(&k[..], &mk[..], "subkey `{l}` equals MK");
        assert!(seen.insert(k.to_vec()), "duplicate subkey for `{l}`");
    }
}
