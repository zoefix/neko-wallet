//! Address registration and the balance cache.

use neko_core::{NewWalletSpec, VaultFile};
use neko_vault::profile;

const EMAIL: &str = "zoe@example.com";
const PW: &str = "correct horse battery staple xyzzy";
const LEDGER_PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const LEDGER_ADDR: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";

fn session(dir: &std::path::Path) -> neko_core::Session {
    VaultFile::at(dir.join("w.db"))
        .create(EMAIL, PW, profile::TESTONLY)
        .unwrap()
}

#[test]
fn creating_a_wallet_registers_its_address() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = s
        .create_wallet(
            "ledger",
            NewWalletSpec::ImportMnemonic {
                phrase: LEDGER_PHRASE,
                passphrase: None,
            },
        )
        .unwrap();

    let rows = neko_store::repo::addresses::for_wallet(s.conn().unwrap(), id).unwrap();
    assert_eq!(rows.len(), 1, "no address row was created");
    assert_eq!(rows[0].address, LEDGER_ADDR);
    assert_eq!(rows[0].address_raw.len(), 21);
}

/// Base58 and the raw bytes must always describe the same address. A corrupted
/// hex is usually still a *valid* address, so nothing else catches this.
#[test]
fn address_drift_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = s
        .create_wallet(
            "ledger",
            NewWalletSpec::ImportMnemonic {
                phrase: LEDGER_PHRASE,
                passphrase: None,
            },
        )
        .unwrap();
    assert_eq!(s.verify_address_consistency().unwrap(), 1);

    // Corrupt the raw bytes the way a hand-written SQL fix would.
    let mut raw = neko_store::repo::addresses::for_wallet(s.conn().unwrap(), id).unwrap()[0]
        .address_raw
        .clone();
    raw[5] ^= 0xFF;
    s.conn()
        .unwrap()
        .execute(
            "UPDATE addresses SET address_raw = ?1",
            rusqlite::params![raw],
        )
        .unwrap();

    assert!(
        s.verify_address_consistency().is_err(),
        "a drifted address passed the consistency check"
    );
}

#[test]
fn balances_round_trip_through_the_cache() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = s
        .create_wallet("w", NewWalletSpec::Generate { words: 12 })
        .unwrap();

    assert!(s.cached_assets(id).unwrap().rows.is_empty());
    assert!(s.cached_assets(id).unwrap().updated_at.is_none());

    s.cache_assets(
        id,
        &[
            ("TRX".into(), 6, 1_500_000),
            // 2^53 + 1: must survive the round trip exactly.
            ("USDT".into(), 6, 9_007_199_254_740_993),
        ],
    )
    .unwrap();

    let cached = s.cached_assets(id).unwrap();
    assert_eq!(cached.amount("TRX"), Some((1_500_000, 6)));
    assert_eq!(cached.amount("USDT"), Some((9_007_199_254_740_993, 6)));
    assert!(cached.updated_at.is_some(), "no timestamp recorded");
}

#[test]
fn caching_again_overwrites_rather_than_duplicating() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = s
        .create_wallet("w", NewWalletSpec::Generate { words: 12 })
        .unwrap();

    s.cache_assets(id, &[("TRX".into(), 6, 100)]).unwrap();
    s.cache_assets(id, &[("TRX".into(), 6, 200)]).unwrap();

    let cached = s.cached_assets(id).unwrap();
    assert_eq!(cached.rows.len(), 1, "duplicate balance rows");
    assert_eq!(cached.amount("TRX"), Some((200, 6)));
}

/// The cache must survive a lock/unlock cycle: that is the whole point.
#[test]
fn the_cache_survives_reopening_the_vault() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("w.db");
    let id;
    {
        let mut s = VaultFile::at(&path)
            .create(EMAIL, PW, profile::TESTONLY)
            .unwrap();
        id = s
            .create_wallet("w", NewWalletSpec::Generate { words: 12 })
            .unwrap();
        s.cache_assets(id, &[("USDT".into(), 6, 12_345_678)])
            .unwrap();
    }
    let s = VaultFile::at(&path).unlock(EMAIL, PW).unwrap();
    assert_eq!(
        s.cached_assets(id).unwrap().amount("USDT"),
        Some((12_345_678, 6))
    );
}

/// Wallets created before address bookkeeping existed must be adopted silently.
#[test]
fn wallets_without_an_address_row_are_backfilled() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let id = s
        .create_wallet(
            "ledger",
            NewWalletSpec::ImportMnemonic {
                phrase: LEDGER_PHRASE,
                passphrase: None,
            },
        )
        .unwrap();

    // Simulate the older schema state.
    s.conn()
        .unwrap()
        .execute("DELETE FROM addresses", [])
        .unwrap();
    assert!(
        neko_store::repo::addresses::for_wallet(s.conn().unwrap(), id)
            .unwrap()
            .is_empty()
    );

    assert_eq!(s.backfill_addresses().unwrap(), 1);
    let rows = neko_store::repo::addresses::for_wallet(s.conn().unwrap(), id).unwrap();
    assert_eq!(
        rows[0].address, LEDGER_ADDR,
        "backfill derived the wrong address"
    );

    // Running it again must be a no-op, not a duplicate.
    assert_eq!(s.backfill_addresses().unwrap(), 0);
}

/// Each wallet's balances stay its own.
#[test]
fn balances_do_not_leak_between_wallets() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());
    let a = s
        .create_wallet("a", NewWalletSpec::Generate { words: 12 })
        .unwrap();
    let b = s
        .create_wallet("b", NewWalletSpec::Generate { words: 12 })
        .unwrap();

    s.cache_assets(a, &[("TRX".into(), 6, 111)]).unwrap();
    assert_eq!(s.cached_assets(a).unwrap().amount("TRX"), Some((111, 6)));
    assert!(
        s.cached_assets(b).unwrap().rows.is_empty(),
        "balances bled across wallets"
    );
}

/// Two wallets may legitimately hold the same address — importing the same key
/// twice, or importing a key a derived wallet already covers. That must not
/// break address registration.
#[test]
fn two_wallets_may_share_an_address() {
    const KEY: &str = "b5a4cea271ff424d7c31dc12a3e43e401df7a40d7412a15750f3f0b6b5449a28";
    let dir = tempfile::tempdir().unwrap();
    let mut s = session(dir.path());

    let a = s
        .create_wallet("first", NewWalletSpec::ImportPrivateKey { hex: KEY })
        .unwrap();
    let b = s
        .create_wallet("second", NewWalletSpec::ImportPrivateKey { hex: KEY })
        .unwrap();

    for id in [a, b] {
        let rows = neko_store::repo::addresses::for_wallet(s.conn().unwrap(), id).unwrap();
        assert_eq!(rows.len(), 1, "wallet {id} has no address row");
        assert_eq!(rows[0].address, LEDGER_ADDR);
    }
    // Each keeps its own cache entry rather than colliding.
    s.cache_assets(a, &[("TRX".into(), 6, 111)]).unwrap();
    s.cache_assets(b, &[("TRX".into(), 6, 222)]).unwrap();
    assert_eq!(s.cached_assets(a).unwrap().amount("TRX"), Some((111, 6)));
    assert_eq!(s.cached_assets(b).unwrap().amount("TRX"), Some((222, 6)));
}
