-- Polygon.
--
-- The third EVM chain, and the migration is the shortest here yet: nothing
-- about the schema has to change. That is the whole return on the `chains`
-- table having existed since 0001 and on the address column having been
-- widened once, in 0003, for a reason that was never Polygon's.

-- Coin type 60 is Ethereum's, and both BNB Chain and Polygon borrow it - so
-- one phrase gives the *same address* on all three, which is what every EVM
-- wallet does and what people expect.
INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (7, 'polygon', 60, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (7, 7, 'POL', NULL, 18, 1);

-- What does NOT change: `addresses.address_raw` already accepts 20 bytes,
-- because two EVM chains were already here. And `assets.decimals` allows 0-18,
-- which covers POL's 18 and this chain's USDT at 6 - six here and on Ethereum,
-- eighteen on BNB Chain, for the same token name.
--
-- What is not modelled: that Polygon's USDT contract calls itself `USDT0`. The
-- symbol stored against a balance is the one this wallet shows, and what the
-- contract calls itself is checked against the chain at send time rather than
-- written down here, where it would be a second copy to keep true.

PRAGMA user_version = 7;
