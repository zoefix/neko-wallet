-- zkSync Era.
--
-- Ether again, and a gas number unlike any other chain's: a plain transfer
-- estimates at about 178,000 rather than 21,000, because Era folds the cost of
-- publishing to Ethereum into gas. Nothing about that is stored here; it comes
-- from the node at quote time. Coin type 60.

INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (15, 'zksync_era', 60, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (15, 15, 'ETH', NULL, 18, 1);

PRAGMA user_version = 15;
