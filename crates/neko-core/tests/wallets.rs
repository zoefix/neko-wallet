//! Wallet lifecycle: create, import, derive, reveal, rename, delete.

use neko_core::ChainId;
use neko_core::{CoreError, NewWalletSpec, VaultFile};
use neko_store::repo::wallets::Origin;
use neko_vault::profile;

const EMAIL: &str = "zoe@example.com";
const PW: &str = "correct horse battery staple xyzzy";
const PW2: &str = "plaid walrus trombone velvet 41";

/// LedgerHQ's official BIP39 test mnemonic and the TRON address it must yield.
const LEDGER_PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const LEDGER_ADDR: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";
/// m/44'/195'/0'/0/0 for the phrase above.
const LEDGER_KEY: &str = "b5a4cea271ff424d7c31dc12a3e43e401df7a40d7412a15750f3f0b6b5449a28";

fn vault(dir: &std::path::Path) -> (VaultFile, std::path::PathBuf) {
    let p = dir.join("neko-wallet.db");
    (VaultFile::at(&p), p)
}

fn fresh(dir: &std::path::Path) -> (neko_core::Session, std::path::PathBuf) {
    let (vf, p) = vault(dir);
    (vf.create(EMAIL, PW, profile::TESTONLY).unwrap(), p)
}

#[test]
fn generated_wallet_derives_a_valid_tron_address() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = fresh(dir.path());

    let id = s
        .create_wallet("钱包1", NewWalletSpec::Generate { words: 12 })
        .unwrap();
    let list = s.list_wallets().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].id, id);
    assert_eq!(
        list[0].label, "钱包1",
        "CJK label must survive the round trip"
    );
    assert_eq!(list[0].origin, Origin::Generated);
    assert!(
        list[0].address(ChainId::Tron).starts_with('T'),
        "got {}",
        list[0].address(ChainId::Tron)
    );
    assert_eq!(list[0].address(ChainId::Tron).len(), 34);
}

/// Importing the Ledger vector must land on the exact published address.
#[test]
fn imported_mnemonic_matches_the_ledger_vector() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = fresh(dir.path());

    let id = s
        .create_wallet(
            "ledger",
            NewWalletSpec::ImportMnemonic {
                phrase: LEDGER_PHRASE,
                passphrase: None,
            },
        )
        .unwrap();
    assert_eq!(
        s.address_of(id, ChainId::Tron, 0).unwrap().to_string(),
        LEDGER_ADDR
    );
}

#[test]
fn imported_private_key_matches_the_reference() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = fresh(dir.path());

    let id = s
        .create_wallet(
            "hot key",
            NewWalletSpec::ImportPrivateKey { hex: LEDGER_KEY },
        )
        .unwrap();
    assert_eq!(
        s.address_of(id, ChainId::Tron, 0).unwrap().to_string(),
        LEDGER_ADDR
    );
    assert_eq!(s.list_wallets().unwrap()[0].origin, Origin::ImportedPrivkey);

    // A 0x prefix is accepted too.
    let id2 = s
        .create_wallet(
            "prefixed",
            NewWalletSpec::ImportPrivateKey {
                hex: &format!("0x{LEDGER_KEY}"),
            },
        )
        .unwrap();
    assert_eq!(
        s.address_of(id2, ChainId::Tron, 0).unwrap().to_string(),
        LEDGER_ADDR
    );
}

#[test]
fn malformed_imports_are_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = fresh(dir.path());

    for bad in ["", "not a mnemonic at all", "abandon abandon abandon"] {
        assert!(matches!(
            s.create_wallet(
                "x",
                NewWalletSpec::ImportMnemonic {
                    phrase: bad,
                    passphrase: None
                }
            ),
            Err(CoreError::BadMnemonic)
        ));
    }
    for bad in ["", "zz", &"0".repeat(64), &"f".repeat(64), "0x123"] {
        assert!(
            matches!(
                s.create_wallet("x", NewWalletSpec::ImportPrivateKey { hex: bad }),
                Err(CoreError::BadPrivateKey)
            ),
            "accepted invalid private key {bad:?}"
        );
    }
    assert!(
        s.list_wallets().unwrap().is_empty(),
        "a rejected import must leave no row"
    );
}

