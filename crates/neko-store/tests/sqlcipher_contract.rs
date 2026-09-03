//! Pins the SQLCipher behaviours the neko-wallet file format DEPENDS on.
//!
//! Every assertion here corresponds to a design decision in the plan. If one
//! fails, the storage design is invalid — fix the design, do not weaken the test.

use rusqlite::Connection;
use std::fs;
use std::io::Read;

const KEY_HEX: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const KEY2_HEX: &str = "a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf";
/// Our 16-byte plaintext header: fmt_ver=01, kdf_profile=02, then 14 "random" bytes.
const HDR_HEX: &str = "0102aabbccddeeff00112233445566";
const HDR_FULL: &str = "0102aabbccddeeff001122334455667788";

fn hdr16() -> [u8; 16] {
    let mut h = [0u8; 16];
    h[0] = 0x01; // fmt_ver
    h[1] = 0x02; // kdf_profile = BALANCED
    for (i, b) in h[2..].iter_mut().enumerate() {
        *b = 0xA0 + i as u8;
    }
    h
}
fn hex16(h: &[u8; 16]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

/// 92-hex keyspec: x'<64 hex key><32 hex salt>' — sets key AND salt before any file I/O.
fn key_with_salt(conn: &Connection, key_hex: &str, salt_hex: &str) {
    conn.execute_batch(&format!("PRAGMA key = \"x'{key_hex}{salt_hex}'\";"))
        .expect("key+salt keyspec rejected");
}
/// 64-hex keyspec: raw key only, salt read from the file header.
fn key_only(conn: &Connection, key_hex: &str) -> rusqlite::Result<()> {
    conn.execute_batch(&format!("PRAGMA key = \"x'{key_hex}'\";"))
}

fn first16(path: &std::path::Path) -> [u8; 16] {
    let mut f = fs::File::open(path).unwrap();
    let mut buf = [0u8; 16];
    f.read_exact(&mut buf).unwrap();
    buf
}

fn seed_schema(conn: &Connection) {
    conn.execute_batch(
        "PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = FULL;
         CREATE TABLE secret (id INTEGER PRIMARY KEY, mnemonic TEXT NOT NULL);
         INSERT INTO secret (id, mnemonic) VALUES (1, 'abandon abandon about');",
    )
    .unwrap();
}

fn read_secret(conn: &Connection) -> rusqlite::Result<String> {
    conn.query_row("SELECT mnemonic FROM secret WHERE id = 1", [], |r| r.get(0))
}

#[test]
fn sqlcipher_is_actually_linked() {
    let conn = Connection::open_in_memory().unwrap();
    let v: String = conn
        .query_row("PRAGMA cipher_version", [], |r| r.get(0))
        .expect("no cipher_version => plain SQLite, NOT SQLCipher");
    eprintln!("SQLCipher version: {v}");
    assert!(v.starts_with("4."), "unexpected SQLCipher major: {v}");
}

/// CORE OF THE DESIGN: the 92-hex keyspec must write OUR 16 bytes into the file
/// header, so the Argon2 salt travels inside the .db and is readable without a key.
#[test]
fn our_header_lands_in_first_16_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    let hdr = hdr16();

    let conn = Connection::open(&db).unwrap();
    key_with_salt(&conn, KEY_HEX, &hex16(&hdr));
    seed_schema(&conn);
    drop(conn);

    assert_eq!(
        first16(&db),
        hdr,
        "SQLCipher did not persist our salt into bytes 0..16"
    );
}

/// The whole point of "copy the .db anywhere": reopen with key-only (64-hex),
/// salt recovered from the file itself.
#[test]
fn reopen_with_key_only_using_file_salt() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    let hdr = hdr16();

    let conn = Connection::open(&db).unwrap();
    key_with_salt(&conn, KEY_HEX, &hex16(&hdr));
    seed_schema(&conn);
    drop(conn);

    let conn = Connection::open(&db).unwrap();
    key_only(&conn, KEY_HEX).unwrap();
    assert_eq!(read_secret(&conn).unwrap(), "abandon abandon about");
}

