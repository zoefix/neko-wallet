//! Transfer construction and signing, end to end from a real vault.

use neko_core::{
    Amount, Asset, ChainAddress, ChainId, ChainTxParams, NewWalletSpec, TransferRequest, VaultFile,
};
use neko_tron::tx::TxParams;
use neko_vault::profile;

const EMAIL: &str = "zoe@example.com";
const PW: &str = "correct horse battery staple xyzzy";
const LEDGER_PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const LEDGER_ADDR: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";
const TO: &str = "TNYxHL2s6Wjpx86NRwhekYzc27p3oDYrk6";

/// The exact parameters from `vectors/tx.json`, so signatures are comparable.
fn vector_params(fee_limit: i64) -> TxParams {
    TxParams {
        ref_block_num: 68_000_123,
        ref_block_hash: [0xab; 32],
        timestamp: 1_756_000_000_000,
        expiration: 1_756_000_060_000,
        fee_limit,
    }
}

fn session(dir: &std::path::Path) -> neko_core::Session {
    let mut s = VaultFile::at(dir.join("w.db"))
        .create(EMAIL, PW, profile::TESTONLY)
        .unwrap();
    s.create_wallet(
        "ledger",
        NewWalletSpec::ImportMnemonic {
            phrase: LEDGER_PHRASE,
            passphrase: None,
        },
    )
    .unwrap();
    s
}

/// A signed TRX transfer must reproduce the reference transaction exactly.
#[test]
fn signed_trx_transfer_matches_the_reference_vector() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(dir.path());
    let id = s.list_wallets().unwrap()[0].id;

    let req = TransferRequest::parse(
        id,
        ChainAddress::parse(ChainId::Tron, LEDGER_ADDR).unwrap(),
        TO,
        "1.5",
        Asset::Trx,
    )
    .unwrap();
    assert_eq!(req.amount.raw, 1_500_000);

    let signed = s
        .sign_transfer(
            &req,
            &ChainTxParams::Tron(Box::new(vector_params(100_000_000))),
        )
        .unwrap();
    assert_eq!(
        signed.id, "81ccc5c00584abbd0dc17eb5da418911868dec309056cc0ee6420bb7bda8c70e",
        "txid diverges from the reference"
    );
    // The signature recovering to the paying address is enforced by
    // construction: `neko_tron::tx::sign` performs that check and returns an
    // error rather than a transaction when it fails, so a `SignedTransfer`
    // cannot exist carrying somebody else's signature. `neko-tron`'s own
    // vectors exercise the check directly.
    assert!(!signed.raw.is_empty());
}

#[test]
fn signed_trc20_transfer_matches_the_reference_vector() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(dir.path());
    let id = s.list_wallets().unwrap()[0].id;

    let asset = Asset::Trc20 {
        contract: neko_hd::Address::parse("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t").unwrap(),
        decimals: 6,
    };
    let req = TransferRequest::parse(
        id,
        ChainAddress::parse(ChainId::Tron, LEDGER_ADDR).unwrap(),
        TO,
        "2.5",
        asset,
    )
    .unwrap();
    assert_eq!(req.amount.raw, 2_500_000);

    let signed = s
        .sign_transfer(
            &req,
            &ChainTxParams::Tron(Box::new(vector_params(100_000_000))),
        )
        .unwrap();
    assert_eq!(
        signed.id, "a4da5677d59ed5ce830b3a5f57c764bace6b5805f77a1a157c336d650fa8d477",
        "TRC20 txid diverges from the reference"
    );
}

#[test]
fn malformed_recipients_and_amounts_are_rejected() {
    let from = ChainAddress::parse(ChainId::Tron, LEDGER_ADDR).unwrap();
    for bad_addr in [
        "",
        "T",
        "not-base58-0OIl",
        "1BvBMSEYstWetqTFn5Au4m4GFg7xJaNVN2",
        "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdX",
    ] {
        assert!(
            TransferRequest::parse(1, from, bad_addr, "1", Asset::Trx).is_err(),
            "accepted invalid recipient {bad_addr:?}"
        );
    }
    for bad_amt in ["", "abc", "0", "-1", "1.2345678"] {
        assert!(
            TransferRequest::parse(1, from, TO, bad_amt, Asset::Trx).is_err(),
            "accepted invalid amount {bad_amt:?}"
        );
    }
}

