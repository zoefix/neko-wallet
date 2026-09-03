//! Wallet records.
//!
//! A wallet is one BIP39 mnemonic (or one imported private key). Mnemonics are
//! never derived from the master key: this build is strictly zero-recovery, so
//! each wallet's entropy is independently random and the only backup is the
//! user writing the words down.

use neko_vault::keys::DataKey;
use rusqlite::Connection;
use zeroize::Zeroizing;

use crate::codec;
use crate::error::StoreError;

pub const TABLE: &str = "wallets";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Generated,
    ImportedMnemonic,
    ImportedPrivkey,
}

impl Origin {
    pub fn as_str(self) -> &'static str {
        match self {
            Origin::Generated => "generated",
            Origin::ImportedMnemonic => "imported_mnemonic",
            Origin::ImportedPrivkey => "imported_privkey",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "generated" => Some(Origin::Generated),
            "imported_mnemonic" => Some(Origin::ImportedMnemonic),
            "imported_privkey" => Some(Origin::ImportedPrivkey),
            _ => None,
        }
    }
    /// Only mnemonic-backed wallets have words to show.
    pub fn has_mnemonic(self) -> bool {
        matches!(self, Origin::Generated | Origin::ImportedMnemonic)
    }
}

/// Metadata only. Secret material is fetched separately and deliberately: no
/// listing operation should ever pull a mnemonic into memory.
#[derive(Debug, Clone)]
pub struct WalletMeta {
    pub id: i64,
    pub seq: i64,
    pub origin: Origin,
    pub label: String,
    pub wordlist_lang: String,
    pub created_at: i64,
}

/// What to store for a new wallet.
pub struct NewWallet<'a> {
    pub origin: Origin,
    pub label: &'a str,
    pub wordlist_lang: &'a str,
    /// BIP39 entropy; `None` only for a private-key import.
    pub entropy: Option<&'a [u8]>,
    /// The optional 25th word. Without it the seed cannot be rebuilt from the
    /// entropy, so it must be stored whenever it exists.
    pub bip39_passphrase: Option<&'a str>,
    pub privkey: Option<&'a [u8]>,
}

pub fn create(
    conn: &mut Connection,
    key: &DataKey,
    w: NewWallet<'_>,
    now: i64,
) -> Result<i64, StoreError> {
    let tx = conn.transaction()?;
    let seq: i64 = tx.query_row("SELECT wallet_seq FROM vault WHERE id = 1", [], |r| {
        r.get(0)
    })?;

    // The AAD binds the rowid, so we must know it *before* sealing. Reserving
    // the id up front lets everything go in one INSERT, which matters because
    // the schema's CHECK constraints describe the finished row: a two-step
    // insert would momentarily violate them with NULL ciphertext columns.
    // A lost race here fails on the primary key rather than corrupting anything.
    let id: i64 = tx.query_row("SELECT COALESCE(MAX(id), 0) + 1 FROM wallets", [], |r| {
        r.get(0)
    })?;

    let label_ct = codec::seal_str(key, TABLE, "label_ct", id, w.label)?;
    let entropy_ct = w
        .entropy
        .map(|e| codec::seal(key, TABLE, "entropy_ct", id, e))
        .transpose()?;
    let privkey_ct = w
        .privkey
        .map(|p| codec::seal(key, TABLE, "privkey_ct", id, p))
        .transpose()?;
    let pass_ct = w
        .bip39_passphrase
        .map(|p| codec::seal_str(key, TABLE, "bip39_pass_ct", id, p))
        .transpose()?;

    tx.execute(
        "INSERT INTO wallets
           (id, seq, origin, wordlist_lang, label_ct, entropy_ct, privkey_ct,
            bip39_pass_ct, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            id,
            seq,
            w.origin.as_str(),
            w.wordlist_lang,
            label_ct,
            entropy_ct,
            privkey_ct,
            pass_ct,
            now,
        ],
    )?;

    tx.execute(
        "UPDATE vault SET wallet_seq = wallet_seq + 1 WHERE id = 1",
        [],
    )?;
    tx.commit()?;
    Ok(id)
}

pub fn list(conn: &Connection, key: &DataKey) -> Result<Vec<WalletMeta>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT id, seq, origin, wordlist_lang, created_at, label_ct
         FROM wallets WHERE status = 'active' ORDER BY seq",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, i64>(4)?,
            r.get::<_, Option<Vec<u8>>>(5)?,
        ))
    })?;

    let mut out = Vec::new();
    for row in rows {
        let (id, seq, origin, lang, created_at, label_ct) = row?;
        let label = match label_ct {
            Some(ct) => codec::open_str(key, TABLE, "label_ct", id, &ct)?.to_string(),
            None => String::new(),
        };
        out.push(WalletMeta {
            id,
            seq,
            origin: Origin::parse(&origin).unwrap_or(Origin::Generated),
            label,
            wordlist_lang: lang,
            created_at,
        });
    }
    Ok(out)
}

pub fn rename(conn: &Connection, key: &DataKey, id: i64, label: &str) -> Result<(), StoreError> {
    let ct = codec::seal_str(key, TABLE, "label_ct", id, label)?;
    let n = conn.execute(
        "UPDATE wallets SET label_ct = ?1 WHERE id = ?2",
        rusqlite::params![ct, id],
    )?;
    if n == 0 {
        return Err(StoreError::NoSuchWallet(id));
    }
    Ok(())
}

/// Hard delete. The words are gone with it, which is why the UI must confirm
/// against something only a deliberate user would produce.
pub fn delete(conn: &Connection, id: i64) -> Result<(), StoreError> {
    let n = conn.execute("DELETE FROM wallets WHERE id = ?1", rusqlite::params![id])?;
    if n == 0 {
        return Err(StoreError::NoSuchWallet(id));
    }
    Ok(())
}

pub fn get(conn: &Connection, key: &DataKey, id: i64) -> Result<WalletMeta, StoreError> {
    list(conn, key)?
        .into_iter()
        .find(|w| w.id == id)
        .ok_or(StoreError::NoSuchWallet(id))
}

/// Fetch a wallet's BIP39 entropy. Separate from every listing call on purpose:
/// secret material is only ever loaded when something explicitly asks for it.
pub fn entropy(
    conn: &Connection,
    key: &DataKey,
    id: i64,
) -> Result<Option<Zeroizing<Vec<u8>>>, StoreError> {
    codec::read_sealed_opt(conn, key, TABLE, "entropy_ct", id)
}

pub fn bip39_passphrase(
    conn: &Connection,
    key: &DataKey,
    id: i64,
) -> Result<Option<Zeroizing<String>>, StoreError> {
    Ok(
        codec::read_sealed_opt(conn, key, TABLE, "bip39_pass_ct", id)?
            .map(|b| Zeroizing::new(String::from_utf8_lossy(&b).into_owned())),
    )
}

pub fn privkey(
    conn: &Connection,
    key: &DataKey,
    id: i64,
) -> Result<Option<Zeroizing<Vec<u8>>>, StoreError> {
    codec::read_sealed_opt(conn, key, TABLE, "privkey_ct", id)
}
