-- Linea.
--
-- A zkEVM holding ether. Coin type 60.
--
-- The fifth row to say 'ETH' with no contract. Legal because `assets` is keyed
-- on (chain_id, symbol) rather than on symbol alone.

INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (14, 'linea', 60, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (14, 14, 'ETH', NULL, 18, 1);

PRAGMA user_version = 14;
