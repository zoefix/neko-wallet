-- Scroll.
--
-- A zkEVM holding ether, and the one chain here that charges for L1 at an
-- address of its own rather than the OP-stack predeploy. Coin type 60.
--
-- Seven rows now say 'ETH' with no contract, on chains 5, 8, 9, 10, 14, 15
-- and 16.

INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (16, 'scroll', 60, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (16, 16, 'ETH', NULL, 18, 1);

PRAGMA user_version = 16;
