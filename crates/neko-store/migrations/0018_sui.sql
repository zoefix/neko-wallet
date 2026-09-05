-- Sui.
--
-- Coin type 784, and the second chain here where a balance is a set of objects
-- rather than a number - Bitcoin is the other. Nothing about that is stored:
-- the objects are read from the chain at send time, because their versions
-- change every time they are touched and a stale one is refused rather than
-- applied.
--
-- Its address is 32 bytes, like Solana's and Aptos's, and derived differently
-- from both: blake2b256(scheme || key), where Aptos hashes key || scheme with
-- SHA3. Same length, three different accounts.

INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (18, 'sui', 784, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (18, 18, 'SUI', NULL, 9, 1);

PRAGMA user_version = 18;
