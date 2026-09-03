//! Conformance against `vectors/tx.json`.
//!
//! From the reference project's own note: "浏览器端必须构造出逐字节相同的
//! raw_data，否则 txid 不同、签名无效." One byte of difference changes the
//! transaction id, invalidates the signature, and the network rejects it with
//! an error that says nothing about encoding — the hardest class of bug to
//! diagnose after the fact.

use neko_hd::Address;
use neko_tron::tx::{self, TxParams};
use serde::Deserialize;

const RAW: &str = include_str!("../../../vectors/tx.json");

#[derive(Deserialize)]
struct Params {
    expiration: i64,
    #[serde(rename = "feeLimit")]
    fee_limit: i64,
    #[serde(rename = "refBlockHash")]
    ref_block_hash: String,
    #[serde(rename = "refBlockNum")]
    ref_block_num: u64,
    timestamp: i64,
}

#[derive(Deserialize)]
struct Trx {
    #[serde(rename = "amountSun")]
    amount_sun: i64,
    #[serde(rename = "rawData")]
    raw_data: String,
    signature: String,
    #[serde(rename = "signedTx")]
    signed_tx: String,
    txid: String,
}

#[derive(Deserialize)]
struct Trc20 {
    amount: String,
    calldata: String,
    #[serde(rename = "rawData")]
    raw_data: String,
    signature: String,
    #[serde(rename = "signedTx")]
    signed_tx: String,
    txid: String,
}

#[derive(Deserialize)]
struct Stake {
    #[serde(rename = "rawData")]
    raw_data: String,
    txid: String,
}

#[derive(Deserialize)]
struct Vectors {
    contract: String,
    from: String,
    to: String,
    params: Params,
    #[serde(rename = "privKey")]
    priv_key: String,
    trx: Trx,
    trc20: Trc20,
    stake: std::collections::BTreeMap<String, Stake>,
    #[serde(rename = "stakeAmountSun")]
    stake_amount_sun: i64,
    #[serde(rename = "delegateAmountSun")]
    delegate_amount_sun: i64,
}

fn v() -> Vectors {
    serde_json::from_str(RAW).expect("vectors/tx.json is malformed")
}
fn addr(s: &str) -> Address {
    Address::parse(s).expect("bad address in vectors")
}
fn params(p: &Params) -> TxParams {
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&hex::decode(&p.ref_block_hash).unwrap());
    TxParams {
        ref_block_num: p.ref_block_num,
        ref_block_hash: hash,
        timestamp: p.timestamp,
        expiration: p.expiration,
        fee_limit: p.fee_limit,
    }
}
fn privkey(s: &str) -> [u8; 32] {
    let mut k = [0u8; 32];
    k.copy_from_slice(&hex::decode(s).unwrap());
    k
}

/// The whole encoder rests on this: proto3 omits default values.
#[test]
fn trx_transfer_raw_data_is_byte_exact() {
    let vec = v();
    // The reference vectors carry a fee limit on every transaction, including
    // plain transfers, where the chain simply ignores it. Decoding the expected
    // bytes confirms field 18 is present.
    let p = params(&vec.params);
    let raw =
        tx::build_trx_transfer(addr(&vec.from), addr(&vec.to), vec.trx.amount_sun, &p).unwrap();
    assert_eq!(hex::encode(&raw), vec.trx.raw_data, "TRX raw_data diverges");
    assert_eq!(
        hex::encode(tx::txid(&raw)),
        vec.trx.txid,
        "TRX txid diverges"
    );
}

/// RFC-6979 makes ECDSA deterministic, so the signature is reproducible and can
/// be asserted rather than merely verified.
#[test]
fn trx_transfer_signature_is_reproducible() {
    let vec = v();
    let p = params(&vec.params);
    let raw =
        tx::build_trx_transfer(addr(&vec.from), addr(&vec.to), vec.trx.amount_sun, &p).unwrap();
    let signed = tx::sign(&raw, &privkey(&vec.priv_key), addr(&vec.from)).unwrap();

    assert_eq!(
        hex::encode(signed.signature),
        vec.trx.signature,
        "signature diverges"
    );
    assert_eq!(
        hex::encode(&signed.raw_tx),
        vec.trx.signed_tx,
        "signed transaction diverges"
    );
    assert_eq!(hex::encode(signed.txid), vec.trx.txid);
}

/// The TRON-specific trap: the ABI address argument drops the 0x41 prefix.
#[test]
fn trc20_calldata_is_byte_exact() {
    let vec = v();
    let amount: u128 = vec.trc20.amount.parse().unwrap();
    let data = tx::encode_trc20_transfer(addr(&vec.to), amount).unwrap();
    assert_eq!(
        hex::encode(&data),
        vec.trc20.calldata,
        "TRC20 calldata diverges"
    );
    assert_eq!(hex::encode(tx::trc20_transfer_selector()), "a9059cbb");
    // Bytes 4..16 are the zero padding ahead of the 20-byte address.
    assert!(
        data[4..16].iter().all(|b| *b == 0),
        "address argument is misaligned"
    );
}

