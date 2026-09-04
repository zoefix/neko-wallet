//! Wallet operations.
//!
//! Two rules shape this module:
//!
//! 1. **Creating a wallet never returns the mnemonic.** The user views it later,
//!    deliberately, through [`Session::reveal_mnemonic`], which re-authenticates.
//! 2. **Secret material is loaded only when explicitly requested.** Listing
//!    wallets does not touch entropy; deriving an address borrows the key for
//!    the duration of the call and drops it.

use neko_hd::derive;
use neko_store::repo::wallets::{self, NewWallet, Origin, WalletMeta};
use zeroize::Zeroizing;

use crate::chain::{ChainAddress, ChainId};
use crate::error::CoreError;
use crate::session::Session;

/// Metadata plus the wallet's first TRON address. Never any secret.
#[derive(Debug, Clone)]
pub struct WalletView {
    pub id: i64,
    pub label: String,
    pub origin: Origin,
    /// One address per chain. A wallet is not a single account any more: the
    /// coin type in the derivation path differs per chain, so the same phrase
    /// yields a different address on each.
    pub addresses: Vec<(ChainId, String)>,
    pub created_at: i64,
    /// Last known balances per chain, straight from the local cache. Rendered
    /// immediately; refreshed in the background.
    ///
    /// Kept per chain rather than merged. "USDT" exists on both, with six
    /// decimals on one and eighteen on the other, and the two are not
    /// interchangeable without a bridge - a combined figure would be a number
    /// nobody can spend.
    pub assets: Vec<(ChainId, CachedAssets)>,
}

impl WalletView {
    /// Cached balances on one chain, empty if nothing has been fetched yet.
    pub fn assets_on(&self, chain: ChainId) -> CachedAssets {
        self.assets
            .iter()
            .find(|(c, _)| *c == chain)
            .map(|(_, a)| a.clone())
            .unwrap_or_default()
    }

