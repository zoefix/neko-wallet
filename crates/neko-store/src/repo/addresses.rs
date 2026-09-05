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
pub const SOLANA_CHAIN_ID: i64 = 3;
pub const BITCOIN_CHAIN_ID: i64 = 4;
pub const ETHEREUM_CHAIN_ID: i64 = 5;
pub const TON_CHAIN_ID: i64 = 6;
pub const POLYGON_CHAIN_ID: i64 = 7;

/// Lengths Bitcoin's script column may take: 22 for P2WPKH, 23 for P2SH, 25
/// for P2PKH, 34 for P2WSH and Taproot. Bitcoin is the only chain here whose
/// address is not one fixed size, because what is stored is the locking script
/// and the script *is* the address type.
pub const BITCOIN_SCRIPT_LENS: [usize; 4] = [22, 23, 25, 34];

/// Whether these bytes are a plausible address on this chain.
///
/// The width is the one cheap guard against a truncated or mis-encoded address
/// reaching the column that incoming payments are matched on. TRON carries a
/// `0x41` prefix and is 21 bytes; EVM chains are 20; a Solana address is a
/// 32-byte Ed25519 public key; Bitcoin is one of four script lengths.
///
/// Public so that the migration tests can ask *this* function which widths a
/// chain accepts rather than repeating the list. A hand-copied list is how the
/// column's test came to assert that 33 bytes was not an address on any chain,
/// which stayed true right up until TON.
pub fn width_is_plausible(chain_id: i64, len: usize) -> bool {
    match chain_id {
        TRON_CHAIN_ID => len == 21,
        BSC_CHAIN_ID | ETHEREUM_CHAIN_ID | POLYGON_CHAIN_ID => len == 20,
        SOLANA_CHAIN_ID => len == 32,
        // A workchain byte and a 256-bit account.
        TON_CHAIN_ID => len == 33,
        BITCOIN_CHAIN_ID => BITCOIN_SCRIPT_LENS.contains(&len),
        _ => false,
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
    if !width_is_plausible(chain_id, address_raw.len()) {
        return Err(StoreError::AddressDrift(wallet_id));
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
