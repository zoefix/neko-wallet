//! Wallet operations.
//!
//! Two rules shape this module:
//!
//! 1. **Creating a wallet never returns the mnemonic.** The user views it later,
//!    deliberately, through [`Session::reveal_mnemonic`], which re-authenticates.
//! 2. **Secret material is loaded only when explicitly requested.** Listing
//!    wallets does not touch entropy; deriving an address borrows the key for
//!    the duration of the call and drops it.

use neko_hd::{derive, Address};
use neko_store::repo::wallets::{self, NewWallet, Origin, WalletMeta};
use zeroize::Zeroizing;

use crate::error::CoreError;
use crate::session::Session;

/// Metadata plus the wallet's first TRON address. Never any secret.
#[derive(Debug, Clone)]
pub struct WalletView {
    pub id: i64,
    pub label: String,
    pub origin: Origin,
    pub address: String,
    pub created_at: i64,
    /// Last known balances, straight from the local cache. Rendered
    /// immediately; refreshed in the background.
    pub assets: CachedAssets,
}

pub enum NewWalletSpec<'a> {
    /// Generate fresh entropy. The words are NOT shown at this point.
    Generate {
        words: usize,
    },
    ImportMnemonic {
        phrase: &'a str,
        passphrase: Option<&'a str>,
    },
    ImportPrivateKey {
        hex: &'a str,
    },
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Session {
    pub fn create_wallet(
        &mut self,
        label: &str,
        spec: NewWalletSpec<'_>,
    ) -> Result<i64, CoreError> {
        let data_key = self.data_key().clone();
        let conn = self.conn_mut()?;

        let id = match spec {
            NewWalletSpec::Generate { words } => {
                let phrase = derive::generate_mnemonic(words)?;
                let entropy = derive::entropy_from_mnemonic(&phrase)?;
                wallets::create(
                    conn,
                    &data_key,
                    NewWallet {
                        origin: Origin::Generated,
                        label,
                        wordlist_lang: "en",
                        entropy: Some(&entropy),
                        bip39_passphrase: None,
                        privkey: None,
                    },
                    now(),
                )?
            }
            NewWalletSpec::ImportMnemonic { phrase, passphrase } => {
                if !derive::validate_mnemonic(phrase) {
                    return Err(CoreError::BadMnemonic);
                }
                let entropy = derive::entropy_from_mnemonic(phrase)?;
                wallets::create(
                    conn,
                    &data_key,
                    NewWallet {
                        origin: Origin::ImportedMnemonic,
                        label,
                        wordlist_lang: "en",
                        entropy: Some(&entropy),
                        // Must be stored: without the 25th word the entropy
                        // alone cannot rebuild the seed.
                        bip39_passphrase: passphrase.filter(|p| !p.is_empty()),
                        privkey: None,
                    },
                    now(),
                )?
            }
            NewWalletSpec::ImportPrivateKey { hex } => {
                let raw = parse_private_key(hex)?;
                wallets::create(
                    conn,
                    &data_key,
                    NewWallet {
                        origin: Origin::ImportedPrivkey,
                        label,
                        wordlist_lang: "en",
                        entropy: None,
                        bip39_passphrase: None,
                        privkey: Some(&raw),
                    },
                    now(),
                )?
            }
        };
        // Give the wallet an address row immediately so balances and history
        // have somewhere to attach.
        self.register_address(id)?;
        Ok(id)
    }

    pub fn list_wallets(&self) -> Result<Vec<WalletView>, CoreError> {
        let metas = wallets::list(self.conn()?, self.data_key())?;
        metas.into_iter().map(|m| self.view(m)).collect()
    }

    fn view(&self, m: WalletMeta) -> Result<WalletView, CoreError> {
        let address = self.address_of(m.id, 0)?.to_string();
        // Straight from the local cache: the list must render without waiting
        // on the network. An empty cache simply shows no figure yet.
        let assets = self.cached_assets(m.id).unwrap_or_default();
        Ok(WalletView {
            id: m.id,
            label: m.label,
            origin: m.origin,
            address,
            created_at: m.created_at,
            assets,
        })
    }

    /// Derive an address. The private key is borrowed for the call and dropped.
    pub fn address_of(&self, wallet_id: i64, index: u32) -> Result<Address, CoreError> {
        let conn = self.conn()?;
        let key = self.data_key();

        if let Some(pk) = wallets::privkey(conn, key, wallet_id)? {
            let mut sk = [0u8; 32];
            if pk.len() != 32 {
                return Err(CoreError::BadPrivateKey);
            }
            sk.copy_from_slice(&pk);
            return Ok(derive::address_from_private_key(&sk)?);
        }

        let seed = self.seed_for(wallet_id)?;
        Ok(derive::address_at(&seed, 0, index)?)
    }

    pub(crate) fn seed_for(&self, wallet_id: i64) -> Result<Zeroizing<[u8; 64]>, CoreError> {
        let conn = self.conn()?;
        let key = self.data_key();
        let entropy = wallets::entropy(conn, key, wallet_id)?.ok_or(CoreError::NoMnemonic)?;
        let phrase = derive::mnemonic_from_entropy(&entropy)?;
        let pass = wallets::bip39_passphrase(conn, key, wallet_id)?;
        Ok(derive::seed_from_mnemonic(
            &phrase,
            pass.as_deref().map(|s| s.as_str()).unwrap_or(""),
        )?)
    }

    pub fn rename_wallet(&self, id: i64, label: &str) -> Result<(), CoreError> {
        Ok(wallets::rename(self.conn()?, self.data_key(), id, label)?)
    }

    pub fn delete_wallet(&self, id: i64) -> Result<(), CoreError> {
        Ok(wallets::delete(self.conn()?, id)?)
    }

    /// Reveal a wallet's recovery phrase.
    ///
    /// Re-runs the **full** Argon2id derivation even though the session is
    /// already unlocked. The threat this addresses is an unlocked terminal that
    /// the owner walked away from, and a check that reuses the in-memory key
    /// would be decorative. The ~0.5s cost is the point: deliberate friction on
    /// the most dangerous action in the application.
    pub fn reveal_mnemonic(
        &self,
        path: &std::path::Path,
        wallet_id: i64,
        password: &str,
    ) -> Result<Zeroizing<String>, CoreError> {
        // Any failure here is indistinguishable from a wrong password.
        let probe = crate::VaultFile::at(path)
            .unlock(self.email(), password)
            .map_err(|_| CoreError::WrongCredentials)?;
        drop(probe);

        let entropy = wallets::entropy(self.conn()?, self.data_key(), wallet_id)?
            .ok_or(CoreError::NoMnemonic)?;
        Ok(derive::mnemonic_from_entropy(&entropy)?)
    }
}