/// The 25th word changes the wallet, so it has to be stored and reused.
#[test]
fn bip39_passphrase_is_persisted_and_applied() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = fresh(dir.path());

    let plain = s
        .create_wallet(
            "plain",
            NewWalletSpec::ImportMnemonic {
                phrase: LEDGER_PHRASE,
                passphrase: None,
            },
        )
        .unwrap();
    let with_pass = s
        .create_wallet(
            "with pass",
            NewWalletSpec::ImportMnemonic {
                phrase: LEDGER_PHRASE,
                passphrase: Some("trezor"),
            },
        )
        .unwrap();

    let a = s.address_of(plain, ChainId::Tron, 0).unwrap().to_string();
    let b = s
        .address_of(with_pass, ChainId::Tron, 0)
        .unwrap()
        .to_string();
    assert_eq!(a, LEDGER_ADDR);
    assert_ne!(a, b, "passphrase was not applied");
}

/// Wallets and their addresses must survive a lock/unlock cycle.
#[test]
fn wallets_survive_reopening_the_vault() {
    let dir = tempfile::tempdir().unwrap();
    let (vf, path) = vault(dir.path());

    let address;
    {
        let mut s = vf.create(EMAIL, PW, profile::TESTONLY).unwrap();
        let id = s
            .create_wallet(
                "cold",
                NewWalletSpec::ImportMnemonic {
                    phrase: LEDGER_PHRASE,
                    passphrase: None,
                },
            )
            .unwrap();
        address = s.address_of(id, ChainId::Tron, 0).unwrap().to_string();
    }

    let s = VaultFile::at(&path).unlock(EMAIL, PW).unwrap();
    let list = s.list_wallets().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].label, "cold");
    assert_eq!(list[0].address(ChainId::Tron), address);
}

/// Creating a wallet must not hand back the phrase; revealing it later requires
/// the password again, even though the session is already unlocked.
#[test]
fn revealing_a_mnemonic_requires_the_password_again() {
    let dir = tempfile::tempdir().unwrap();
    let (vf, path) = vault(dir.path());
    let mut s = vf.create(EMAIL, PW, profile::TESTONLY).unwrap();

    let id = s
        .create_wallet(
            "ledger",
            NewWalletSpec::ImportMnemonic {
                phrase: LEDGER_PHRASE,
                passphrase: None,
            },
        )
        .unwrap();

    assert!(matches!(
        s.reveal_mnemonic(&path, id, "the wrong password entirely"),
        Err(CoreError::WrongCredentials)
    ));

    let phrase = s.reveal_mnemonic(&path, id, PW).unwrap();
    assert_eq!(&*phrase, LEDGER_PHRASE);
}

/// A private-key import has no words, and the UI must be told so explicitly
/// rather than shown something misleading.
#[test]
fn private_key_wallets_report_that_they_have_no_phrase() {
    let dir = tempfile::tempdir().unwrap();
    let (vf, path) = vault(dir.path());
    let mut s = vf.create(EMAIL, PW, profile::TESTONLY).unwrap();

    let id = s
        .create_wallet("k", NewWalletSpec::ImportPrivateKey { hex: LEDGER_KEY })
        .unwrap();
    assert!(matches!(
        s.reveal_mnemonic(&path, id, PW),
        Err(CoreError::NoMnemonic)
    ));
}