/// Signing with a wallet that does not own `from` must fail the self-check
/// rather than produce a transaction attributed to the wrong address.
#[test]
fn signing_refuses_a_mismatched_sender() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let other = s
        .create_wallet("other", NewWalletSpec::Generate { words: 12 })
        .unwrap();

    // Claim to be sending from the Ledger address, but sign with `other`.
    let req = TransferRequest::parse(
        other,
        ChainAddress::parse(ChainId::Tron, LEDGER_ADDR).unwrap(),
        TO,
        "1",
        Asset::Trx,
    )
    .unwrap();
    let err = s.sign_transfer(&req, &ChainTxParams::Tron(Box::new(vector_params(0))));
    assert!(
        err.is_err(),
        "signed a transfer from an address this wallet does not own"
    );
}

/// A contract call needs a fee limit or it fails on-chain for lack of energy.
#[test]
fn trc20_requires_a_fee_limit() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(dir.path());
    let id = s.list_wallets().unwrap()[0].id;
    let asset = Asset::Trc20 {
        contract: neko_hd::Address::parse("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t").unwrap(),
        decimals: 6,
    };
    assert_eq!(
        asset.tron_fee_limit().unwrap(),
        100_000_000,
        "TRC20 must carry a fee limit"
    );
    assert_eq!(Asset::Trx.tron_fee_limit().unwrap(), 0);

    let req = TransferRequest::parse(
        id,
        ChainAddress::parse(ChainId::Tron, LEDGER_ADDR).unwrap(),
        TO,
        "1",
        asset,
    )
    .unwrap();
    assert!(
        s.sign_transfer(&req, &ChainTxParams::Tron(Box::new(vector_params(0))))
            .is_err(),
        "built a contract call with no fee limit"
    );
}

#[test]
fn calldata_is_produced_only_for_contract_transfers() {
    let from = ChainAddress::parse(ChainId::Tron, LEDGER_ADDR).unwrap();
    let trx = TransferRequest::parse(1, from, TO, "1", Asset::Trx).unwrap();
    assert!(trx.calldata().unwrap().is_none());

    let asset = Asset::Trc20 {
        contract: neko_hd::Address::parse("TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t").unwrap(),
        decimals: 6,
    };
    let trc20 = TransferRequest::parse(1, from, TO, "2.5", asset).unwrap();
    let data = trc20.calldata().unwrap().expect("no calldata");
    assert_eq!(
        hex::encode(&data),
        "a9059cbb0000000000000000000000008a035d6a1840369c2d85dbe08ac5dcc640d0f0de00000000000000000000000000000000000000000000000000000000002625a0"
    );
}

/// A private-key wallet must be able to sign too.
#[test]
fn private_key_wallets_can_sign() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = VaultFile::at(dir.path().join("w.db"))
        .create(EMAIL, PW, profile::TESTONLY)
        .unwrap();
    let id = s
        .create_wallet(
            "key",
            NewWalletSpec::ImportPrivateKey {
                hex: "b5a4cea271ff424d7c31dc12a3e43e401df7a40d7412a15750f3f0b6b5449a28",
            },
        )
        .unwrap();

    let req = TransferRequest::parse(
        id,
        ChainAddress::parse(ChainId::Tron, LEDGER_ADDR).unwrap(),
        TO,
        "1.5",
        Asset::Trx,
    )
    .unwrap();
    let signed = s
        .sign_transfer(
            &req,
            &ChainTxParams::Tron(Box::new(vector_params(100_000_000))),
        )
        .unwrap();
    assert_eq!(
        signed.id,
        "81ccc5c00584abbd0dc17eb5da418911868dec309056cc0ee6420bb7bda8c70e"
    );
}

#[test]
fn amounts_that_break_f64_survive_the_whole_path() {
    let from = ChainAddress::parse(ChainId::Tron, LEDGER_ADDR).unwrap();
    let req = TransferRequest::parse(1, from, TO, "9007199254.740993", Asset::Trx).unwrap();
    assert_eq!(req.amount.raw, 9_007_199_254_740_993);
    assert_eq!(req.amount.to_exact_string(), "9007199254.740993");
    assert_eq!(
        Amount::new(req.amount.raw, 6).to_display_string(),
        "9,007,199,254.740993"
    );
}
