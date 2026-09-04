//! The single-row `vault` table: the authenticated record of how this database
//! was keyed.

use neko_crypto::Argon2idParams;
use rusqlite::{Connection, OptionalExtension};

use crate::error::StoreError;

pub const CURRENT_SCHEMA: i32 = 3;
pub const BLOB_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct VaultRow {
    pub blob_version: u32,
    pub kdf_profile: u8,
    pub params: Argon2idParams,
    pub key_ver: u32,
    pub vault_salt: Vec<u8>,
    pub email_norm: String,
    pub wrapped_mk: Vec<u8>,
    pub wrapped_mk_prev: Option<Vec<u8>>,
    pub rewrap_state: i64,
    pub verifier: Vec<u8>,
    pub wallet_seq: i64,
}

/// Create the schema, then bring it to the current version.
///
/// `0001` is the original schema and stays as it was; everything since is a
/// migration. Running them here as well means a new database and an upgraded
/// one end up byte-identical in structure, rather than the new one skipping
/// steps and diverging in ways only an upgraded install would ever hit.
pub fn init_schema(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(include_str!("../migrations/0001_init.sql"))?;
    crate::migrate::run(conn)?;
    Ok(())
}

pub fn schema_version(conn: &Connection) -> Result<i32, StoreError> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

pub fn insert(conn: &Connection, v: &VaultRow, created_at: i64) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO vault (id, blob_version, kdf_profile, kdf_mem_kib, kdf_iters, kdf_par,
                            kdf_key_len, key_ver, vault_salt, email_norm, wrapped_mk,
                            wrapped_mk_prev, rewrap_state, verifier, wallet_seq, created_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, 0, ?11, 0, ?12)",
        rusqlite::params![
            v.blob_version,
            v.kdf_profile,
            v.params.mem_kib,
            v.params.iters,
            v.params.par,
            v.params.key_len,
            v.key_ver,
            v.vault_salt,
            v.email_norm,
            v.wrapped_mk,
            v.verifier,
            created_at,
        ],
    )?;
    Ok(())
}

pub fn load(conn: &Connection) -> Result<Option<VaultRow>, StoreError> {
    let row = conn
        .query_row(
            "SELECT blob_version, kdf_profile, kdf_mem_kib, kdf_iters, kdf_par, kdf_key_len,
                    key_ver, vault_salt, email_norm, wrapped_mk, wrapped_mk_prev,
                    rewrap_state, verifier, wallet_seq
             FROM vault WHERE id = 1",
            [],
            |r| {
                Ok(VaultRow {
                    blob_version: r.get(0)?,
                    kdf_profile: r.get(1)?,
                    params: Argon2idParams {
                        mem_kib: r.get(2)?,
                        iters: r.get(3)?,
                        par: r.get(4)?,
                        key_len: r.get(5)?,
                    },
                    key_ver: r.get(6)?,
                    vault_salt: r.get(7)?,
                    email_norm: r.get(8)?,
                    wrapped_mk: r.get(9)?,
                    wrapped_mk_prev: r.get(10)?,
                    rewrap_state: r.get(11)?,
                    verifier: r.get(12)?,
                    wallet_seq: r.get(13)?,
                })
            },
        )
        .optional()?;
    Ok(row)
}

/// Password change, step 1: stage the new wrap while keeping the old one, so an
/// interrupted rekey is recoverable rather than fatal.
pub fn stage_rewrap(conn: &Connection, new_wrapped: &[u8], now: i64) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE vault SET wrapped_mk_prev = wrapped_mk, wrapped_mk = ?1,
                          rewrap_state = 1, rewrapped_at = ?2 WHERE id = 1",
        rusqlite::params![new_wrapped, now],
    )?;
    Ok(())
}

/// Password change, step 3: the rekey succeeded, drop the old wrap.
pub fn finish_rewrap(conn: &Connection) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE vault SET wrapped_mk_prev = NULL, rewrap_state = 0 WHERE id = 1",
        [],
    )?;
    Ok(())
}
