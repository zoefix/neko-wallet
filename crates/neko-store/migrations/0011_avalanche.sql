-- Avalanche C-Chain.
--
-- The seventh EVM chain, and the first added since Polygon whose coin is
-- neither ether nor a rename of something else. Coin type 60, like every EVM
-- chain here.
--
-- Not modelled, and checked against the chain at send time instead: its USDT
-- contract calls itself `USDt`, with a lowercase t.

INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (11, 'avalanche', 60, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (11, 11, 'AVAX', NULL, 18, 1);

PRAGMA user_version = 11;
