//! Transfer construction and signing, end to end from a real vault.

use neko_core::{
    Amount, Asset, ChainAddress, ChainId, ChainTxParams, CoreError, NewWalletSpec, TransferRequest,
    VaultFile,
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

// ── TON checks the key against the address before it signs ─────────────────

const TON_ADDR: &str = "EQAzWZa6nM5mJev91wGc7VCSfBoIsYRqKJpV78N8Add9-U9d";
const TON_TO: &str = "EQDVJucJT96vGh_bYm3e5uzenasiTOwA9orUHQiyhNsKmEcK";

fn ton_params() -> ChainTxParams {
    ChainTxParams::Ton(Box::new(neko_core::TonTxParams {
        seqno: 3,
        valid_until: 1_800_000_000,
        deploy: false,
        jetton_wallet: None,
    }))
}

/// The ordinary case, so the refusals below are about what they claim to be.
#[test]
fn a_gram_transfer_signs_for_its_own_address() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(dir.path());
    let id = s.list_wallets().unwrap()[0].id;

    let req = TransferRequest::parse(
        id,
        ChainAddress::parse(ChainId::Ton, TON_ADDR).unwrap(),
        TON_TO,
        "0.5",
        Asset::Gram,
    )
    .unwrap();
    assert_eq!(req.amount.raw, 500_000_000, "GRAM is nine decimals");

    let signed = s.sign_transfer(&req, &ton_params()).unwrap();
    assert_eq!(signed.id.len(), 64, "the id is a message hash in hex");
    assert!(!signed.raw.is_empty());
}

/// A TON address is the hash of the contract holding a public key, so the two
/// can be checked against each other before anything is signed.
///
/// No other chain here can do this. Everywhere else a mismatched key produces a
/// perfectly valid signature by somebody else, and the failure only shows up as
/// a message the contract ignores - which is indistinguishable from a transfer
/// that vanished.
#[test]
fn signing_for_an_address_this_key_does_not_own_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(dir.path());
    let id = s.list_wallets().unwrap()[0].id;

    // A real TON address, and not this wallet's.
    let someone_else = ChainAddress::parse(ChainId::Ton, TON_TO).unwrap();
    let req = TransferRequest::parse(id, someone_else, TON_ADDR, "0.5", Asset::Gram).unwrap();

    match s.sign_transfer(&req, &ton_params()) {
        Err(CoreError::WrongSigningKey { expected, derived }) => {
            assert_eq!(expected, TON_TO);
            assert_eq!(derived, TON_ADDR, "the key derives this wallet's address");
        }
        Err(e) => panic!("refused for the wrong reason: {e}"),
        Ok(_) => panic!("signed a message for an address this key does not own"),
    }
}

/// A jetton wallet address cannot be checked offline - it is a contract's hash,
/// obtained by asking the master. So the master travels with it, and signing
/// refuses an address that was derived from a different token.
#[test]
fn a_jetton_wallet_quoted_for_another_token_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(dir.path());
    let id = s.list_wallets().unwrap()[0].id;

    let usdt = ChainId::Ton.stable().expect("TON has a USDT");
    let req = TransferRequest::parse(
        id,
        ChainAddress::parse(ChainId::Ton, TON_ADDR).unwrap(),
        TON_TO,
        "10",
        usdt,
    )
    .unwrap();
    assert_eq!(req.amount.raw, 10_000_000, "USDT is six decimals on TON");

    // Quoted against some other jetton master, with a wallet address under it.
    let other = neko_ton::TonAddress::parse(TON_TO).unwrap();
    let params = ChainTxParams::Ton(Box::new(neko_core::TonTxParams {
        seqno: 3,
        valid_until: 1_800_000_000,
        deploy: false,
        jetton_wallet: Some((other, other)),
    }));

    match s.sign_transfer(&req, &params) {
        Err(CoreError::WrongToken { quoted, asked }) => {
            assert_eq!(quoted, TON_TO);
            assert_eq!(asked, neko_ton::USDT_MASTER);
        }
        Err(e) => panic!("refused for the wrong reason: {e}"),
        Ok(_) => panic!("signed a transfer into another token's wallet contract"),
    }
}

