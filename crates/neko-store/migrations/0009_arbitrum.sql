-- Arbitrum One.
--
-- The fifth EVM chain, the third whose coin is ETH, and the third migration in
-- a row that changes no schema. Coin type 60 again.

INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (9, 'arbitrum', 60, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (9, 9, 'ETH', NULL, 18, 1);

-- Three rows now say 'ETH' with no contract, on chains 5, 8 and 9. That is
-- legal because `assets` is keyed on (chain_id, symbol) rather than on symbol
-- alone - which was true from 0001 and is only now being leaned on.
--
-- What is not modelled: that this chain's USDT contract calls itself `USD₮0`,
-- with a tugrik sign where the T should be. The stored symbol is what the
-- wallet shows; what the contract calls itself is checked against the chain at
-- send time, where a mismatch can stop a transfer rather than sit in a row.

PRAGMA user_version = 9;