fn parse_private_key(s: &str) -> Result<Zeroizing<Vec<u8>>, CoreError> {
    let clean = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    if clean.len() != 64 || !clean.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(CoreError::BadPrivateKey);
    }
    let mut out = Zeroizing::new(Vec::with_capacity(32));
    for i in (0..64).step_by(2) {
        out.push(u8::from_str_radix(&clean[i..i + 2], 16).map_err(|_| CoreError::BadPrivateKey)?);
    }
    // Reject keys secp256k1 will not accept (zero, or >= curve order).
    let mut arr = [0u8; 32];
    arr.copy_from_slice(&out);
    derive::address_from_private_key(&arr).map_err(|_| CoreError::BadPrivateKey)?;
    Ok(out)
}

// ── Address registration and balance cache ─────────────────────────────────

/// A wallet's cached balances, with the age of the newest reading.
#[derive(Debug, Clone, Default)]
pub struct CachedAssets {
    pub rows: Vec<neko_store::repo::balances::CachedBalance>,
    /// Unix seconds of the freshest entry, or `None` if nothing is cached yet.
    pub updated_at: Option<i64>,
}

impl CachedAssets {
    pub fn amount(&self, symbol: &str) -> Option<(i128, u8)> {
        self.rows
            .iter()
            .find(|r| r.symbol == symbol)
            .map(|r| (r.amount, r.decimals))
    }
}

impl Session {
    /// Record a wallet's address so balances and history have something to hang
    /// off. Idempotent.
    pub fn register_address(&self, wallet_id: i64) -> Result<i64, CoreError> {
        let addr = self.address_of(wallet_id, 0)?;
        Ok(neko_store::repo::addresses::ensure(
            self.conn()?,
            wallet_id,
            neko_store::repo::addresses::TRON_CHAIN_ID,
            0,
            &addr.to_string(),
            addr.as_bytes(),
        )?)
    }

    /// Register any wallet that predates address bookkeeping.
    ///
    /// Wallets created before this existed have no address row, so their
    /// balances could never be cached. Backfilling on unlock keeps that
    /// invisible to the user rather than requiring a migration step.
    pub fn backfill_addresses(&self) -> Result<usize, CoreError> {
        let mut added = 0;
        for w in self.list_wallets()? {
            let existing = neko_store::repo::addresses::for_wallet(self.conn()?, w.id)?;
            if existing.is_empty() {
                self.register_address(w.id)?;
                added += 1;
            }
        }
        Ok(added)
    }

    /// Re-encode every stored raw address and compare it to the stored base58.
    ///
    /// A corrupted hex is usually still a *valid* address, so nothing else
    /// catches this. Refuse to continue rather than show one address while
    /// watching another.
    pub fn verify_address_consistency(&self) -> Result<usize, CoreError> {
        Ok(neko_store::repo::addresses::verify_consistency(
            self.conn()?,
            |raw| {
                neko_hd::Address::from_bytes(raw)
                    .ok()
                    .map(|a| a.to_string())
            },
        )?)
    }

    pub fn cached_assets(&self, wallet_id: i64) -> Result<CachedAssets, CoreError> {
        let rows = neko_store::repo::balances::for_wallet(self.conn()?, wallet_id)?;
        let updated_at = rows.iter().map(|r| r.updated_at).max();
        Ok(CachedAssets { rows, updated_at })
    }

    /// Store freshly fetched balances. `assets` is (symbol, decimals, amount).
    pub fn cache_assets(
        &self,
        wallet_id: i64,
        assets: &[(String, u8, i128)],
    ) -> Result<(), CoreError> {
        use neko_store::repo::{addresses, balances};
        let conn = self.conn()?;
        let address_id = match addresses::for_wallet(conn, wallet_id)?.first() {
            Some(a) => a.id,
            None => self.register_address(wallet_id)?,
        };
        let now = now();
        for (symbol, decimals, amount) in assets {
            let contract = (symbol == "USDT").then(neko_tron::usdt_address);
            let asset = balances::asset_id(
                conn,
                addresses::TRON_CHAIN_ID,
                symbol,
                contract.as_ref().map(|c| c.as_bytes().as_slice()),
                *decimals,
            )?;
            balances::upsert(conn, address_id, asset, *amount, 0, now)?;
        }
        Ok(())
    }
}