/// After a password change the reveal must accept the NEW password only.
#[test]
fn reveal_follows_a_password_change() {
    let dir = tempfile::tempdir().unwrap();
    let (vf, path) = vault(dir.path());
    let mut s = vf.create(EMAIL, PW, profile::TESTONLY).unwrap();
    let id = s
        .create_wallet(
            "w",
            NewWalletSpec::ImportMnemonic {
                phrase: LEDGER_PHRASE,
                passphrase: None,
            },
        )
        .unwrap();

    s.change_password(&path, PW2).unwrap();
    assert!(matches!(
        s.reveal_mnemonic(&path, id, PW),
        Err(CoreError::WrongCredentials)
    ));
    assert_eq!(&*s.reveal_mnemonic(&path, id, PW2).unwrap(), LEDGER_PHRASE);
}

#[test]
fn rename_and_delete() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = fresh(dir.path());

    let a = s
        .create_wallet("wallet 1", NewWalletSpec::Generate { words: 12 })
        .unwrap();
    let b = s
        .create_wallet("wallet 2", NewWalletSpec::Generate { words: 12 })
        .unwrap();

    s.rename_wallet(a, "公司备用金").unwrap();
    let list = s.list_wallets().unwrap();
    assert_eq!(list.iter().find(|w| w.id == a).unwrap().label, "公司备用金");

    s.delete_wallet(b).unwrap();
    assert_eq!(s.list_wallets().unwrap().len(), 1);
    assert!(s.delete_wallet(b).is_err(), "deleting twice should fail");
}

/// Unlimited wallets, each with a distinct address.
#[test]
fn many_wallets_each_get_a_distinct_address() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = fresh(dir.path());

    for i in 0..12 {
        s.create_wallet(&format!("钱包{i}"), NewWalletSpec::Generate { words: 12 })
            .unwrap();
    }
    let list = s.list_wallets().unwrap();
    assert_eq!(list.len(), 12);
    let uniq: std::collections::HashSet<_> =
        list.iter().map(|w| w.address(ChainId::Tron)).collect();
    assert_eq!(uniq.len(), 12, "address collision across wallets");
}

/// Ciphertext is AAD-bound to its row: moving one wallet's sealed entropy onto
/// another row must fail to decrypt rather than silently swapping wallets.
#[test]
fn sealed_columns_cannot_be_swapped_between_rows() {
    let dir = tempfile::tempdir().unwrap();
    let (mut s, _) = fresh(dir.path());

    let a = s
        .create_wallet("a", NewWalletSpec::Generate { words: 12 })
        .unwrap();
    let b = s
        .create_wallet("b", NewWalletSpec::Generate { words: 12 })
        .unwrap();

    let stolen: Vec<u8> = s
        .conn()
        .unwrap()
        .query_row("SELECT entropy_ct FROM wallets WHERE id = ?1", [a], |r| {
            r.get(0)
        })
        .unwrap();
    s.conn()
        .unwrap()
        .execute(
            "UPDATE wallets SET entropy_ct = ?1 WHERE id = ?2",
            rusqlite::params![stolen, b],
        )
        .unwrap();

    assert!(
        s.address_of(b, ChainId::Tron, 0).is_err(),
        "row swap was accepted"
    );
}

/// Nothing secret may appear in the file, even after wallets exist.
#[test]
fn wallet_secrets_do_not_leak_into_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let (vf, path) = vault(dir.path());
    {
        let mut s = vf.create(EMAIL, PW, profile::TESTONLY).unwrap();
        s.create_wallet(
            "公司备用金",
            NewWalletSpec::ImportMnemonic {
                phrase: LEDGER_PHRASE,
                passphrase: Some("trezor"),
            },
        )
        .unwrap();
    }
    let bytes = std::fs::read(&path).unwrap();
    for needle in [
        b"abandon abandon".as_slice(),
        "公司备用金".as_bytes(),
        b"trezor".as_slice(),
        LEDGER_ADDR.as_bytes(),
    ] {
        assert!(
            !bytes.windows(needle.len()).any(|w| w == needle),
            "leak: {:?}",
            String::from_utf8_lossy(needle)
        );
    }
}
