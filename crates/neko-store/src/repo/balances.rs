//! Cached on-chain balances.
//!
//! The cache exists so the wallet list can render instantly instead of staring
//! at spinners while several network round trips complete. Every read carries
//! `updated_at` so the UI can say how old the number is — a stale balance
//! presented as current is a lie, but a stale balance labelled as stale is
//! useful.

use rusqlite::Connection;

use crate::error::StoreError;

#[derive(Debug, Clone)]
pub struct CachedBalance {
    pub symbol: String,
    pub decimals: u8,
    /// Minimal units. Stored as an i128 blob: exact, and sortable in SQL.
    pub amount: i128,
    pub updated_at: i64,
}

/// Look up an asset id by symbol, inserting it if this is the first sighting.
pub fn asset_id(
    conn: &Connection,
    chain_id: i64,
    symbol: &str,
    contract: Option<&[u8]>,
    decimals: u8,
) -> Result<i64, StoreError> {
    conn.execute(
        "INSERT OR IGNORE INTO assets (chain_id, symbol, contract, decimals)
         VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![chain_id, symbol, contract, decimals],
    )?;
    Ok(conn.query_row(
        "SELECT id FROM assets WHERE chain_id = ?1 AND symbol = ?2",
        rusqlite::params![chain_id, symbol],
        |r| r.get(0),
    )?)
}

pub fn upsert(
    conn: &Connection,
    address_id: i64,
    asset_id: i64,
    amount: i128,
    block_num: i64,
    now: i64,
) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO balances (address_id, asset_id, amount, block_num, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(address_id, asset_id) DO UPDATE
           SET amount = ?3, block_num = ?4, updated_at = ?5",
        rusqlite::params![address_id, asset_id, amount, block_num, now],
    )?;
    Ok(())
}

/// Cached balances for every address belonging to a wallet.
pub fn for_wallet(conn: &Connection, wallet_id: i64) -> Result<Vec<CachedBalance>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT s.symbol, s.decimals, b.amount, b.updated_at
         FROM balances b
         JOIN addresses a  ON a.id = b.address_id
         JOIN accounts  ac ON ac.id = a.account_id
         JOIN assets    s  ON s.id = b.asset_id
         WHERE ac.wallet_id = ?1
         ORDER BY s.symbol",
    )?;
    let rows = stmt.query_map([wallet_id], |r| {
        Ok(CachedBalance {
            symbol: r.get(0)?,
            decimals: r.get::<_, i64>(1)? as u8,
            amount: r.get(2)?,
            updated_at: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}
