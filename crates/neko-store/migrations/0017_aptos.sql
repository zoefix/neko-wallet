-- Aptos.
--
-- The first chain added since TON that is not an EVM chain, and the first with
-- coin type 637. Its address is 32 bytes, the same width as Solana's - a
-- coincidence, and the width check treats them separately so that stays
-- visible.
--
-- Not modelled here: that its USDT is a *fungible asset* rather than a coin.
-- Aptos has both systems with different entry points, and which one a token
-- uses is a property of the token rather than of the chain. The contract also
-- calls itself `USDt`, with a lowercase t, which is checked against the chain
-- at send time.

INSERT OR IGNORE INTO chains (id, slug, coin_type, enabled) VALUES (17, 'aptos', 637, 1);
INSERT OR IGNORE INTO assets (id, chain_id, symbol, contract, decimals, enabled)
  VALUES (17, 17, 'APT', NULL, 8, 1);

PRAGMA user_version = 17;
