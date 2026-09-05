-- HyperEVM.
--
-- The EVM side of Hyperliquid. Coin type 60.
--
-- The one chain in this database whose coin the wallet will not price: HYPE
-- exists on no other chain here, and its only V2 pool holds about a thousand
-- dollars. The balance is real and the value column says it does not know.

INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (12, 'hyperevm', 60, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (12, 12, 'HYPE', NULL, 18, 1);

PRAGMA user_version = 12;
