//! Derived addresses.
//!
//! Each row stores the address twice: base58 (what the user sees and copies)
//! and the 21 raw bytes (what matching and lookups use). Those two drifting
//! apart is the worst failure mode in a wallet — the user is shown one address
//! while the software watches another, so deposits vanish with no error
//! anywhere. [`verify_consistency`] refuses to proceed if they disagree.

use rusqlite::{Connection, OptionalExtension};

use crate::error::StoreError;

pub const TRON_CHAIN_ID: i64 = 1;
pub const BSC_CHAIN_ID: i64 = 2;

/// Address widths this store accepts, by chain. TRON carries a `0x41` prefix
/// and is 21 bytes; EVM chains are 20.
fn expected_len(chain_id: i64) -> Option<usize> {
    match chain_id {
        TRON_CHAIN_ID => Some(21),
        BSC_CHAIN_ID => Some(20),
        _ => None,
    }
}

#[derive(Debug, Clone)]
pub struct AddressRow {
    pub id: i64,
    pub wallet_id: i64,
    /// Which chain this address belongs to. Without it a caller cannot tell a
    /// 20-byte EVM address from a truncated TRON one.
    pub chain_id: i64,
    pub address: String,
    pub address_raw: Vec<u8>,
    pub deriv_index: i64,
}

/// Insert the account + address pair for a wallet, if not already present.
/// Returns the address row id.
pub fn ensure(
    conn: &Connection,
    wallet_id: i64,
    chain_id: i64,
    deriv_index: i64,
    address: &str,
    address_raw: &[u8],
) -> Result<i64, StoreError> {
    // The width is chain-specific, and checking it here means a mis-encoded
    // address is refused before it reaches the column that incoming transfers
    // are matched on.
    match expected_len(chain_id) {
        Some(n) if address_raw.len() == n => {}
        _ => return Err(StoreError::AddressDrift(wallet_id)),
    }

    conn.execute(
        "INSERT OR IGNORE INTO accounts (wallet_id, chain_id, account_index)
         VALUES (?1, ?2, 0)",
        rusqlite::params![wallet_id, chain_id],
    )?;
    let account_id: i64 = conn.query_row(
        "SELECT id FROM accounts WHERE wallet_id = ?1 AND chain_id = ?2 AND account_index = 0",
        rusqlite::params![wallet_id, chain_id],
        |r| r.get(0),
    )?;

    conn.execute(
        "INSERT OR IGNORE INTO addresses (account_id, deriv_index, change, address, address_raw)
         VALUES (?1, ?2, 0, ?3, ?4)",
        rusqlite::params![account_id, deriv_index, address, address_raw],
    )?;
    // Look the row up rather than trusting last_insert_rowid: the INSERT is a
    // no-op when this account already has the address.
    conn.query_row(
        "SELECT id FROM addresses WHERE account_id = ?1 AND change = 0 AND deriv_index = ?2",
        rusqlite::params![account_id, deriv_index],
        |r| r.get(0),
    )
    .map_err(StoreError::from)
}

pub fn for_wallet(conn: &Connection, wallet_id: i64) -> Result<Vec<AddressRow>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT a.id, ac.wallet_id, ac.chain_id, a.address, a.address_raw, a.deriv_index
         FROM addresses a
         JOIN accounts ac ON ac.id = a.account_id
         WHERE ac.wallet_id = ?1
         ORDER BY ac.chain_id, a.deriv_index",
    )?;
    let rows = stmt.query_map([wallet_id], |r| {
        Ok(AddressRow {
            id: r.get(0)?,
            wallet_id: r.get(1)?,
            chain_id: r.get(2)?,
            address: r.get(3)?,
            address_raw: r.get(4)?,
            deriv_index: r.get(5)?,
        })
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

pub fn find(conn: &Connection, address: &str) -> Result<Option<i64>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT id FROM addresses WHERE address = ?1",
            [address],
            |r| r.get(0),
        )
        .optional()?)
}

/// Re-encode every stored raw address and compare it to the stored base58.
///
/// A mistyped or corrupted hex is usually still a *valid* address, so format
/// checks cannot catch this. Run it at startup and refuse to continue on a
/// mismatch: not running is better than running while showing the user an
/// address the software is not watching.
/// Re-encode every stored raw address and compare it to the stored text.
///
/// `encode` is given the chain as well as the bytes: a 20-byte EVM address
/// handed to a TRON decoder does not merely fail, it fails in a way that looks
/// exactly like the corruption this check exists to find, and the wallet would
/// refuse to start over an address that is perfectly fine.
pub fn verify_consistency<F>(conn: &Connection, encode: F) -> Result<usize, StoreError>
where
    F: Fn(i64, &[u8]) -> Option<String>,
{
    let mut stmt = conn.prepare(
        "SELECT a.id, a.address, a.address_raw, ac.chain_id
         FROM addresses a JOIN accounts ac ON ac.id = a.account_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Vec<u8>>(2)?,
            r.get::<_, i64>(3)?,
        ))
    })?;
    let mut checked = 0usize;
    for row in rows {
        let (id, address, raw, chain_id) = row?;
        match encode(chain_id, &raw) {
            Some(rebuilt) if rebuilt == address => checked += 1,
            _ => return Err(StoreError::AddressDrift(id)),
        }
    }
    Ok(checked)
}