    /// This wallet's address on one chain.
    pub fn address(&self, chain: ChainId) -> &str {
        self.addresses
            .iter()
            .find(|(c, _)| *c == chain)
            .map(|(_, a)| a.as_str())
            .unwrap_or("")
    }
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
        // Give the wallet an address row on every chain immediately, so
        // balances and history have somewhere to attach.
        self.register_all_chains(id)?;
        Ok(id)
    }

    pub fn list_wallets(&self) -> Result<Vec<WalletView>, CoreError> {
        let metas = wallets::list(self.conn()?, self.data_key())?;
        metas.into_iter().map(|m| self.view(m)).collect()
    }

    fn view(&self, m: WalletMeta) -> Result<WalletView, CoreError> {
        let mut addresses = Vec::new();
        for c in crate::chain::CHAINS {
            addresses.push((c, self.address_of(m.id, c, 0)?.to_string()));
        }
        // Straight from the local cache: the list must render without waiting
        // on the network. An empty cache simply shows no figure yet.
        let assets = crate::chain::CHAINS
            .into_iter()
            .map(|c| (c, self.cached_assets(m.id, c).unwrap_or_default()))
            .collect();
        Ok(WalletView {
            id: m.id,
            label: m.label,
            origin: m.origin,
            addresses,
            created_at: m.created_at,
            assets,
        })
    }

    /// Derive an address on one chain. The private key is borrowed for the
    /// call and dropped.
    ///
    /// A wallet imported from a raw private key has one key and therefore one
    /// account per chain - the *same* twenty bytes, printed two ways. A wallet
    /// with a phrase derives a different key per chain, because the coin type
    /// in the path differs. Both are correct and standard; the difference
    /// surprises people, which is why it is stated here.
    pub fn address_of(
        &self,
        wallet_id: i64,
        chain: ChainId,
        index: u32,
    ) -> Result<ChainAddress, CoreError> {
        let conn = self.conn()?;
        let key = self.data_key();

        if let Some(pk) = wallets::privkey(conn, key, wallet_id)? {
            let mut sk = [0u8; 32];
            if pk.len() != 32 {
                return Err(CoreError::BadPrivateKey);
            }
            sk.copy_from_slice(&pk);
            return Ok(match chain {
                ChainId::Tron => ChainAddress::Tron(derive::address_from_private_key(&sk)?),
                ChainId::Bsc => ChainAddress::Evm(derive::evm_address_from_private_key(&sk)?),
                ChainId::Solana => {
                    ChainAddress::Solana(neko_hd::solana::address_from_private_key(&sk)?)
                }
                ChainId::Bitcoin => {
                    ChainAddress::Bitcoin(neko_hd::bitcoin::address_from_private_key(&sk)?)
                }
            });
        }

        let seed = self.seed_for(wallet_id)?;
        Ok(match chain {
            ChainId::Tron => ChainAddress::Tron(derive::address_at(&seed, 0, index)?),
            ChainId::Bsc => ChainAddress::Evm(derive::evm_address_at(&seed, 0, index)?),
            // SLIP-0010, hardened at every level, so the account level is what
            // varies rather than a change/index pair that cannot exist here.
            ChainId::Solana => ChainAddress::Solana(neko_hd::solana::address_at(&seed, index)?),
            // `m/84'/0'/0'/0/{index}` - the receiving branch. Change comes back
            // to the same address, which the one-address-per-chain model
            // already implies.
            ChainId::Bitcoin => {
                ChainAddress::Bitcoin(neko_hd::bitcoin::address_at(&seed, 0, 0, index)?)
            }
        })
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
    pub fn register_address(&self, wallet_id: i64, chain: ChainId) -> Result<i64, CoreError> {
        let addr = self.address_of(wallet_id, chain, 0)?;
        Ok(neko_store::repo::addresses::ensure(
            self.conn()?,
            wallet_id,
            db_chain_id(chain),
            0,
            &addr.to_string(),
            &addr.as_bytes(),
        )?)
    }

    pub fn register_all_chains(&self, wallet_id: i64) -> Result<(), CoreError> {
        for c in crate::chain::CHAINS {
            self.register_address(wallet_id, c)?;
        }
        Ok(())
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
            for c in crate::chain::CHAINS {
                // Every wallet that predates a chain needs its address on that
                // chain. This is what gives an existing wallet its BNB Chain
                // account on the first unlock after upgrading, with no
                // separate step and no new phrase.
                if !existing.iter().any(|r| r.chain_id == db_chain_id(c)) {
                    self.register_address(w.id, c)?;
                    added += 1;
                }
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
            |chain_id, raw| {
                // Decoded with the chain's own encoder. Using TRON's on a
                // 20-byte EVM address would fail in exactly the way real
                // corruption does, and the wallet would refuse to start over
                // an address that is perfectly correct.
                let chain = from_db_chain_id(chain_id)?;
                ChainAddress::from_bytes(chain, raw)
                    .ok()
                    .map(|a| a.to_string())
            },
        )?)
    }

    pub fn cached_assets(&self, wallet_id: i64, chain: ChainId) -> Result<CachedAssets, CoreError> {
        let rows = neko_store::repo::balances::for_wallet_on_chain(
            self.conn()?,
            wallet_id,
            db_chain_id(chain),
        )?;
        let updated_at = rows.iter().map(|r| r.updated_at).max();
        Ok(CachedAssets { rows, updated_at })
    }

    /// Store freshly fetched balances. `assets` is (symbol, decimals, amount).
    pub fn cache_assets(
        &self,
        wallet_id: i64,
        chain: ChainId,
        assets: &[(String, u8, i128)],
    ) -> Result<(), CoreError> {
        use neko_store::repo::{addresses, balances};
        let conn = self.conn()?;
        let db_chain = db_chain_id(chain);
        let address_id = match addresses::for_wallet(conn, wallet_id)?
            .into_iter()
            .find(|r| r.chain_id == db_chain)
        {
            Some(a) => a.id,
            None => self.register_address(wallet_id, chain)?,
        };
        let now = now();
        for (symbol, decimals, amount) in assets {
            let contract = (symbol == "USDT").then(|| match chain {
                ChainId::Tron => neko_tron::usdt_address().as_bytes().to_vec(),
                ChainId::Bsc => neko_evm::usdt_address().as_bytes().to_vec(),
                ChainId::Solana => neko_solana::usdt_mint().as_bytes().to_vec(),
                // Unreachable: Bitcoin has no USDT, so this closure is never
                // reached for it.
                ChainId::Bitcoin => Vec::new(),
            });
            let asset = balances::asset_id(conn, db_chain, symbol, contract.as_deref(), *decimals)?;
            balances::upsert(conn, address_id, asset, *amount, 0, now)?;
        }
        Ok(())
    }
}

/// The chain ids the database uses. Kept next to the code that reads and
/// writes them rather than exported, so nothing outside this module has to
/// know that `tron` is 1.
fn db_chain_id(c: ChainId) -> i64 {
    match c {
        ChainId::Tron => neko_store::repo::addresses::TRON_CHAIN_ID,
        ChainId::Bsc => neko_store::repo::addresses::BSC_CHAIN_ID,
        ChainId::Solana => neko_store::repo::addresses::SOLANA_CHAIN_ID,
        ChainId::Bitcoin => neko_store::repo::addresses::BITCOIN_CHAIN_ID,
    }
}

fn from_db_chain_id(id: i64) -> Option<ChainId> {
    crate::chain::CHAINS
        .into_iter()
        .find(|c| db_chain_id(*c) == id)
}
