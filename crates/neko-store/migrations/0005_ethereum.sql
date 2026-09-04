-- Ethereum.
--
-- The chain the EVM support was written for, added last. Almost nothing has to
-- change, and that is the point of the schema having been chain-shaped from the
-- first migration.

-- Coin type 60 is Ethereum's own, and BNB Chain already borrows it - so one
-- phrase gives the *same address* on both, which is what every EVM wallet does
-- and what people expect.
INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (5, 'ethereum', 60, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (5, 5, 'ETH', NULL, 18, 1);

-- What does NOT change: the address column already accepts 20 bytes, because
-- BNB Chain's addresses are the same twenty. And `assets.decimals` allows 0-18,
-- which covers ETH's 18 and this chain's USDT at 6 - six here, eighteen on BNB
-- Chain, for the same token name.

PRAGMA user_version = 5;
