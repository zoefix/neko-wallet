//! End-to-end acceptance tests for the vault.
//!
//! The headline requirement: **the `.db` file is self-contained and portable.**
//! Copy it anywhere, drop it back, and email + password recovers everything.
//! Lose the password and it is gone forever, by design.

use neko_core::{CoreError, VaultFile};
use neko_vault::profile;
use std::fs;

const EMAIL: &str = "zoe@example.com";
const PW: &str = "correct horse battery staple xyzzy";
const PW2: &str = "plaid walrus trombone velvet 41";

/// Deliberately weak Argon2 so the suite runs in milliseconds. Production
/// vaults use BALANCED (256 MiB); see `neko_vault::profile`.
fn fast() -> profile::Profile {
    profile::TESTONLY
}

fn seed_secret(s: &neko_core::Session) {
    s.conn()
        .unwrap()
        .execute(
            "INSERT INTO wallets (id, seq, origin, entropy_ct, created_at)
             VALUES (1, 0, 'generated', ?1, 0)",
            rusqlite::params![b"pretend-sealed-entropy".to_vec()],
        )
        .unwrap();
}

fn read_secret(s: &neko_core::Session) -> Vec<u8> {
    s.conn()
        .unwrap()
        .query_row("SELECT entropy_ct FROM wallets WHERE id = 1", [], |r| {
            r.get(0)
        })
        .unwrap()
}

/// THE acceptance test for the entire storage design.
#[test]
fn db_file_is_portable() {
    let dir = tempfile::tempdir().unwrap();
    let original = dir.path().join("neko-wallet.db");

    {
        let s = VaultFile::at(&original).create(EMAIL, PW, fast()).unwrap();
        seed_secret(&s);
    } // Session dropped -> connection closed -> keys zeroized

    // Simulate the user copying the file to a USB stick, wiping the machine,
    // and dropping it back into a fresh install directory.
    let elsewhere = dir.path().join("usb").join("backup.db");
    fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
    fs::copy(&original, &elsewhere).unwrap();
    fs::remove_file(&original).unwrap();
    let restored = dir.path().join("fresh-install").join("neko-wallet.db");
    fs::create_dir_all(restored.parent().unwrap()).unwrap();
    fs::copy(&elsewhere, &restored).unwrap();

    let s = VaultFile::at(&restored)
        .unlock(EMAIL, PW)
        .expect("restored vault must unlock");
    assert_eq!(read_secret(&s), b"pretend-sealed-entropy");
}

#[test]
fn no_wal_sidecars_are_created() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    {
        let s = VaultFile::at(&db).create(EMAIL, PW, fast()).unwrap();
        seed_secret(&s);
    }
    neko_store::assert_no_wal_sidecars(&db).expect("WAL sidecars would break portability");
}

#[test]
fn wrong_password_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    drop(VaultFile::at(&db).create(EMAIL, PW, fast()).unwrap());

    let e = VaultFile::at(&db)
        .unlock(EMAIL, "correct horse battery stapleX")
        .unwrap_err();
    assert!(matches!(e, CoreError::WrongCredentials), "got {e:?}");
}

#[test]
fn wrong_email_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    drop(VaultFile::at(&db).create(EMAIL, PW, fast()).unwrap());

    let e = VaultFile::at(&db)
        .unlock("someone.else@example.com", PW)
        .unwrap_err();
    assert!(matches!(e, CoreError::WrongCredentials), "got {e:?}");
}

/// Neither error may reveal which input was wrong; that would be an oracle.
#[test]
fn wrong_email_and_wrong_password_are_indistinguishable() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    drop(VaultFile::at(&db).create(EMAIL, PW, fast()).unwrap());

    let a = VaultFile::at(&db)
        .unlock("nope@example.com", PW)
        .unwrap_err()
        .to_string();
    let b = VaultFile::at(&db)
        .unlock(EMAIL, "nope nope nope nope nope")
        .unwrap_err()
        .to_string();
    assert_eq!(a, b, "unlock errors distinguish email from password");
}

