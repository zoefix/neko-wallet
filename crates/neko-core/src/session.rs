//! Vault lifecycle.
//!
//! Zero recovery by design: the password is the only key. There is no password
//! hash, no verifier stored outside the encryption, no recovery code. "Checking
//! the password" *is* decrypting — it works or it doesn't.

use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use neko_store::vault_row::{self, VaultRow, BLOB_VERSION};
use neko_vault::keys::{self, DataKey, Mk};
use neko_vault::{password, profile::Profile, FileHeader, FileKey};
use rusqlite::Connection;

use crate::error::CoreError;

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A vault on disk, identified by path. Cheap to construct; does no I/O.
pub struct VaultFile {
    path: PathBuf,
}

/// An unlocked vault. Dropping it (or calling [`Session::lock`]) closes the
/// SQLCipher connection *first*, which makes SQLCipher wipe its own copy of the
/// file key, and then zeroizes ours.
pub struct Session {
    conn: Option<Connection>,
    mk: Mk,
    data_key: DataKey,
    header: FileHeader,
    email_norm: String,
    last_activity: Instant,
}

impl VaultFile {
    pub fn at(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn exists(&self) -> bool {
        self.path.exists()
    }

    /// First-run setup. CPU-bound (Argon2id); dispatch via `spawn_blocking`.
    ///
    /// `profile` is chosen by calibration on the user's machine and is recorded
    /// in the plaintext file header so unlock knows the cost parameters before
    /// spending a single Argon2 cycle.
    pub fn create(
        &self,
        email: &str,
        password_raw: &str,
        profile: Profile,
    ) -> Result<Session, CoreError> {
        let pw = neko_vault::normalize::password(password_raw);
        let strength = password::estimate(&pw);
        if !strength.acceptable() {
            return Err(CoreError::WeakPassword(strength.warnings));
        }
        let email_norm = neko_vault::normalize::email(email);

        let header = FileHeader::new(profile)?;
        let stretched = keys::stretch(&email_norm, &pw, &header)?;
        let file_key = keys::file_key(&stretched)?;

        let vault_salt = neko_crypto::random(16)?;
        let kek = keys::kek(&stretched, &vault_salt)?;
        let mk = keys::new_mk()?;

        let extra = keys::vault_aad_extra(profile.params, &vault_salt);
        let wrapped_mk = keys::wrap_mk(&kek, &mk, 1, &extra)?;
        let verifier = keys::verifier(&mk)?;

        let conn = neko_store::create(&self.path, &file_key, &header)?;
        vault_row::init_schema(&conn)?;
        vault_row::insert(
            &conn,
            &VaultRow {
                blob_version: BLOB_VERSION,
                kdf_profile: profile.id,
                params: profile.params,
                key_ver: 1,
                vault_salt: vault_salt.to_vec(),
                email_norm: email_norm.clone(),
                wrapped_mk,
                wrapped_mk_prev: None,
                rewrap_state: 0,
                verifier: verifier.to_vec(),
                wallet_seq: 0,
            },
            now(),
        )?;

        // Prove the header actually landed in bytes 0..16 rather than trusting it.
        let on_disk = neko_store::read_header(&self.path)?;
        debug_assert_eq!(on_disk.as_bytes(), header.as_bytes());

        let data_key = keys::data_key(&mk)?;
        Ok(Session {
            conn: Some(conn),
            mk,
            data_key,
            header,
            email_norm,
            last_activity: Instant::now(),
        })
    }

    /// Unlock an existing vault. CPU-bound; dispatch via `spawn_blocking`.
    pub fn unlock(&self, email: &str, password_raw: &str) -> Result<Session, CoreError> {
        let pw = neko_vault::normalize::password(password_raw);
        let email_norm = neko_vault::normalize::email(email);

        // The salt travels inside the file: read it with no key at all.
        let header = neko_store::read_header(&self.path)?;
        let stretched = keys::stretch(&email_norm, &pw, &header)?;
        let file_key = keys::file_key(&stretched)?;

        // Wrong credentials surface here, as SQLCipher's page-1 HMAC failing.
        let conn = neko_store::open::open(&self.path, &file_key, &header)
            .map_err(|_| CoreError::WrongCredentials)?;

        let row = vault_row::load(&conn)?.ok_or(CoreError::WrongCredentials)?;

        // Authenticate the plaintext header against the encrypted record.
        keys::assert_profile_matches(&header, row.params)?;
        row.params.validate()?;

        let kek = keys::kek(&stretched, &row.vault_salt)?;
        let extra = keys::vault_aad_extra(row.params, &row.vault_salt);

        // If a password change was interrupted, the current wrap may be the new
        // one while the file is still under the old key. Try both.
        let mk = match keys::unwrap_mk(&kek, &row.wrapped_mk, row.key_ver, &extra) {
            Ok(mk) => mk,
            Err(e) => match row.wrapped_mk_prev.as_deref() {
                Some(prev) => keys::unwrap_mk(&kek, prev, row.key_ver, &extra)?,
                None => return Err(e.into()),
            },
        };

        if !neko_crypto::ct_eq(&keys::verifier(&mk)?, &row.verifier) {
            return Err(CoreError::WrongCredentials);
        }

        let data_key = keys::data_key(&mk)?;
        Ok(Session {
            conn: Some(conn),
            mk,
            data_key,
            header,
            email_norm,
            last_activity: Instant::now(),
        })
    }
}

impl Session {
    pub fn conn(&self) -> Result<&Connection, CoreError> {
        self.conn.as_ref().ok_or(CoreError::Locked)
    }

