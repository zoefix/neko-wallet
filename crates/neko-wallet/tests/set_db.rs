//! `neko-wallet set db` - choosing which vault to open, without unlocking it.
//!
//! Driven as a subprocess against the real binary, because the thing worth
//! testing is what a user typing the command actually gets: the exit status,
//! and whether a mistake is stopped before it is written down.

use std::path::Path;
use std::process::Command;

const EXE: &str = env!("CARGO_BIN_EXE_neko-wallet");

struct Env {
    _dir: tempfile::TempDir,
    config: std::path::PathBuf,
}

fn env() -> Env {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    Env { _dir: dir, config }
}

impl Env {
    fn run(&self, args: &[&str]) -> (bool, String) {
        let out = Command::new(EXE)
            .args(args)
            // Pinned so the test never reads or writes the real user's config.
            .env("NEKO_WALLET_CONFIG", &self.config)
            .env_remove("NEKO_WALLET_DB")
            .output()
            .unwrap();
        let mut text = String::from_utf8_lossy(&out.stdout).to_string();
        text.push_str(&String::from_utf8_lossy(&out.stderr));
        (out.status.success(), text)
    }

    fn where_db(&self) -> String {
        self.run(&["--where-db"]).1.trim().to_string()
    }
}

/// A file with a valid neko-wallet header. Not a working vault - nothing here
/// opens one - but enough for the checks this command performs.
fn vault_at(p: &Path) {
    let h = neko_vault::FileHeader::new(neko_vault::profile::LIGHT).unwrap();
    let mut bytes = h.as_bytes().to_vec();
    bytes.extend_from_slice(&[0u8; 8192]);
    std::fs::write(p, bytes).unwrap();
}

#[test]
fn setting_a_vault_makes_it_the_one_that_opens() {
    let e = env();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("main.db");
    vault_at(&db);

    let (ok, out) = e.run(&["set", "db", db.to_str().unwrap()]);
    assert!(ok, "set db failed: {out}");
    assert_eq!(
        e.where_db(),
        std::fs::canonicalize(&db).unwrap().to_string_lossy()
    );
}

/// The setting has to survive the process exiting - that is the whole point.
#[test]
fn the_choice_persists_across_runs() {
    let e = env();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("main.db");
    vault_at(&db);
    assert!(e.run(&["set", "db", db.to_str().unwrap()]).0);

    // A completely separate invocation.
    let first = e.where_db();
    let second = e.where_db();
    assert_eq!(first, second);
    assert!(first.ends_with("main.db"), "{first}");
}

/// The mistake this guard exists for. A typo would otherwise be recorded, the
/// next launch would show first-run setup, and a user who takes that at face
/// value creates a second empty wallet believing the first is gone.
#[test]
fn a_path_that_does_not_exist_is_refused() {
    let e = env();
    let before = e.where_db();

    let (ok, out) = e.run(&["set", "db", "/tmp/definitely/not/here.db"]);
    assert!(!ok, "a nonexistent path was accepted");
    assert!(
        out.contains("--new"),
        "the way forward is not offered: {out}"
    );
    assert_eq!(
        e.where_db(),
        before,
        "a refused command still changed the setting"
    );
}

/// ...but starting a new wallet somewhere is legitimate, with a word said.
#[test]
fn a_new_vault_can_be_started_when_asked_for_explicitly() {
    let e = env();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("fresh.db");

    let (ok, _) = e.run(&["set", "db", db.to_str().unwrap(), "--new"]);
    assert!(ok);
    assert!(e.where_db().ends_with("fresh.db"));
    assert!(!db.exists(), "set db must not create the file itself");
}

/// Pointing at the wrong kind of file must be caught here, not discovered as a
/// baffling failure at unlock time.
#[test]
fn files_that_are_not_vaults_are_refused() {
    let e = env();
    let dir = tempfile::tempdir().unwrap();

    // The single most likely wrong file, and the one whose generic header
    // error ("format v83") explains nothing.
    let sqlite = dir.path().join("other.db");
    let mut b = b"SQLite format 3\0".to_vec();
    b.extend_from_slice(&[0u8; 4096]);
    std::fs::write(&sqlite, b).unwrap();
    let (ok, out) = e.run(&["set", "db", sqlite.to_str().unwrap()]);
    assert!(!ok, "a plain SQLite database was accepted as a vault");
    assert!(
        out.contains("SQLite") || out.contains("unencrypted"),
        "the reason is not explained: {out}"
    );

    // Something far too short to be anything.
    let junk = dir.path().join("junk");
    std::fs::write(&junk, b"hi").unwrap();
    assert!(!e.run(&["set", "db", junk.to_str().unwrap()]).0);

    // A directory.
    assert!(!e.run(&["set", "db", dir.path().to_str().unwrap()]).0);
}

#[test]
fn unset_goes_back_to_the_default_search() {
    let e = env();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("main.db");
    vault_at(&db);

    assert!(e.run(&["set", "db", db.to_str().unwrap()]).0);
    assert!(e.where_db().ends_with("main.db"));

    let (ok, out) = e.run(&["unset", "db"]);
    assert!(ok, "{out}");
    assert!(
        !e.where_db().ends_with("main.db"),
        "the setting outlived unset"
    );
}

/// A relative path must be stored absolute, or it means a different file from
/// every directory the user later runs the command in.
#[test]
fn a_relative_path_is_stored_absolutely() {
    let e = env();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("rel.db");
    vault_at(&db);

    let out = Command::new(EXE)
        .args(["set", "db", "rel.db"])
        .current_dir(dir.path())
        .env("NEKO_WALLET_CONFIG", &e.config)
        .env_remove("NEKO_WALLET_DB")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    let saved = e.where_db();
    assert!(
        Path::new(&saved).is_absolute(),
        "stored as a relative path: {saved}"
    );
    assert!(saved.ends_with("rel.db"));
}

/// `--db` is a one-off look at another vault and must not disturb the saved
/// choice; `$NEKO_WALLET_DB` likewise outranks it without overwriting it.
#[test]
fn a_one_off_override_does_not_overwrite_the_setting() {
    let e = env();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("main.db");
    vault_at(&db);
    assert!(e.run(&["set", "db", db.to_str().unwrap()]).0);

    let other = dir.path().join("other.db");
    let (_, shown) = e.run(&["--db", other.to_str().unwrap(), "--where-db"]);
    assert!(
        shown.trim().ends_with("other.db"),
        "--db was ignored: {shown}"
    );

    assert!(
        e.where_db().ends_with("main.db"),
        "--db overwrote the setting"
    );

    let out = Command::new(EXE)
        .args(["--where-db"])
        .env("NEKO_WALLET_CONFIG", &e.config)
        .env("NEKO_WALLET_DB", other.to_str().unwrap())
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout)
        .trim()
        .ends_with("other.db"));
    assert!(
        e.where_db().ends_with("main.db"),
        "the variable overwrote the setting"
    );
}

/// No email, no password, no unlock. The command only records a path, and must
/// never touch the vault it points at.
#[test]
fn setting_a_vault_never_reads_or_changes_it() {
    let e = env();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("main.db");
    vault_at(&db);
    let before = std::fs::read(&db).unwrap();

    assert!(e.run(&["set", "db", db.to_str().unwrap()]).0);

    assert_eq!(
        std::fs::read(&db).unwrap(),
        before,
        "the vault was modified"
    );
    let strays: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|r| r.ok())
        .map(|r| r.file_name().to_string_lossy().to_string())
        .filter(|n| n != "main.db")
        .collect();
    assert!(strays.is_empty(), "left behind {strays:?}");
}