/// Email is normalized: case and surrounding whitespace must not matter.
#[test]
fn email_is_normalized() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    drop(VaultFile::at(&db).create(EMAIL, PW, fast()).unwrap());
    VaultFile::at(&db)
        .unlock("  ZoE@ExAmPle.COM  ", PW)
        .expect("email normalization broken");
}

/// Passwords are case-sensitive and NFKC-normalized. A password typed as NFD on
/// macOS must unlock a vault created with the NFC form, or users lose funds
/// while insisting -- correctly -- that they typed it right.
#[test]
fn password_unicode_form_does_not_matter() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    let nfc = "café trombone walrus plaid 41"; // é = U+00E9
    let nfd = "cafe\u{0301} trombone walrus plaid 41"; // e + combining acute
    assert_ne!(
        nfc.as_bytes(),
        nfd.as_bytes(),
        "test inputs must differ byte-wise"
    );

    drop(VaultFile::at(&db).create(EMAIL, nfc, fast()).unwrap());
    VaultFile::at(&db)
        .unlock(EMAIL, nfd)
        .expect("NFKC normalization is not being applied");
}

#[test]
fn password_remains_case_sensitive() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    drop(VaultFile::at(&db).create(EMAIL, PW, fast()).unwrap());
    assert!(VaultFile::at(&db)
        .unlock(EMAIL, &PW.to_uppercase())
        .is_err());
}

/// Flipping the plaintext profile byte must fail closed, never open with weaker
/// parameters.
#[test]
fn tampered_profile_byte_fails_closed() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    drop(VaultFile::at(&db).create(EMAIL, PW, fast()).unwrap());

    let mut bytes = fs::read(&db).unwrap();
    bytes[1] = profile::LIGHT.id;
    fs::write(&db, &bytes).unwrap();

    assert!(
        VaultFile::at(&db).unlock(EMAIL, PW).is_err(),
        "downgraded header still opened"
    );
}

#[test]
fn corrupted_file_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    drop(VaultFile::at(&db).create(EMAIL, PW, fast()).unwrap());

    // Flip a byte inside page 1's ciphertext (past the 16-byte plaintext
    // header). Page 1 is always read, so its HMAC is always checked -- unlike
    // the file tail, which may sit in a page no query touches.
    let mut bytes = fs::read(&db).unwrap();
    bytes[100] ^= 0xFF;
    fs::write(&db, &bytes).unwrap();

    assert!(
        VaultFile::at(&db).unlock(EMAIL, PW).is_err(),
        "corrupted vault opened"
    );
}

#[test]
fn weak_password_is_refused_at_setup() {
    let dir = tempfile::tempdir().unwrap();
    for (i, weak) in ["short", "password123", "aaaaaaaaaaaaaaaa", "Password2026!"]
        .iter()
        .enumerate()
    {
        let db = dir.path().join(format!("w{i}.db"));
        let e = VaultFile::at(&db).create(EMAIL, weak, fast()).unwrap_err();
        assert!(
            matches!(e, CoreError::WeakPassword(_)),
            "accepted weak password {weak:?}"
        );
        assert!(
            !db.exists(),
            "a rejected setup must not leave a file behind"
        );
    }
}

/// Changing the password rotates only the outer file key. MK -- and therefore
/// every wallet and every ciphertext -- is untouched.
#[test]
fn change_password_preserves_data_and_salt() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");

    let header_before;
    {
        let mut s = VaultFile::at(&db).create(EMAIL, PW, fast()).unwrap();
        seed_secret(&s);
        header_before = fs::read(&db).unwrap()[..16].to_vec();
        s.change_password(&db, PW2).unwrap();
    }

    assert_eq!(
        &fs::read(&db).unwrap()[..16],
        &header_before[..],
        "rekey changed the file salt"
    );

    assert!(
        VaultFile::at(&db).unlock(EMAIL, PW).is_err(),
        "old password still works"
    );
    let s = VaultFile::at(&db)
        .unlock(EMAIL, PW2)
        .expect("new password does not work");
    assert_eq!(
        read_secret(&s),
        b"pretend-sealed-entropy",
        "data lost during password change"
    );
}

