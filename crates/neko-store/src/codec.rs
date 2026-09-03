//! Field-level envelope encryption for the `_ct` columns.
//!
//! Layer 2 on top of SQLCipher. It defends the window where the database is
//! already open: a process memory dump, a careless export, or a future
//! SQLCipher vulnerability.
//!
//! Every ciphertext is AAD-bound to `table/column/rowid/key_ver`, so an
//! attacker with write access cannot copy one row's ciphertext over another's —
//! e.g. overwrite the cold wallet's encrypted key with one they control.

use neko_crypto::Aad;
use neko_vault::keys::DataKey;
use rusqlite::{Connection, Transaction};
use zeroize::Zeroizing;

use crate::error::StoreError;

pub const KEY_VER: u32 = 1;

pub fn seal(
    key: &DataKey,
    table: &str,
    column: &str,
    row_id: i64,
    plaintext: &[u8],
) -> Result<Vec<u8>, StoreError> {
    let aad = Aad::new(table, column, row_id, KEY_VER);
    Ok(neko_crypto::seal(key.as_bytes(), plaintext, aad)?)
}

pub fn open(
    key: &DataKey,
    table: &str,
    column: &str,
    row_id: i64,
    sealed: &[u8],
) -> Result<Zeroizing<Vec<u8>>, StoreError> {
    let aad = Aad::new(table, column, row_id, KEY_VER);
    Ok(neko_crypto::open(key.as_bytes(), sealed, aad)?)
}

pub fn seal_str(
    key: &DataKey,
    table: &str,
    column: &str,
    row_id: i64,
    s: &str,
) -> Result<Vec<u8>, StoreError> {
    seal(key, table, column, row_id, s.as_bytes())
}

pub fn open_str(
    key: &DataKey,
    table: &str,
    column: &str,
    row_id: i64,
    sealed: &[u8],
) -> Result<Zeroizing<String>, StoreError> {
    let bytes = open(key, table, column, row_id, sealed)?;
    Ok(Zeroizing::new(String::from_utf8_lossy(&bytes).into_owned()))
}

/// Write sealed columns after the row exists.
///
/// The AAD binds the rowid, which we do not know until the INSERT has run. So
/// every sealed column is written in a second statement inside the same
/// transaction. This helper exists so nobody has to remember that.
pub fn write_sealed(
    tx: &Transaction<'_>,
    key: &DataKey,
    table: &str,
    row_id: i64,
    columns: &[(&str, &[u8])],
) -> Result<(), StoreError> {
    for (col, plaintext) in columns {
        let ct = seal(key, table, col, row_id, plaintext)?;
        tx.execute(
            &format!("UPDATE {table} SET {col} = ?1 WHERE id = ?2"),
            rusqlite::params![ct, row_id],
        )?;
    }
    Ok(())
}

/// Read an optional sealed column.
pub fn read_sealed_opt(
    conn: &Connection,
    key: &DataKey,
    table: &str,
    column: &str,
    row_id: i64,
) -> Result<Option<Zeroizing<Vec<u8>>>, StoreError> {
    let ct: Option<Vec<u8>> = conn.query_row(
        &format!("SELECT {column} FROM {table} WHERE id = ?1"),
        rusqlite::params![row_id],
        |r| r.get(0),
    )?;
    match ct {
        None => Ok(None),
        Some(ct) => Ok(Some(open(key, table, column, row_id, &ct)?)),
    }
}