/// THE ACCEPTANCE TEST FOR THE WHOLE DESIGN: copy the file elsewhere, open it there.
#[test]
fn copied_db_file_still_opens() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    let moved = dir.path().join("sub").join("moved-copy.db");
    fs::create_dir_all(moved.parent().unwrap()).unwrap();
    let hdr = hdr16();

    let conn = Connection::open(&db).unwrap();
    key_with_salt(&conn, KEY_HEX, &hex16(&hdr));
    seed_schema(&conn);
    drop(conn);

    fs::copy(&db, &moved).unwrap();

    let conn = Connection::open(&moved).unwrap();
    key_only(&conn, KEY_HEX).unwrap();
    assert_eq!(read_secret(&conn).unwrap(), "abandon abandon about");
}

/// WAL would split state across -wal/-shm sidecars and silently lose commits
/// when only the .db is copied. journal_mode=DELETE must leave no sidecars.
#[test]
fn no_wal_sidecars_after_clean_close() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");

    let conn = Connection::open(&db).unwrap();
    key_with_salt(&conn, KEY_HEX, &hex16(&hdr16()));
    seed_schema(&conn);
    drop(conn);

    for ext in ["-wal", "-shm"] {
        let p = db.with_file_name(format!("w.db{ext}"));
        assert!(!p.exists(), "unexpected sidecar {p:?}");
    }
}

/// PRAGMA rekey (64-hex form) must preserve the file salt, or every password
/// change would orphan the database.
#[test]
fn rekey_preserves_file_salt() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    let hdr = hdr16();

    let conn = Connection::open(&db).unwrap();
    key_with_salt(&conn, KEY_HEX, &hex16(&hdr));
    seed_schema(&conn);
    conn.execute_batch(&format!("PRAGMA rekey = \"x'{KEY2_HEX}'\";"))
        .expect("rekey failed");
    drop(conn);

    assert_eq!(first16(&db), hdr, "rekey changed the file salt");

    let conn = Connection::open(&db).unwrap();
    key_only(&conn, KEY2_HEX).unwrap();
    assert_eq!(read_secret(&conn).unwrap(), "abandon abandon about");
    drop(conn);

    let conn = Connection::open(&db).unwrap();
    let _ = key_only(&conn, KEY_HEX);
    assert!(
        read_secret(&conn).is_err(),
        "old key still opens after rekey"
    );
}

#[test]
fn wrong_key_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");

    let conn = Connection::open(&db).unwrap();
    key_with_salt(&conn, KEY_HEX, &hex16(&hdr16()));
    seed_schema(&conn);
    drop(conn);

    let conn = Connection::open(&db).unwrap();
    let _ = key_only(&conn, KEY2_HEX);
    assert!(read_secret(&conn).is_err(), "wrong key opened the database");
}

/// A one-byte edit anywhere in the payload must fail the page HMAC, not silently
/// return garbage.
#[test]
fn tampered_file_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");

    let conn = Connection::open(&db).unwrap();
    key_with_salt(&conn, KEY_HEX, &hex16(&hdr16()));
    seed_schema(&conn);
    drop(conn);

    let mut bytes = fs::read(&db).unwrap();
    let n = bytes.len();
    bytes[n - 32] ^= 0xFF; // flip a byte well past the plaintext header
    fs::write(&db, &bytes).unwrap();

    let conn = Connection::open(&db).unwrap();
    let _ = key_only(&conn, KEY_HEX);
    assert!(
        read_secret(&conn).is_err(),
        "tampered database still readable"
    );
}

