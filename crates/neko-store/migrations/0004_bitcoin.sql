-- Bitcoin.
--
-- The first chain here that is not account-based, and the schema notices in
-- exactly one place.

-- 1. The chain and its only asset. Coin type 0 is Bitcoin's own, and there is
--    no second row: no contracts, no tokens, one asset.
INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (4, 'bitcoin', 0, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (4, 4, 'BTC', NULL, 8, 1);

-- 2. `addresses.address_raw` has held one fixed width per chain: 20, 21 or 32
--    bytes. Bitcoin stores the *locking script*, and its length is the address
--    type - 22 for P2WPKH, 23 for P2SH, 25 for P2PKH, 34 for P2WSH and
--    Taproot. A single width would reject four of the five.
--
--    SQLite cannot alter a CHECK, so the table is rebuilt - ids and all,
--    because `balances` references them. Widening rather than dropping: the
--    length is the one cheap guard against a truncated address reaching the
--    column that incoming payments are matched on.
CREATE TABLE addresses_new (
  id          INTEGER PRIMARY KEY,
  account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  deriv_index INTEGER NOT NULL CHECK (deriv_index BETWEEN 0 AND 16777215),
  change      INTEGER NOT NULL DEFAULT 0,
  address     TEXT NOT NULL,
  address_raw BLOB NOT NULL CHECK (length(address_raw) IN (20, 21, 22, 23, 25, 32, 34)),
  label_ct    BLOB,
  UNIQUE (account_id, change, deriv_index)
);

INSERT INTO addresses_new (id, account_id, deriv_index, change, address, address_raw, label_ct)
  SELECT id, account_id, deriv_index, change, address, address_raw, label_ct FROM addresses;

DROP TABLE addresses;
ALTER TABLE addresses_new RENAME TO addresses;

-- What does NOT change: `assets.decimals` is checked to be 0-18 and BTC is 8.
-- And nothing here models unspent outputs. A Bitcoin balance is the sum of
-- coins held at an address, read from an index each time rather than
-- accumulated in a row - so `balances` carries the same single figure it does
-- for every other chain, and means the same thing.

PRAGMA user_version = 4;