/// And without a jetton wallet at all there is nothing to send to. Refused
/// rather than defaulted to the recipient's own address, which is a real
/// address that would not credit them.
#[test]
fn a_jetton_transfer_without_a_wallet_address_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(dir.path());
    let id = s.list_wallets().unwrap()[0].id;

    let req = TransferRequest::parse(
        id,
        ChainAddress::parse(ChainId::Ton, TON_ADDR).unwrap(),
        TON_TO,
        "10",
        ChainId::Ton.stable().unwrap(),
    )
    .unwrap();
    assert!(s.sign_transfer(&req, &ton_params()).is_err());
}

// ── Polygon ────────────────────────────────────────────────────────────────

/// One phrase, every EVM chain, one address - and a different signature each.
///
/// The address being shared is correct and is what every EVM wallet does. The
/// signatures being different is what stops a transfer signed for one chain
/// being replayed on another where the same address also holds funds, and the
/// only thing that separates them is the chain id inside the envelope.
///
/// **Both assets, and the chain list comes from `CHAINS`.** This test used to
/// name five chains by hand and sign only the coin. When a sixth arrived it
/// was in neither the list nor the two arms of `sign_transfer` that handle EVM
/// assets, and the wildcard at the end of that match refused it as "that
/// address belongs to a different chain" - at the password prompt, with the
/// test still green.
#[test]
fn every_evm_chain_signs_for_itself_and_no_other() {
    let dir = tempfile::tempdir().unwrap();
    let s = session(dir.path());
    let id = s.list_wallets().unwrap()[0].id;

    let evm_addr = "0x9858EfFD232B4033E47d90003D41EC34EcaEda94";
    let to = "0x742d35Cc6634C0532925a3b844Bc454e4438f44e";
    let evm_chains: Vec<ChainId> = neko_core::CHAINS
        .into_iter()
        .filter(|c| c.evm().is_some())
        .collect();
    assert!(evm_chains.len() >= 6, "every EVM chain is covered here");

    for chain in &evm_chains {
        assert_eq!(
            s.address_of(id, *chain, 0).unwrap().to_string(),
            evm_addr,
            "{chain:?} derived a different address"
        );
    }

    let mut raws = Vec::new();
    for chain in &evm_chains {
        let evm = chain.evm().unwrap();
        // Every asset the chain carries, not just its coin: the coin and the
        // token are signed by two different arms, and only one of them was
        // missing the new chain.
        for asset in chain.assets() {
            let amount = if asset.is_native() { "0.5" } else { "1" };
            let req = TransferRequest::parse(
                id,
                ChainAddress::parse(*chain, evm_addr).unwrap(),
                to,
                amount,
                asset,
            )
            .unwrap();

            let params = ChainTxParams::Evm(neko_evm::tx::TxParams {
                nonce: 7,
                gas_limit: 21_000,
                chain_id: evm.chain_id,
                fees: neko_evm::tx::Fees::Legacy {
                    gas_price: 30_000_000_000,
                },
            });
            let signed = s.sign_transfer(&req, &params).unwrap_or_else(|e| {
                panic!("{chain:?} could not sign {}: {e}", asset.symbol())
            });
            raws.push((*chain, asset.symbol(), signed.raw));
        }
    }

    // Two per chain, and no two alike: a coin transfer differs from a token
    // one, and the same transfer on two chains differs by the chain id.
    assert_eq!(raws.len(), evm_chains.len() * 2);
    for (i, (ca, sa, a)) in raws.iter().enumerate() {
        for (cb, sb, b) in &raws[i + 1..] {
            assert_ne!(
                a, b,
                "{ca:?}/{sa} and {cb:?}/{sb} produced the same signed bytes"
            );
        }
    }
}