    pub fn conn_mut(&mut self) -> Result<&mut Connection, CoreError> {
        self.conn.as_mut().ok_or(CoreError::Locked)
    }

    pub fn data_key(&self) -> &DataKey {
        &self.data_key
    }

    pub fn mk(&self) -> &Mk {
        &self.mk
    }

    pub fn email(&self) -> &str {
        &self.email_norm
    }

    pub fn header(&self) -> &FileHeader {
        &self.header
    }

    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    pub fn idle(&self) -> std::time::Duration {
        self.last_activity.elapsed()
    }

    /// Order matters: drop the connection FIRST so SQLCipher frees its own copy
    /// of the file key. Holding it open while "locked" would be theatre.
    pub fn lock(&mut self) {
        self.conn.take();
    }

    /// Change the password. MK is untouched, so every wallet, address and
    /// ciphertext stays valid; only the outer file key rotates.
    ///
    /// Crash safety: the new wrap is staged alongside the old one before the
    /// rekey, and only cleared after it succeeds. An interruption at any point
    /// leaves a vault that [`VaultFile::unlock`] can still open.
    pub fn change_password(
        &mut self,
        path: &Path,
        new_password_raw: &str,
    ) -> Result<(), CoreError> {
        let pw = neko_vault::normalize::password(new_password_raw);
        let strength = password::estimate(&pw);
        if !strength.acceptable() {
            return Err(CoreError::WeakPassword(strength.warnings));
        }

        let conn = self.conn()?;
        let row = vault_row::load(conn)?.ok_or(CoreError::Locked)?;

        let stretched = keys::stretch(&self.email_norm, &pw, &self.header)?;
        let new_file_key: FileKey = keys::file_key(&stretched)?;
        let new_kek = keys::kek(&stretched, &row.vault_salt)?;
        let extra = keys::vault_aad_extra(row.params, &row.vault_salt);
        let new_wrapped = keys::wrap_mk(&new_kek, &self.mk, row.key_ver, &extra)?;

        vault_row::stage_rewrap(conn, &new_wrapped, now())?;
        neko_store::rekey(conn, &new_file_key)?;
        vault_row::finish_rewrap(conn)?;

        let _ = path;
        Ok(())
    }
}

/// Redacted at the type level. A Session holds MK and the data key; no `dbg!`
/// or `{:?}` anywhere in the tree may ever print them.
impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("email", &self.email_norm)
            .field("locked", &self.conn.is_none())
            .field("keys", &"[redacted]")
            .finish()
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.lock();
    }
}

// ── Settings ───────────────────────────────────────────────────────────────

impl Session {
    pub fn setting(&self, key: &str) -> Result<Option<String>, CoreError> {
        Ok(neko_store::repo::settings::get(self.conn()?, key)?)
    }

    pub fn set_setting(&self, key: &str, value: &str) -> Result<(), CoreError> {
        Ok(neko_store::repo::settings::set(self.conn()?, key, value)?)
    }

    /// Credentials are encrypted like any other secret column.
    pub fn secret_setting(
        &self,
        key: &str,
    ) -> Result<Option<zeroize::Zeroizing<String>>, CoreError> {
        Ok(neko_store::repo::settings::get_secret(
            self.conn()?,
            self.data_key(),
            key,
        )?)
    }

    pub fn set_secret_setting(&self, key: &str, value: &str) -> Result<(), CoreError> {
        Ok(neko_store::repo::settings::set_secret(
            self.conn()?,
            self.data_key(),
            key,
            value,
        )?)
    }
}

impl Session {
    /// Re-check the vault password without disturbing this session.
    ///
    /// Runs the full Argon2id derivation deliberately: the point is to prove a
    /// human is present right now, and a check against the already-unlocked
    /// key in memory would prove nothing. CPU-bound — call it from a blocking
    /// context, never on an async executor.
    pub fn verify_password(&self, path: &Path, password: &str) -> bool {
        VaultFile::at(path)
            .unlock(&self.email_norm, password)
            .is_ok()
    }
}