/// VERIFIED 2026-09-02 on SQLCipher 4.14.0: contrary to what we expected,
/// `VACUUM INTO` DOES carry the codec's kdf_salt over to the new file, so the
/// export opens with the same key. That makes it a supported compaction/export
/// path rather than the footgun the plan assumed. Pinned here so a future
/// SQLCipher change that breaks it is caught immediately.
#[test]
fn vacuum_into_preserves_salt_and_key() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    let out = dir.path().join("exported.db");
    let hdr = hdr16();

    let conn = Connection::open(&db).unwrap();
    key_with_salt(&conn, KEY_HEX, &hex16(&hdr));
    seed_schema(&conn);
    conn.execute_batch(&format!("VACUUM INTO '{}';", out.display()))
        .unwrap();
    drop(conn);

    assert_eq!(
        first16(&out),
        hdr,
        "VACUUM INTO changed the salt -- exports would be unopenable; block the code path"
    );

    let conn = Connection::open(&out).unwrap();
    key_only(&conn, KEY_HEX).expect("exported copy rejected the original key");
    assert_eq!(read_secret(&conn).unwrap(), "abandon abandon about");
    drop(conn);

    // ...and it must still reject the wrong key.
    let conn = Connection::open(&out).unwrap();
    let _ = key_only(&conn, KEY2_HEX);
    assert!(
        read_secret(&conn).is_err(),
        "exported copy opened with wrong key"
    );
}

/// In-place VACUUM must NOT change the salt (we may want it for compaction).
#[test]
fn inplace_vacuum_preserves_salt() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    let hdr = hdr16();

    let conn = Connection::open(&db).unwrap();
    key_with_salt(&conn, KEY_HEX, &hex16(&hdr));
    seed_schema(&conn);
    conn.execute_batch("VACUUM;").unwrap();
    drop(conn);

    assert_eq!(first16(&db), hdr, "in-place VACUUM changed the file salt");

    let conn = Connection::open(&db).unwrap();
    key_only(&conn, KEY_HEX).unwrap();
    assert_eq!(read_secret(&conn).unwrap(), "abandon abandon about");
}

/// Sanity: unused consts referenced so the file documents the keyspec shapes.
#[test]
fn keyspec_shapes_documented() {
    assert_eq!(KEY_HEX.len(), 64, "raw key is 64 hex chars");
    assert_eq!(hex16(&hdr16()).len(), 32, "salt is 32 hex chars");
    assert_eq!(KEY_HEX.len() + 32, 96, "key+salt keyspec is 96 hex chars");
    let _ = (HDR_HEX, HDR_FULL);
}

/// SQLCipher locks its key material with `mlock`/`VirtualLock` from inside
/// `sqlcipher_malloc`, unconditionally. On Windows that is charged against the
/// process working set quota, and exhausting the quota breaks page commits
/// elsewhere - a thread stack growing then fails as `STATUS_STACK_OVERFLOW`,
/// which is how this surfaced: a crash that looked like infinite recursion in
/// code that has none.
///
/// So the vault must be openable repeatedly, with the key derivation that
/// precedes it, without the process running out of quota. On Unix this is
/// unremarkable; on Windows it is the regression test for that failure.
#[test]
fn repeated_opens_do_not_exhaust_the_page_lock_quota() {
    let dir = tempfile::tempdir().unwrap();
    let header = neko_vault::FileHeader::new(neko_vault::profile::TESTONLY).unwrap();
    let stretched = neko_vault::keys::stretch(
        "zoe@example.com",
        "correct horse battery staple xyzzy",
        &header,
    )
    .unwrap();
    let key = neko_vault::keys::file_key(&stretched).unwrap();

    // Several vaults, opened and reopened. One is not enough: the quota is
    // consumed cumulatively, which is why the original failure hit whichever
    // test happened to run after the others had used it up.
    for i in 0..8 {
        let path = dir.path().join(format!("v{i}.db"));
        {
            let conn = neko_store::open::create(&path, &key, &header).unwrap();
            conn.execute_batch("CREATE TABLE t (x BLOB); INSERT INTO t VALUES (randomblob(4096));")
                .unwrap();
        }
        let conn = neko_store::open::open(&path, &key, &header).unwrap();
        let n: i64 = conn
            .query_row("SELECT count(*) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 1, "vault {i} did not reopen cleanly");
    }
}