/// The vault must still be recoverable if the process dies between staging the
/// new wrap and completing the rekey.
#[test]
fn interrupted_rewrap_is_recoverable() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");

    {
        let s = VaultFile::at(&db).create(EMAIL, PW, fast()).unwrap();
        seed_secret(&s);

        // Simulate a crash: stage the new wrap, then never rekey.
        let row = neko_store::vault_row::load(s.conn().unwrap())
            .unwrap()
            .unwrap();
        let header = *s.header();
        let stretched = neko_vault::keys::stretch(EMAIL, PW2, &header).unwrap();
        let kek = neko_vault::keys::kek(&stretched, &row.vault_salt).unwrap();
        let extra = neko_vault::keys::vault_aad_extra(row.params, &row.vault_salt);
        let new_wrapped = neko_vault::keys::wrap_mk(&kek, s.mk(), row.key_ver, &extra).unwrap();
        neko_store::vault_row::stage_rewrap(s.conn().unwrap(), &new_wrapped, 0).unwrap();
    }

    // The file is still under the OLD key, but wrapped_mk is the NEW one.
    // Recovery goes through wrapped_mk_prev.
    let s = VaultFile::at(&db)
        .unlock(EMAIL, PW)
        .expect("interrupted rewrap orphaned the vault");
    assert_eq!(read_secret(&s), b"pretend-sealed-entropy");
}

/// A locked session must not hand out a connection.
#[test]
fn locking_releases_the_connection() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");
    let mut s = VaultFile::at(&db).create(EMAIL, PW, fast()).unwrap();
    assert!(s.conn().is_ok());
    s.lock();
    assert!(matches!(s.conn(), Err(CoreError::Locked)));
}

/// The plaintext file header must be unique per database even for identical
/// credentials, so no work can be precomputed or shared across backups.
#[test]
fn each_vault_gets_unique_file_randomness() {
    let dir = tempfile::tempdir().unwrap();
    let mut seen = std::collections::HashSet::new();
    for i in 0..5 {
        let db = dir.path().join(format!("w{i}.db"));
        drop(VaultFile::at(&db).create(EMAIL, PW, fast()).unwrap());
        let hdr = fs::read(&db).unwrap()[..16].to_vec();
        assert_eq!(hdr[0], 1, "fmt_ver");
        assert_eq!(hdr[1], fast().id, "profile id");
        assert!(
            seen.insert(hdr[2..].to_vec()),
            "file_rand repeated across vaults"
        );
    }
}

/// Nothing recognisable may survive in the file. This is the property a user
/// checks with `strings wallet.db | grep -i ...` before trusting a backup.
#[test]
fn nothing_readable_leaks_into_the_file() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("w.db");

    const NEEDLES: &[&[u8]] = &[
        b"abandon abandon abandon",
        b"my-secret-wallet-label",
        b"zoe@example.com",
        b"correct horse battery staple",
        b"wallets", // even the schema must not be readable
        b"SQLite format 3",
    ];

    {
        let s = VaultFile::at(&db).create(EMAIL, PW, fast()).unwrap();
        s.conn()
            .unwrap()
            .execute(
                "INSERT INTO wallets (id, seq, origin, entropy_ct, label_ct, created_at)
                 VALUES (1, 0, 'generated', ?1, ?2, 0)",
                rusqlite::params![
                    b"abandon abandon abandon".to_vec(),
                    b"my-secret-wallet-label".to_vec()
                ],
            )
            .unwrap();
    }

    let bytes = fs::read(&db).unwrap();
    for needle in NEEDLES {
        assert!(
            !bytes.windows(needle.len()).any(|w| w == *needle),
            "plaintext leak: {:?} found in the database file",
            String::from_utf8_lossy(needle)
        );
    }
    // Sanity: the 16-byte header IS plaintext, by design.
    assert_eq!(bytes[0], 1, "fmt_ver should be readable without a key");
}
