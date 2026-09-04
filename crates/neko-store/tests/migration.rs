//! Upgrading a vault that already holds funds.
//!
//! The databases that need migrating are the ones somebody is using. A fresh
//! install exercises none of this, so these tests build a schema-1 database by
//! hand - the way it existed before BNB Chain - fill it, and check that
//! everything is still there afterwards.

use rusqlite::Connection;

/// The parts of schema 1 these tests touch, exactly as they were.
const SCHEMA_V1: &str = "
CREATE TABLE chains (
  id INTEGER PRIMARY KEY, slug TEXT NOT NULL UNIQUE,
  coin_type INTEGER NOT NULL, enabled INTEGER NOT NULL DEFAULT 1);
CREATE TABLE assets (
  id INTEGER PRIMARY KEY, chain_id INTEGER NOT NULL REFERENCES chains(id),
  symbol TEXT NOT NULL, contract BLOB, decimals INTEGER NOT NULL,
  enabled INTEGER NOT NULL DEFAULT 1, UNIQUE (chain_id, symbol));
CREATE TABLE wallets (id INTEGER PRIMARY KEY, seq INTEGER NOT NULL UNIQUE);
CREATE TABLE accounts (
  id INTEGER PRIMARY KEY,
  wallet_id INTEGER NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
  chain_id INTEGER NOT NULL REFERENCES chains(id),
  account_index INTEGER NOT NULL DEFAULT 0,
  UNIQUE (wallet_id, chain_id, account_index));
CREATE TABLE addresses (
  id INTEGER PRIMARY KEY,
  account_id INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  deriv_index INTEGER NOT NULL CHECK (deriv_index BETWEEN 0 AND 16777215),
  change INTEGER NOT NULL DEFAULT 0,
  address TEXT NOT NULL,
  address_raw BLOB NOT NULL CHECK (length(address_raw) = 21),
  label_ct BLOB,
  UNIQUE (account_id, change, deriv_index));
CREATE TABLE balances (
  address_id INTEGER NOT NULL REFERENCES addresses(id) ON DELETE CASCADE,
  asset_id INTEGER NOT NULL REFERENCES assets(id),
  amount BLOB NOT NULL, pending BLOB, updated_at INTEGER NOT NULL,
  PRIMARY KEY (address_id, asset_id));
INSERT INTO chains (id, slug, coin_type, enabled) VALUES (1, 'tron', 195, 1);
INSERT INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (1, 1, 'TRX', NULL, 6, 1);
PRAGMA user_version = 1;
";

const TRON_ADDR: &str = "TPZrDZTUWQqqUTVRxAmSdQyGXSSgAUyyk4";

fn v1_with_a_funded_wallet() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(SCHEMA_V1).unwrap();
    let raw = neko_hd::Address::parse(TRON_ADDR)
        .unwrap()
        .as_bytes()
        .to_vec();
    conn.execute("INSERT INTO wallets (id, seq) VALUES (7, 1)", [])
        .unwrap();
    conn.execute(
        "INSERT INTO accounts (id, wallet_id, chain_id, account_index) VALUES (3, 7, 1, 0)",
        [],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO addresses (id, account_id, deriv_index, change, address, address_raw)
         VALUES (11, 3, 0, 0, ?1, ?2)",
        rusqlite::params![TRON_ADDR, raw],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO balances (address_id, asset_id, amount, pending, updated_at)
         VALUES (11, 1, ?1, NULL, 1756000000)",
        rusqlite::params![8_655_007i128],
    )
    .unwrap();
    conn
}

