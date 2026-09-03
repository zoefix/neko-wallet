//! Opening and creating the SQLCipher database.
//!
//! Every behaviour relied on here is pinned by `tests/sqlcipher_contract.rs`
//! against real SQLCipher 4.14.0.

use std::fs;
use std::io::Read;
use std::path::Path;

use neko_vault::{FileHeader, FileKey, HEADER_LEN};
use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::error::StoreError;

/// Raw-key-plus-salt keyspec, 96 hex chars. Sets key AND salt before any file
/// I/O, which is how we get *our* 16 bytes into the header at creation time.
fn keyspec_key_salt(k: &FileKey, h: &FileHeader) -> Zeroizing<String> {
    Zeroizing::new(format!(
        "PRAGMA key = \"x'{}{}'\";",
        hex::encode(k.as_bytes()),
        h.to_hex()
    ))
}

/// Read the 16-byte plaintext header without any key. This is what makes the
/// `.db` self-contained: the Argon2 salt travels inside the file.
pub fn read_header(path: &Path) -> Result<FileHeader, StoreError> {
    if !path.exists() {
        return Err(StoreError::NotFound(path.to_path_buf()));
    }
    let mut f = fs::File::open(path)?;
    let mut buf = [0u8; HEADER_LEN];
    f.read_exact(&mut buf).map_err(|_| StoreError::TooSmall)?;
    Ok(FileHeader::parse(&buf)?)
}

/// Applied to every connection, keyed or not.
///
/// `journal_mode = DELETE` is load-bearing, not a preference: WAL would put
/// recent commits in a separate `-wal` file, so copying just the `.db` — the
/// user's whole backup story — would silently lose data.
fn apply_pragmas(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "PRAGMA journal_mode = DELETE;
         PRAGMA synchronous = FULL;
         PRAGMA foreign_keys = ON;
         PRAGMA busy_timeout = 5000;
         PRAGMA cipher_memory_security = ON;",
    )?;
    Ok(())
}

/// Force SQLCipher to derive the key and verify page 1's HMAC. Until a real
/// read happens, a wrong key produces no error.
fn probe(conn: &Connection) -> Result<(), StoreError> {
    conn.query_row("SELECT count(*) FROM sqlite_schema", [], |r| {
        r.get::<_, i64>(0)
    })
    .map(|_| ())
    .map_err(|_| StoreError::KeyRejected)
}

/// Open an existing vault. `header` must come from [`read_header`].
pub fn open(path: &Path, key: &FileKey, header: &FileHeader) -> Result<Connection, StoreError> {
    let conn = Connection::open(path)?;
    // 96-hex form: our independently-read salt is cross-checked by SQLCipher.
    conn.execute_batch(&keyspec_key_salt(key, header))?;
    apply_pragmas(&conn)?;
    probe(&conn)?;
    Ok(conn)
}

/// Create a new vault, writing `header` into bytes 0..16 of the file.
pub fn create(path: &Path, key: &FileKey, header: &FileHeader) -> Result<Connection, StoreError> {
    if path.exists() {
        return Err(StoreError::AlreadyExists(path.to_path_buf()));
    }
    if let Some(dir) = path.parent() {
        if !dir.as_os_str().is_empty() {
            fs::create_dir_all(dir)?;
        }
    }
    // Pre-create with 0600 so SQLite never gets the chance to make it 0644.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
    }

    let conn = Connection::open(path)?;
    conn.execute_batch(&keyspec_key_salt(key, header))?;
    apply_pragmas(&conn)?;
    Ok(conn)
}

/// Re-key the whole-file layer during a password change.
///
/// MUST use the 64-hex form. The 96-hex form overwrites the shared salt
/// mid-rekey, which would orphan the database.
pub fn rekey(conn: &Connection, new_key: &FileKey) -> Result<(), StoreError> {
    conn.execute_batch(&Zeroizing::new(format!(
        "PRAGMA rekey = \"x'{}'\";",
        hex::encode(new_key.as_bytes())
    )))?;
    Ok(())
}

/// Guard against the WAL sidecars that would break file portability.
pub fn assert_no_wal_sidecars(path: &Path) -> Result<(), StoreError> {
    let name = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    for ext in ["-wal", "-shm"] {
        if path.with_file_name(format!("{name}{ext}")).exists() {
            return Err(StoreError::StaleWal);
        }
    }
    Ok(())
}
