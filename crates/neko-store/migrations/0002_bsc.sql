-- BNB Chain.
--
-- The schema was already multi-chain: `accounts` carries a chain_id and
-- addresses hang off accounts. Two things were not.

-- 1. The chain itself, and its native coin. Coin type 60 is Ethereum's, which
--    is what every EVM wallet uses for BNB Chain - so an address here matches
--    what MetaMask shows for the same phrase.
INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (2, 'bsc', 60, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (2, 2, 'BNB', NULL, 18, 1);

-- 2. `addresses.address_raw` was checked against TRON's 21 bytes. EVM
--    addresses are 20. SQLite cannot alter a CHECK, so the table is rebuilt --
--    ids and all, because `balances` references them.
--
--    Widening rather than dropping the check: the length is the one cheap
--    guard against a truncated or mis-encoded address reaching the column that
--    incoming transfers are matched on.
CREATE TABLE addresses_new (
  id          INTEGER PRIMARY KEY,
  account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  deriv_index INTEGER NOT NULL CHECK (deriv_index BETWEEN 0 AND 16777215),
  change      INTEGER NOT NULL DEFAULT 0,
  address     TEXT NOT NULL,
  address_raw BLOB NOT NULL CHECK (length(address_raw) IN (20, 21)),
  label_ct    BLOB,
  UNIQUE (account_id, change, deriv_index)
);

INSERT INTO addresses_new (id, account_id, deriv_index, change, address, address_raw, label_ct)
  SELECT id, account_id, deriv_index, change, address, address_raw, label_ct FROM addresses;

DROP TABLE addresses;
ALTER TABLE addresses_new RENAME TO addresses;

PRAGMA user_version = 2;
