//! Persisted settings.
//!
//! Non-secret values live in `value`; credentials go through the field-level
//! AEAD into `value_ct`. The schema's CHECK constraint keeps the two in step.

use neko_vault::keys::DataKey;
use rusqlite::{Connection, OptionalExtension};
use zeroize::Zeroizing;

use crate::codec;
use crate::error::StoreError;

pub const TABLE: &str = "settings";

pub mod keys {
    pub const NETWORK: &str = "network";
    pub const NODE_URL: &str = "node_url";
    pub const API_KEY: &str = "trongrid_api_key";
    pub const AUTOLOCK_SECS: &str = "autolock_secs";
    pub const LANGUAGE: &str = "language";
}

pub fn get(conn: &Connection, key: &str) -> Result<Option<String>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT value FROM settings WHERE key = ?1 AND secret = 0",
            [key],
            |r| r.get(0),
        )
        .optional()?
        .flatten())
}

pub fn set(conn: &Connection, key: &str, value: &str) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO settings (key, value, value_ct, secret) VALUES (?1, ?2, NULL, 0)
         ON CONFLICT(key) DO UPDATE SET value = ?2, value_ct = NULL, secret = 0",
        rusqlite::params![key, value],
    )?;
    Ok(())
}

/// Secrets are AAD-bound to their own row, like every other `_ct` column.
///
/// `settings` has a TEXT primary key rather than a rowid we can bind, so the
/// key name itself is folded into the AAD via the column position — the row is
/// addressed by `key`, and the seal uses a stable synthetic row id derived from
/// it, so a ciphertext still cannot be moved to a different setting.
fn secret_row_id(key: &str) -> i64 {
    // FNV-1a, purely to give each setting a stable distinct id for the AAD.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in key.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    (h & 0x7fff_ffff_ffff_ffff) as i64
}

pub fn get_secret(
    conn: &Connection,
    dk: &DataKey,
    key: &str,
) -> Result<Option<Zeroizing<String>>, StoreError> {
    let ct: Option<Vec<u8>> = conn
        .query_row(
            "SELECT value_ct FROM settings WHERE key = ?1 AND secret = 1",
            [key],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    match ct {
        None => Ok(None),
        Some(ct) => Ok(Some(codec::open_str(
            dk,
            TABLE,
            "value_ct",
            secret_row_id(key),
            &ct,
        )?)),
    }
}

pub fn set_secret(
    conn: &Connection,
    dk: &DataKey,
    key: &str,
    value: &str,
) -> Result<(), StoreError> {
    if value.is_empty() {
        conn.execute("DELETE FROM settings WHERE key = ?1", [key])?;
        return Ok(());
    }
    let ct = codec::seal_str(dk, TABLE, "value_ct", secret_row_id(key), value)?;
    conn.execute(
        "INSERT INTO settings (key, value, value_ct, secret) VALUES (?1, NULL, ?2, 1)
         ON CONFLICT(key) DO UPDATE SET value = NULL, value_ct = ?2, secret = 1",
        rusqlite::params![key, ct],
    )?;
    Ok(())
}
