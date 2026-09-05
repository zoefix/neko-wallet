-- Optimism.
--
-- The sixth EVM chain, the fourth whose coin is ETH, and no schema change
-- again. Coin type 60.

INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (10, 'optimism', 60, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (10, 10, 'ETH', NULL, 18, 1);

-- A coincidence worth naming, because it is the only one in this file and it
-- invites a wrong conclusion: this chain's row id is 10 and its *EVM* chain id
-- is also 10. Nothing connects them. These ids number the chains this wallet
-- supports, in the order they were added; the EVM chain id is the number that
-- goes into a signature and decides which network it is valid on. Every other
-- chain here makes the difference obvious - Base is row 8 and chain 8453 - so
-- this is the one row where the two could be mistaken for each other.
--
-- Four rows now say 'ETH' with no contract, on chains 5, 8, 9 and 10.

PRAGMA user_version = 10;
