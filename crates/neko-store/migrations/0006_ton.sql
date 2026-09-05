-- TON, whose coin is GRAM.
--
-- The sixth chain, and the first whose *address is not derived from a key*.
-- Everywhere else the address is a hash or an encoding of the public key, and
-- deriving it is arithmetic. Here a wallet is a contract, and its address is
-- the hash of that contract's initial code and storage - which includes the
-- public key, but also the wallet version and a subwallet id. Change the
-- wallet version and the same phrase gives a different address.
--
-- The schema does not model any of that, and does not need to: what it stores
-- is still one address per account, and the derivation lives in code where the
-- constants that produced it can be tested against the live chain.

-- 1. The chain and its native coin. Coin type 607 is TON's own. GRAM has nine
--    decimals - the same as SOL, one more than BTC, nine fewer than BNB.
INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (6, 'ton', 607, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (6, 6, 'GRAM', NULL, 9, 1);

-- 2. `addresses.address_raw` has never seen 33 bytes. A TON address is a
--    workchain byte followed by a 32-byte hash, and both halves matter: the
--    same hash in workchain -1 is the masterchain, a different place entirely.
--    Storing only the hash would make two distinct addresses compare equal.
--
--    SQLite cannot alter a CHECK, so the table is rebuilt - ids and all,
--    because `balances` references them. Widening rather than dropping, for
--    the reason 0003 and 0004 gave: the length is the one cheap guard against
--    a truncated address reaching the column incoming payments are matched on.
CREATE TABLE addresses_new (
  id          INTEGER PRIMARY KEY,
  account_id  INTEGER NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
  deriv_index INTEGER NOT NULL CHECK (deriv_index BETWEEN 0 AND 16777215),
  change      INTEGER NOT NULL DEFAULT 0,
  address     TEXT NOT NULL,
  address_raw BLOB NOT NULL CHECK (length(address_raw) IN (20, 21, 22, 23, 25, 32, 33, 34)),
  label_ct    BLOB,
  UNIQUE (account_id, change, deriv_index)
);

INSERT INTO addresses_new (id, account_id, deriv_index, change, address, address_raw, label_ct)
  SELECT id, account_id, deriv_index, change, address, address_raw, label_ct FROM addresses;

DROP TABLE addresses;
ALTER TABLE addresses_new RENAME TO addresses;

-- What does NOT change: `assets.decimals` allows 0-18, and GRAM is 9 with its
-- USDT at 6. And nothing here records that a jetton balance lives in a separate
-- contract per holder - that is where the number is read *from*, not what the
-- number is, and `balances` carries the same single figure it does everywhere.

PRAGMA user_version = 6;
