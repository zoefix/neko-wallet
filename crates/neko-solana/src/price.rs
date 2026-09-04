//! What one SOL is worth, read from a pool on the chain we are already
//! connected to.
//!
//! Same rule as the other two chains: no third party. The wallet talks to the
//! cluster you point it at and to nothing else, so a price cannot be the thing
//! that tells somebody where your addresses are.
//!
//! The pool is verified before it is believed. A Raydium AMM account states its
//! own two mints, and those are checked against wrapped SOL and USDT on every
//! read - so a pool that was migrated, drained into a different pair, or simply
//! mistyped here yields *no price* rather than a wrong one. That matters more
//! than usual: a price is what turns a balance into a figure somebody decides
//! against.

use neko_hd::SolanaAddress;

use crate::client::Rpc;
use crate::error::SolanaError;

/// Raydium AMM v4's SOL/USDT pool.
///
/// USDT rather than the deeper USDC pool, so this chain quotes in the same unit
/// as TRON and BNB Chain and a portfolio total does not silently mix two
/// stablecoins. The two pools were compared at the time of writing and agreed
/// to within 0.07%.
pub const SOL_USDT_POOL: &str = "7XawhbbxtsRcQA8KTkHT9f9nc6d69UwqCDh6U5EEbEmX";

/// Wrapped SOL, which is what a pool holds rather than SOL itself.
pub const WRAPPED_SOL: &str = "So11111111111111111111111111111111111111112";

/// Offsets into Raydium's 752-byte `AmmInfo`. Confirmed against two live pools
/// whose vaults and mints were already known from another source.
const OFF_COIN_VAULT: usize = 336;
const OFF_PC_VAULT: usize = 368;
const OFF_COIN_MINT: usize = 400;
const OFF_PC_MINT: usize = 432;
const AMM_INFO_LEN: usize = 752;

fn at(data: &[u8], off: usize) -> Result<SolanaAddress, SolanaError> {
    data.get(off..off + 32)
        .ok_or_else(|| SolanaError::BadReply("pool account is truncated".into()))
        .and_then(|b| SolanaAddress::from_bytes(b).map_err(Into::into))
}

/// One SOL, in USDT, at `scale` decimal places.
///
/// `None` is a real answer here, and the caller must not turn it into a zero:
/// a portfolio total with a missing price is unavailable, not smaller.
pub async fn sol_in_usdt(rpc: &Rpc, scale: u8) -> Result<i128, SolanaError> {
    let pool = SolanaAddress::parse(SOL_USDT_POOL)?;
    let data = rpc.account_data(pool).await?;
    if data.len() != AMM_INFO_LEN {
        return Err(SolanaError::BadReply(format!(
            "{SOL_USDT_POOL} is {} bytes, not a Raydium AMM account",
            data.len()
        )));
    }

    // The pool says what it holds. Believing that rather than this file's
    // constants is what makes a migrated or mistyped pool fail loudly.
    let coin_mint = at(&data, OFF_COIN_MINT)?;
    let pc_mint = at(&data, OFF_PC_MINT)?;
    if coin_mint != SolanaAddress::parse(WRAPPED_SOL)? {
        return Err(SolanaError::BadReply(format!(
            "pool {SOL_USDT_POOL} holds {coin_mint}, not wrapped SOL - refusing to price from it"
        )));
    }
    if pc_mint != crate::chain_consts::usdt_mint() {
        return Err(SolanaError::BadReply(format!(
            "pool {SOL_USDT_POOL} quotes in {pc_mint}, not USDT - refusing to price from it"
        )));
    }

    let coin = rpc
        .token_account_balance(at(&data, OFF_COIN_VAULT)?)
        .await?;
    let pc = rpc.token_account_balance(at(&data, OFF_PC_VAULT)?).await?;
    spot(coin.amount, coin.decimals, pc.amount, pc.decimals, scale)
        .ok_or_else(|| SolanaError::BadReply("the pool is empty".into()))
}

/// The arithmetic, split out so it can be tested without a cluster.
///
/// Integer throughout, and `None` rather than a wrapped number on overflow -
/// the same rule as everywhere else that touches money.
pub fn spot(
    coin_amount: u64,
    coin_decimals: u8,
    pc_amount: u64,
    pc_decimals: u8,
    scale: u8,
) -> Option<i128> {
    if coin_amount == 0 {
        return None;
    }
    // price = (pc / 10^pc_dec) / (coin / 10^coin_dec), at 10^scale
    //       = pc * 10^(coin_dec + scale) / (10^pc_dec * coin)
    let up = 10i128.checked_pow(coin_decimals as u32 + scale as u32)?;
    let down = 10i128.checked_pow(pc_decimals as u32)?;
    (pc_amount as i128)
        .checked_mul(up)?
        .checked_div(down.checked_mul(coin_amount as i128)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The live figures at the time this was written: 3,545.45525741 SOL
    /// against 367,143.399606 USDT. Two independent pools were read - this one
    /// and the deeper SOL/USDC pool, which gave 103.62 - so the arithmetic is
    /// pinned against a number that was cross-checked rather than computed once.
    #[test]
    fn the_spot_price_matches_what_the_pools_said() {
        let price = spot(3_545_455_257_410, 9, 367_143_399_606, 6, 6).unwrap();
        assert_eq!(price, 103_553_245, "expected about 103.55 USDT per SOL");

        // The deeper SOL/USDC pool, read at the same moment: 70,234.18044945
        // SOL against 7,277,886.425505 USDC. Two pools that were never told
        // about each other, agreeing to 0.07%, is what says this arithmetic is
        // right - one pool alone would only say it is self-consistent.
        let cross = spot(70_234_180_449_450, 9, 7_277_886_425_505, 6, 6).unwrap();
        assert_eq!(cross, 103_623_141);
        let gap = (price - cross).abs() * 10_000 / cross;
        assert!(
            gap < 100,
            "the two pools disagree by more than 1%: {gap} bp"
        );
    }

    /// Nine decimals on one side and six on the other. Getting the direction of
    /// that adjustment wrong is a factor of a thousand, in either direction.
    #[test]
    fn the_decimal_adjustment_goes_the_right_way() {
        // A pool holding 1 SOL and 200 USDT prices SOL at 200, not 0.005.
        assert_eq!(spot(1_000_000_000, 9, 200_000_000, 6, 6), Some(200_000_000));
        // Same pair, same precision on both sides: still 200.
        assert_eq!(spot(1_000_000, 6, 200_000_000, 6, 6), Some(200_000_000));
    }

    /// An empty pool has no price. Zero would say SOL is worthless.
    #[test]
    fn an_empty_pool_yields_nothing() {
        assert_eq!(spot(0, 9, 1_000_000, 6, 6), None);
        // No USDT in it is a price of zero, which is different from no price -
        // and is what a pool being drained actually looks like.
        assert_eq!(spot(1_000_000_000, 9, 0, 6, 6), Some(0));
    }

    #[test]
    fn the_pool_and_mint_constants_are_well_formed() {
        assert_eq!(
            SolanaAddress::parse(SOL_USDT_POOL).unwrap().to_string(),
            SOL_USDT_POOL
        );
        assert_eq!(
            SolanaAddress::parse(WRAPPED_SOL).unwrap().to_string(),
            WRAPPED_SOL
        );
    }
}