/// Polygon's native coin is POL, and its USDT has six decimals rather than BNB
/// Chain's eighteen. Both are the kind of mistake that is a factor of a
/// million million.
#[test]
fn polygon_amounts_use_the_right_precision() {
    assert_eq!(ChainId::Polygon.native_symbol(), "POL");
    assert_eq!(ChainId::Polygon.native_decimals(), 18);

    let usdt = ChainId::Polygon.stable().expect("Polygon has a USDT");
    assert_eq!(usdt.decimals(), 6, "six here, eighteen on BNB Chain");
    assert_eq!(
        usdt.symbol(),
        "USDT",
        "shown as USDT whatever it calls itself"
    );
    assert_eq!(
        usdt.chain(),
        ChainId::Polygon,
        "an Erc20 variant would say Ethereum"
    );

    // No two chains share a stablecoin contract. Over every chain rather
    // than three of them: the failure this guards against is a contract
    // address copied from the chain above when a new one is added, and a list
    // written by hand cannot see the chain that was just added to it.
    let contracts: Vec<(ChainId, String)> = neko_core::CHAINS
        .into_iter()
        .filter_map(|c| c.stable().map(|t| (c, format!("{t:?}"))))
        .collect();
    for (i, (ca, a)) in contracts.iter().enumerate() {
        for (cb, b) in &contracts[i + 1..] {
            assert_ne!(a, b, "{ca:?} and {cb:?} share a stablecoin contract");
        }
    }
}

/// Base carries USDC, not USDT, and the wallet has to mean it everywhere.
///
/// Tether's contract on Base holds about 23 million against Circle's 4.2
/// billion, and Binance lists nineteen networks for USDT withdrawals with Base
/// on none of them - it offers ETH and USDC. A USDT row there is a row nobody
/// can put anything into.
///
/// Several places used to decide this by comparing a symbol against the
/// literal `"USDT"`: whether a cached balance gets a contract address, and
/// whether a holding is worth a dollar. Both would have been wrong here.
#[test]
fn base_carries_usdc_and_every_chain_agrees_with_itself() {
    use neko_core::Prices;

    let stable = ChainId::Base.stable().expect("Base has a stablecoin");
    assert_eq!(stable.symbol(), "USDC");
    assert_eq!(stable.decimals(), 6);
    assert_eq!(stable.chain(), ChainId::Base);

    // Every chain carries one dollar or the other, and which one is the
    // chain's own answer rather than a list kept here. Four carry Circle's:
    // Base and zkSync Era because Binance sends no USDT to either, Mantle and
    // Linea because Circle's supply there is the larger and neither token has
    // a route from an exchange at all.
    let mut usdc = Vec::new();
    for chain in neko_core::CHAINS {
        let Some(t) = chain.stable() else { continue };
        let sym = t.symbol();
        assert!(
            sym == "USDT" || sym == "USDC",
            "{chain:?} carries {sym}, which is neither dollar this wallet knows"
        );
        // The name shown must be the one the chain says to show. Off the EVM
        // chains there is no `stable_label`, and those all carry USDT.
        if let Some(evm) = chain.evm() {
            assert_eq!(sym, evm.stable_label, "{chain:?} is shown the wrong name");
        } else {
            assert_eq!(sym, "USDT", "{chain:?}");
        }
        if sym == "USDC" {
            usdc.push(chain);
        }
    }
    assert_eq!(
        usdc,
        [
            ChainId::Base,
            ChainId::Mantle,
            ChainId::Linea,
            ChainId::ZkSyncEra
        ],
        "the set of chains carrying Circle's dollar changed"
    );

    // And a dollar is a dollar on each of them.
    for chain in &usdc {
        assert_eq!(
            Prices::default().of(*chain, "USDC"),
            Some(1_000_000),
            "{chain:?}: its stablecoin was not worth a dollar"
        );
    }

    // A dollar-pegged token is one unit of account whatever it is called. This
    // used to compare against "USDT" and so valued every USDC balance at
    // nothing, which silently understates a wallet's total.
    let prices = Prices::default();
    assert_eq!(
        prices.of(ChainId::Base, "USDC"),
        Some(1_000_000),
        "Base's stablecoin was not worth a dollar"
    );
    assert_eq!(prices.of(ChainId::Base, "USDT"), None, "not on this chain");
    assert_eq!(prices.of(ChainId::Ethereum, "USDT"), Some(1_000_000));

    // And every chain's asset list names its own stablecoin.
    for chain in neko_core::CHAINS {
        let named: Vec<&str> = chain.assets().iter().map(|a| a.symbol()).collect();
        match chain.stable() {
            Some(s) => assert!(named.contains(&s.symbol()), "{chain:?}: {named:?}"),
            None => assert_eq!(named.len(), 1, "{chain:?} has only its coin"),
        }
    }
}
