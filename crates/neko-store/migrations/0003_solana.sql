-- Solana.
--
-- Two schema facts change, and one does not.

-- 1. The chain and its native coin. Coin type 501 is Solana's own; unlike BNB
--    Chain, which borrows Ethereum's 60, there is nothing to share here - the
--    curve is different, so the key is different.
INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (3, 'solana', 501, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (3, 3, 'SOL', NULL, 9, 1);

-- 2. `addresses.address_raw` allowed 20 or 21 bytes. A Solana address is a
--    32-byte Ed25519 public key. SQLite cannot alter a CHECK, so the table is
--    rebuilt - ids and all, because `balances` references them.
--
--    Widening rather than dropping: the length is the one cheap guard against
--    a truncated or mis-encoded address reaching the column that incoming
--    transfers are matched on. It matters more here than on the other chains,
--    because a Solana address carries no checksum of its own.
CREATE TABLE addresses_new (
  id          INTEGER PRIMARY KEY,
  account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  deriv_index INTEGER NOT NULL CHECK (deriv_index BETWEEN 0 AND 16777215),
  change      INTEGER NOT NULL DEFAULT 0,
  address     TEXT NOT NULL,
  address_raw BLOB NOT NULL CHECK (length(address_raw) IN (20, 21, 32)),
  label_ct    BLOB,
  UNIQUE (account_id, change, deriv_index)
);

INSERT INTO addresses_new (id, account_id, deriv_index, change, address, address_raw, label_ct)
  SELECT id, account_id, deriv_index, change, address, address_raw, label_ct FROM addresses;

DROP TABLE addresses;
ALTER TABLE addresses_new RENAME TO addresses;

-- What does NOT change: `assets.decimals` is checked to be 0-18 and SOL is 9,
-- SPL USDT is 6. Both already fit, so the constraint stays as it is.

PRAGMA user_version = 3;
