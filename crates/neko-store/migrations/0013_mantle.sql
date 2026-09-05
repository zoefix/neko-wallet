-- Mantle.
--
-- An OP-stack chain whose coin is MNT rather than ether, which is why it can
-- neither price itself nor borrow Ethereum's price. Coin type 60.

INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (13, 'mantle', 60, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (13, 13, 'MNT', NULL, 18, 1);

PRAGMA user_version = 13;
