-- Base.
--
-- The fourth EVM chain and the second migration in a row that changes no
-- schema at all. Coin type 60 again, so the same phrase gives the same address
-- on BNB Chain, Ethereum, Polygon and here.

-- The coin is ETH - the same ETH as Ethereum's, which is why the assets table
-- carries the symbol per chain rather than assuming it is unique.
INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (8, 'base', 60, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (8, 8, 'ETH', NULL, 18, 1);

-- What does NOT change: `addresses.address_raw` already accepts 20 bytes, and
-- `assets.decimals` already covers 18 for the coin and 6 for this chain's USDT.
--
-- What is not modelled: that this chain's coin is priced on Ethereum, because
-- its own pool holds about seventeen dollars. That is a fact about where to
-- ask a question, not about what is stored.

PRAGMA user_version = 8;