/// The test that matters: a wallet with a balance goes through the upgrade and
/// comes out unchanged, ids included - `balances` points at `addresses.id`, and
/// the table is rebuilt during the migration.
#[test]
fn an_existing_wallet_survives_the_upgrade() {
    let conn = v1_with_a_funded_wallet();
    assert_eq!(neko_store::vault_row::schema_version(&conn).unwrap(), 1);

    neko_store::migrate::run(&conn).unwrap();
    assert_eq!(
        neko_store::vault_row::schema_version(&conn).unwrap(),
        neko_store::vault_row::CURRENT_SCHEMA,
    );

    let (addr, raw, id): (String, Vec<u8>, i64) = conn
        .query_row(
            "SELECT address, address_raw, id FROM addresses WHERE id = 11",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(addr, TRON_ADDR, "the address text changed");
    assert_eq!(raw.len(), 21, "the raw address changed");
    assert_eq!(id, 11, "the row id changed - balances now point at nothing");

    let amount: i128 = conn
        .query_row(
            "SELECT amount FROM balances WHERE address_id = 11",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(amount, 8_655_007, "the cached balance was lost");

    // The account row still joins through.
    let joined: i64 = conn
        .query_row(
            "SELECT ac.wallet_id FROM addresses a
             JOIN accounts ac ON ac.id = a.account_id WHERE a.id = 11",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(joined, 7, "the address lost its account");
}

/// After the upgrade the new chain exists and EVM's 20-byte addresses fit,
/// while a length that is neither is still refused.
#[test]
fn the_new_chain_is_usable_afterwards() {
    let conn = v1_with_a_funded_wallet();
    neko_store::migrate::run(&conn).unwrap();

    let (slug, coin): (String, i64) = conn
        .query_row("SELECT slug, coin_type FROM chains WHERE id = 2", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .unwrap();
    assert_eq!((slug.as_str(), coin), ("bsc", 60));

    let (sym, dec): (String, i64) = conn
        .query_row(
            "SELECT symbol, decimals FROM assets WHERE chain_id = 2",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!((sym.as_str(), dec), ("BNB", 18));

    conn.execute(
        "INSERT INTO accounts (id, wallet_id, chain_id, account_index) VALUES (4, 7, 2, 0)",
        [],
    )
    .unwrap();
    let evm = neko_hd::EvmAddress::parse("0x55d398326f99059fF775485246999027B3197955").unwrap();
    conn.execute(
        "INSERT INTO addresses (account_id, deriv_index, change, address, address_raw)
         VALUES (4, 0, 0, ?1, ?2)",
        rusqlite::params![evm.to_string(), evm.as_bytes().to_vec()],
    )
    .expect("a 20-byte EVM address was refused after the migration");

    // The check was widened, not removed: a truncated address is still caught.
    let bad = conn.execute(
        "INSERT INTO addresses (account_id, deriv_index, change, address, address_raw)
         VALUES (4, 1, 0, 'x', ?1)",
        rusqlite::params![vec![0u8; 19]],
    );
    assert!(bad.is_err(), "a 19-byte address was accepted");
}

/// Running the upgrade on a database that has already had it must be a no-op,
/// because it happens on every open.
#[test]
fn the_upgrade_is_idempotent() {
    let conn = v1_with_a_funded_wallet();
    neko_store::migrate::run(&conn).unwrap();
    let before: i64 = conn
        .query_row("SELECT count(*) FROM addresses", [], |r| r.get(0))
        .unwrap();
    neko_store::migrate::run(&conn).unwrap();
    neko_store::migrate::run(&conn).unwrap();
    let after: i64 = conn
        .query_row("SELECT count(*) FROM addresses", [], |r| r.get(0))
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(
        neko_store::vault_row::schema_version(&conn).unwrap(),
        neko_store::vault_row::CURRENT_SCHEMA,
    );
}

/// A Solana address is 32 bytes, which the pre-Solana CHECK constraint refused.
///
/// The point of this test is the *before*: without it, a migration that forgot
/// to widen the column would still pass every other test here and only fail
/// when somebody created their first Solana account.
#[test]
fn the_column_only_accepts_a_solana_address_after_the_migration() {
    let solana_raw = vec![9u8; 32];
    let solana_addr = "So1anaAddressPlaceholderThatIsNotParsedHere1";

    let conn = v1_with_a_funded_wallet();

    // Hung off the existing account, because chain 3 does not exist yet and the
    // foreign key would fail for a reason that has nothing to do with width.
    let insert = |conn: &Connection| {
        conn.execute(
            "INSERT INTO addresses (id, account_id, deriv_index, change, address, address_raw)
             VALUES (12, 3, 1, 0, ?1, ?2)",
            rusqlite::params![solana_addr, solana_raw],
        )
    };

    assert!(
        insert(&conn).is_err(),
        "32 bytes should not fit the old constraint - this test proves nothing otherwise"
    );

    neko_store::migrate::run(&conn).unwrap();
    insert(&conn).expect("a Solana address should fit after the migration");

    // The chain the widening was for now exists, and takes one too.
    conn.execute(
        "INSERT INTO accounts (id, wallet_id, chain_id, account_index) VALUES (4, 7, 3, 0)",
        [],
    )
    .unwrap();

    // And the widening did not open the column to anything at all.
    assert!(
        conn.execute(
            "INSERT INTO addresses (id, account_id, deriv_index, change, address, address_raw)
             VALUES (13, 4, 0, 0, 'x', ?1)",
            rusqlite::params![vec![0u8; 31]],
        )
        .is_err(),
        "31 bytes is not an address on any chain this wallet knows"
    );
}

/// The chain and its native coin have to be registered, or a Solana account
/// has nothing to hang a balance off.
#[test]
fn the_migration_registers_solana_and_sol() {
    let conn = v1_with_a_funded_wallet();
    neko_store::migrate::run(&conn).unwrap();

    let (slug, coin): (String, i64) = conn
        .query_row("SELECT slug, coin_type FROM chains WHERE id = 3", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("no solana row in chains");
    assert_eq!(slug, "solana");
    assert_eq!(coin, 501, "Solana has its own coin type, not Ethereum's 60");

    let (sym, dec): (String, i64) = conn
        .query_row(
            "SELECT symbol, decimals FROM assets WHERE chain_id = 3 AND contract IS NULL",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .expect("no native asset for solana");
    assert_eq!(sym, "SOL");
    assert_eq!(dec, 9, "SOL is quoted in lamports: 1e9");
}

/// Bitcoin stores the locking script, and its length is the address type. Every
/// one of the four has to fit, or four of the five address types are unusable.
#[test]
fn the_column_accepts_every_bitcoin_script_length() {
    let conn = v1_with_a_funded_wallet();
    neko_store::migrate::run(&conn).unwrap();
    conn.execute(
        "INSERT INTO accounts (id, wallet_id, chain_id, account_index) VALUES (5, 7, 4, 0)",
        [],
    )
    .unwrap();

    // P2WPKH, P2SH, P2PKH, and P2WSH/Taproot.
    for (i, len) in neko_store::repo::addresses::BITCOIN_SCRIPT_LENS
        .iter()
        .enumerate()
    {
        conn.execute(
            "INSERT INTO addresses (account_id, deriv_index, change, address, address_raw)
             VALUES (5, ?1, 0, ?2, ?3)",
            rusqlite::params![i as i64, format!("bc1-{len}"), vec![1u8; *len]],
        )
        .unwrap_or_else(|e| panic!("a {len}-byte script was refused: {e}"));
    }

    // And the widening did not open the column to anything at all.
    for bad in [19usize, 21 + 1, 26, 33, 35, 64] {
        if neko_store::repo::addresses::BITCOIN_SCRIPT_LENS.contains(&bad)
            || [20, 21, 32].contains(&bad)
        {
            continue;
        }
        assert!(
            conn.execute(
                "INSERT INTO addresses (account_id, deriv_index, change, address, address_raw)
                 VALUES (5, 900, 0, 'x', ?1)",
                rusqlite::params![vec![0u8; bad]],
            )
            .is_err(),
            "{bad} bytes is not an address on any chain this wallet knows"
        );
    }
}

/// Bitcoin has one asset. A second row would put an empty line on the assets
/// screen for a token that does not exist on this chain.
#[test]
fn the_migration_registers_bitcoin_with_one_asset() {
    let conn = v1_with_a_funded_wallet();
    neko_store::migrate::run(&conn).unwrap();

    let (slug, coin): (String, i64) = conn
        .query_row("SELECT slug, coin_type FROM chains WHERE id = 4", [], |r| {
            Ok((r.get(0)?, r.get(1)?))
        })
        .expect("no bitcoin row in chains");
    assert_eq!(slug, "bitcoin");
    assert_eq!(coin, 0, "Bitcoin is SLIP-44 coin type 0");

    let n: i64 = conn
        .query_row("SELECT count(*) FROM assets WHERE chain_id = 4", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(n, 1, "Bitcoin carries exactly one asset");

    let (sym, dec): (String, i64) = conn
        .query_row(
            "SELECT symbol, decimals FROM assets WHERE chain_id = 4",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(sym, "BTC");
    assert_eq!(dec, 8, "satoshis: 1e8");
}