#[test]
fn trc20_transfer_raw_data_and_signature_are_byte_exact() {
    let vec = v();
    let p = params(&vec.params);
    let amount: u128 = vec.trc20.amount.parse().unwrap();
    let raw = tx::build_trc20_transfer(
        addr(&vec.from),
        addr(&vec.contract),
        addr(&vec.to),
        amount,
        &p,
    )
    .unwrap();
    assert_eq!(
        hex::encode(&raw),
        vec.trc20.raw_data,
        "TRC20 raw_data diverges"
    );
    assert_eq!(
        hex::encode(tx::txid(&raw)),
        vec.trc20.txid,
        "TRC20 txid diverges"
    );

    let signed = tx::sign(&raw, &privkey(&vec.priv_key), addr(&vec.from)).unwrap();
    assert_eq!(hex::encode(signed.signature), vec.trc20.signature);
    assert_eq!(hex::encode(&signed.raw_tx), vec.trc20.signed_tx);
}

/// Five more byte-exact cases through the same encoder, covering enum values
/// 54-58 and the omitted `lock` field.
#[test]
fn staking_transactions_are_byte_exact() {
    let vec = v();
    let p = params(&vec.params);
    let owner = addr(&vec.from);
    let receiver = addr(&vec.to);

    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "freeze",
            tx::build_freeze_for_energy(owner, vec.stake_amount_sun, &p).unwrap(),
        ),
        (
            "unfreeze",
            tx::build_unfreeze_energy(owner, vec.stake_amount_sun, &p).unwrap(),
        ),
        (
            "withdraw",
            tx::build_withdraw_expire_unfreeze(owner, &p).unwrap(),
        ),
        (
            "delegate",
            tx::build_delegate_energy(owner, receiver, vec.delegate_amount_sun, &p).unwrap(),
        ),
        (
            "undelegate",
            tx::build_undelegate_energy(owner, receiver, vec.delegate_amount_sun, &p).unwrap(),
        ),
    ];

    for (name, raw) in cases {
        let want = vec
            .stake
            .get(name)
            .unwrap_or_else(|| panic!("no vector for {name}"));
        assert_eq!(hex::encode(&raw), want.raw_data, "{name} raw_data diverges");
        assert_eq!(
            hex::encode(tx::txid(&raw)),
            want.txid,
            "{name} txid diverges"
        );
    }
}

/// Signing must refuse when the key does not correspond to the sender.
#[test]
fn signature_self_check_catches_the_wrong_key() {
    let vec = v();
    let p = params(&vec.params);
    let raw = tx::build_trx_transfer(addr(&vec.from), addr(&vec.to), 1, &p).unwrap();

    // A valid key, but not the one that owns `from`.
    let other = privkey("edb728e259afca2ddcc428459e7681b8414668649aedbc8d25c0872da219b2e0");
    let err = tx::sign(&raw, &other, addr(&vec.from));
    assert!(
        matches!(err, Err(neko_tron::TxError::SelfCheck { .. })),
        "wrong key was accepted"
    );
}

#[test]
fn recover_signer_round_trips() {
    let vec = v();
    let p = params(&vec.params);
    let raw = tx::build_trx_transfer(addr(&vec.from), addr(&vec.to), 1, &p).unwrap();
    let signed = tx::sign(&raw, &privkey(&vec.priv_key), addr(&vec.from)).unwrap();
    assert_eq!(
        tx::recover_signer(&raw, &signed.signature).unwrap(),
        addr(&vec.from)
    );
}

#[test]
fn malformed_transactions_are_refused() {
    let vec = v();
    let p = params(&vec.params);
    let (from, to, contract) = (addr(&vec.from), addr(&vec.to), addr(&vec.contract));

    assert!(
        tx::build_trx_transfer(from, to, 0, &p).is_err(),
        "zero amount accepted"
    );
    assert!(
        tx::build_trx_transfer(from, to, -1, &p).is_err(),
        "negative amount accepted"
    );

    // A contract call without a fee limit fails on-chain for lack of energy, so
    // refuse to build it at all.
    let mut no_fee = params(&vec.params);
    no_fee.fee_limit = 0;
    assert!(
        tx::build_trc20_transfer(from, contract, to, 1, &no_fee).is_err(),
        "contract call without a fee limit was accepted"
    );

    let mut bad_time = params(&vec.params);
    bad_time.expiration = bad_time.timestamp - 1;
    assert!(
        tx::build_trx_transfer(from, to, 1, &bad_time).is_err(),
        "expired window accepted"
    );
}
