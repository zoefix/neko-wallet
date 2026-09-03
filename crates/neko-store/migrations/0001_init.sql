-- neko-wallet schema v1.
--
-- Everything here already sits inside a SQLCipher-encrypted file. The second,
-- field-level AEAD layer (columns suffixed `_ct`) is defence in depth for the
-- window where the database is already open: a process memory dump, a future
-- export feature that reads rows carelessly, or a SQLCipher vulnerability.
--
-- Rule for which columns get layer 2: anything catastrophic if disclosed OR if
-- swapped between rows, plus private free text. NOT anything we need to index,
-- join, sort, or aggregate on -- SQLCipher already covers those, and encrypting
-- them would only cost us SQL.

CREATE TABLE vault (
  id              INTEGER PRIMARY KEY CHECK (id = 1),
  blob_version    INTEGER NOT NULL,
  kdf_profile     INTEGER NOT NULL,                  -- must equal file header byte 1
  kdf_mem_kib     INTEGER NOT NULL CHECK (kdf_mem_kib >= 65536),
  kdf_iters       INTEGER NOT NULL CHECK (kdf_iters   >= 2),
  kdf_par         INTEGER NOT NULL CHECK (kdf_par     >= 1),
  kdf_key_len     INTEGER NOT NULL CHECK (kdf_key_len >= 32),
  key_ver         INTEGER NOT NULL DEFAULT 1,
  vault_salt      BLOB    NOT NULL CHECK (length(vault_salt) = 16),
  email_norm      TEXT    NOT NULL,                  -- display only, after unlock
  wrapped_mk      BLOB    NOT NULL CHECK (length(wrapped_mk) = 72),  -- 24 nonce + 32 ct + 16 tag
  -- Crash safety for password change: the only place a torn write could
  -- orphan the vault. Both wraps live here during the transition.
  wrapped_mk_prev BLOB CHECK (wrapped_mk_prev IS NULL OR length(wrapped_mk_prev) = 72),
  rewrap_state    INTEGER NOT NULL DEFAULT 0,
  verifier        BLOB    NOT NULL CHECK (length(verifier) = 32),
  wallet_seq      INTEGER NOT NULL DEFAULT 0,
  created_at      INTEGER NOT NULL,
  rewrapped_at    INTEGER
);

CREATE TABLE chains (
  id        INTEGER PRIMARY KEY,
  slug      TEXT NOT NULL UNIQUE,
  coin_type INTEGER NOT NULL,
  enabled   INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE assets (
  id       INTEGER PRIMARY KEY,
  chain_id INTEGER NOT NULL REFERENCES chains(id),
  symbol   TEXT NOT NULL,
  contract BLOB,                                     -- NULL = native coin
  decimals INTEGER NOT NULL CHECK (decimals BETWEEN 0 AND 18),
  enabled  INTEGER NOT NULL DEFAULT 1,
  UNIQUE (chain_id, symbol)
);

CREATE TABLE wallets (
  id            INTEGER PRIMARY KEY,
  seq           INTEGER NOT NULL UNIQUE,
  origin        TEXT NOT NULL CHECK (origin IN
                  ('generated','imported_mnemonic','imported_privkey')),
  wordlist_lang TEXT NOT NULL DEFAULT 'en',
  label_ct      BLOB,        -- AEAD(k_data)  "company float" is itself intelligence
  entropy_ct    BLOB,        -- AEAD(k_data)  BIP39 entropy, 16 or 32 bytes
  bip39_pass_ct BLOB,        -- AEAD(k_data)  the optional 25th word; without it
                             --               the seed cannot be rebuilt from entropy
  privkey_ct    BLOB,        -- AEAD(k_data)  single-key import
  status        TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','archived')),
  created_at    INTEGER NOT NULL,
  CHECK ((origin = 'imported_privkey') = (privkey_ct IS NOT NULL)),
  CHECK ((origin = 'imported_privkey') OR (entropy_ct IS NOT NULL))
);

CREATE TABLE accounts (
  id            INTEGER PRIMARY KEY,
  wallet_id     INTEGER NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
  chain_id      INTEGER NOT NULL REFERENCES chains(id),
  account_index INTEGER NOT NULL DEFAULT 0,
  UNIQUE (wallet_id, chain_id, account_index)
);

-- Addresses stay PLAINTEXT (inside the encrypted file). They are public chain
-- data, and encrypting them would force a blind index on every scan match while
-- making the address/address_raw drift bug far harder to detect.
CREATE TABLE addresses (
  id          INTEGER PRIMARY KEY,
  account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  deriv_index INTEGER NOT NULL CHECK (deriv_index BETWEEN 0 AND 16777215),
  change      INTEGER NOT NULL DEFAULT 0,
  address     TEXT NOT NULL,                          -- base58; what the user copies
  address_raw BLOB NOT NULL CHECK (length(address_raw) = 21),  -- what matching uses
  label_ct    BLOB,
  UNIQUE (account_id, change, deriv_index)
  -- Deliberately NOT globally unique on address_raw. Two wallets can hold the
  -- same address: importing the same private key twice, or importing a key that
  -- a derived wallet already covers. That is worth warning about in the UI, but
  -- it must not break wallet creation.
);

CREATE TABLE balances (
  address_id INTEGER NOT NULL REFERENCES addresses(id) ON DELETE CASCADE,
  asset_id   INTEGER NOT NULL REFERENCES assets(id),
  amount     BLOB NOT NULL,                           -- i128_blob: exact and sortable
  block_num  INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  PRIMARY KEY (address_id, asset_id)
) WITHOUT ROWID;

CREATE TABLE transactions (
  id          INTEGER PRIMARY KEY,
  chain_id    INTEGER NOT NULL REFERENCES chains(id),
  txid        BLOB NOT NULL CHECK (length(txid) = 32),
  block_num   INTEGER NOT NULL,
  block_ts    INTEGER NOT NULL,
  asset_id    INTEGER NOT NULL REFERENCES assets(id),
  from_raw    BLOB NOT NULL,
  to_raw      BLOB NOT NULL,
  amount      BLOB NOT NULL,                          -- i128_blob, minimal units
  direction   TEXT NOT NULL CHECK (direction IN ('in','out')),
  address_id  INTEGER NOT NULL REFERENCES addresses(id) ON DELETE CASCADE,
  success     INTEGER NOT NULL,                       -- failed txs still cost fees
  fee         BLOB,
  energy_used INTEGER,
  net_used    INTEGER,
  memo_ct     BLOB,
  -- Sending to yourself produces BOTH an 'in' and an 'out' row for one txid,
  -- so the idempotency key has to be the whole four-tuple.
  UNIQUE (txid, address_id, asset_id, direction)
);
CREATE INDEX tx_addr_block ON transactions(address_id, block_num DESC);

CREATE TABLE settings (
  key      TEXT PRIMARY KEY,
  value    TEXT,
  value_ct BLOB,                                      -- AEAD(k_data) when secret
  secret   INTEGER NOT NULL DEFAULT 0,
  CHECK ((secret = 1) = (value_ct IS NOT NULL))
);

INSERT INTO chains (id, slug, coin_type, enabled) VALUES (1, 'tron', 195, 1);
INSERT INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (1, 1, 'TRX', NULL, 6, 1);

PRAGMA user_version = 1;
