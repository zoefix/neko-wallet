//! What one GRAM is worth, read from a pool on the chain we are already
//! connected to.

//! Same rule as the other four chains: no third party. The wallet talks to the
//! node you point it at and to nothing else, so a price cannot be the thing
//! that tells somebody which addresses you hold.
//!
//! The pool is verified before it is believed. A DeDust pool states its own two
//! assets, and those are checked against native GRAM and the USDT master on
//! every read - so a pool that was migrated, drained into a different pair, or
//! simply mistyped here yields *no price* rather than a wrong one. That matters
//! more than usual: a price is what turns a balance into a figure somebody
//! decides against.
//!
//! One thing here is unlike Raydium and PancakeSwap: **the pool does not hold
//! the money**. DeDust keeps every asset in a shared vault and the pool holds
//! only an accounting entry, so the pool contract's own balance is a few cents
//! and says nothing about the pair's depth. Reading a balance to sanity-check a
//! reserve, which works on the other chains, silently means nothing here.

use crate::address::TonAddress;
use crate::cell::Cell;
use crate::chain_consts;
use crate::client::Toncenter;
use crate::error::TonError;
use std::sync::Arc;

/// DeDust's GRAM/USDT pool.
///
/// USDT rather than a deeper stable pair, so this chain quotes in the same unit
/// as the other five and a portfolio total does not silently mix two
/// stablecoins.
pub const GRAM_USDT_POOL: &str = "EQA-X_yo3fzzbDbJ_0bzFWKqtRuZFIRa1sJsveZJ1YpViO3r";

/// How DeDust tags an asset, in the first four bits of the cell describing it.
const ASSET_NATIVE: u8 = 0b0000;
const ASSET_JETTON: u8 = 0b0001;

/// One GRAM, in USDT, at `scale` decimal places.
///
/// An error here is a real answer, and the caller must not turn it into a zero:
/// a portfolio total with a missing price is unavailable, not smaller.
pub async fn gram_in_usdt(node: &Toncenter, scale: u8) -> Result<i128, TonError> {
    let pool = TonAddress::parse(GRAM_USDT_POOL)?;

    // The pool says what it holds. Believing that rather than this file's
    // constants is what makes a migrated or mistyped pool fail loudly.
    let assets = node.get_method_cells(&pool, "get_assets", &[]).await?;
    let [first, second] = assets.as_slice() else {
        return Err(TonError::BadReply(format!(
            "{GRAM_USDT_POOL} named {} assets, not two",
            assets.len()
        )));
    };
    if asset_kind(first)? != ASSET_NATIVE {
        return Err(TonError::BadReply(format!(
            "{GRAM_USDT_POOL} does not hold native GRAM on its first side"
        )));
    }
    let quoted = jetton_master(second)?;
    let usdt = chain_consts::usdt_master();
    if quoted != usdt {
        return Err(TonError::BadReply(format!(
            "{GRAM_USDT_POOL} quotes {quoted}, not USDT"
        )));
    }

    let reserves = node.get_method_ints(&pool, "get_reserves", &[]).await?;
    let [gram, usdt_side] = reserves.as_slice() else {
        return Err(TonError::BadReply(format!(
            "{GRAM_USDT_POOL} named {} reserves, not two",
            reserves.len()
        )));
    };

    // The reserves come back in the order the assets did, which is why the
    // assets are checked first and in order rather than as a set.
    spot(
        *gram,
        chain_consts::GRAM_DECIMALS,
        *usdt_side,
        chain_consts::USDT_DECIMALS,
        scale,
    )
    .ok_or_else(|| TonError::BadReply("the pool is empty on one side".into()))
}

/// The four bits that say what kind of asset a cell describes.
fn asset_kind(c: &Arc<Cell>) -> Result<u8, TonError> {
    if c.bits() < 4 {
        return Err(TonError::BadReply("an asset cell is empty".into()));
    }
    Ok(c.data()[0] >> 4)
}

/// The master address out of a jetton asset cell: four bits of tag, eight of
/// workchain, then the account. None of it is byte-aligned.
fn jetton_master(c: &Arc<Cell>) -> Result<TonAddress, TonError> {
    if asset_kind(c)? != ASSET_JETTON {
        return Err(TonError::BadReply(
            "the quote side of the pool is not a jetton".into(),
        ));
    }
    if c.bits() < 4 + 8 + 256 {
        return Err(TonError::BadReply("a jetton asset cell is short".into()));
    }
    let bit = |i: usize| (c.data()[i / 8] >> (7 - (i % 8))) & 1;
    let mut wc = 0u8;
    for i in 0..8 {
        wc = (wc << 1) | bit(4 + i);
    }
    let mut hash = [0u8; 32];
    for i in 0..256 {
        hash[i / 8] = (hash[i / 8] << 1) | bit(12 + i);
    }
    Ok(TonAddress::new(wc as i8, hash))
}

/// The arithmetic, split out so it can be tested without a node.
///
/// Integer throughout, and `None` rather than a wrapped number on overflow -
/// the same rule as everywhere else that touches money.
pub fn spot(
    gram_reserve: u128,
    gram_decimals: u8,
    usdt_reserve: u128,
    usdt_decimals: u8,
    scale: u8,
) -> Option<i128> {
    if gram_reserve == 0 {
        return None;
    }
    // price = (usdt / 10^usdt_dec) / (gram / 10^gram_dec), at 10^scale
    //       = usdt * 10^(gram_dec + scale) / (10^usdt_dec * gram)
    let up = 10i128.checked_pow(gram_decimals as u32 + scale as u32)?;
    let down = 10i128.checked_pow(usdt_decimals as u32)?;
    i128::try_from(usdt_reserve)
        .ok()?
        .checked_mul(up)?
        .checked_div(down.checked_mul(i128::try_from(gram_reserve).ok()?)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cell::CellBuilder;

    /// The live pool at the time this was written: 203,231.463016165 GRAM
    /// against 284,190.508403 USDT.
    ///
    /// Cross-checked against the pool's own `estimate_swap_out`, which reaches
    /// the same reserves through the contract's swap arithmetic rather than
    /// through this file's. Asked to swap one whole GRAM, it answered
    /// 1.396953 USDT and named a fee of 0.001 GRAM - so the two agree to
    /// within the price impact of that swap. A reserve read backwards would
    /// have put the spot at 0.715 and missed by a factor of two.
    #[test]
    fn the_spot_price_matches_what_the_pool_said() {
        let price = spot(203_231_463_016_165, 9, 284_190_508_403, 6, 6).unwrap();
        assert_eq!(price, 1_398_358, "expected about 1.3984 USDT per GRAM");

        // What the contract itself quoted, and the fee it charged to do it.
        const SWAP_IN: i128 = 1_000_000_000; // one GRAM
        const SWAP_OUT: i128 = 1_396_953; // USDT units
        const SWAP_FEE: i128 = 1_000_000; // nanotons, taken off the input

        // Spot, applied by hand to what the pool actually swapped: the input
        // less the fee, at this price. The remaining gap is the swap moving
        // the pool it is priced against, and on a 203,000 GRAM pool that is
        // a rounding error - but it is one that only appears if the price is
        // right.
        let expected = (SWAP_IN - SWAP_FEE) * price / 1_000_000_000;
        let gap = (expected - SWAP_OUT).abs();
        assert!(
            gap <= 10,
            "spot says a fee-adjusted GRAM buys {expected}, the pool paid {SWAP_OUT}"
        );

        // And the fee is the pool's, not a guess: a tenth of a percent.
        assert_eq!(SWAP_FEE * 10_000 / SWAP_IN, 10, "DeDust charges 10 bp here");
    }

    /// Nine decimals on one side and six on the other. Getting the direction of
    /// that adjustment wrong is a factor of a thousand, in either direction -
    /// and 1.4 and 1400 are both figures somebody might believe.
    #[test]
    fn the_decimal_adjustment_goes_the_right_way() {
        // A pool holding 1 GRAM and 200 USDT prices GRAM at 200, not 0.005.
        assert_eq!(spot(1_000_000_000, 9, 200_000_000, 6, 6), Some(200_000_000));
        // Same pair, same precision on both sides: still 200.
        assert_eq!(spot(1_000_000, 6, 200_000_000, 6, 6), Some(200_000_000));
        // An empty pool has no price, rather than a price of zero.
        assert_eq!(spot(0, 9, 200_000_000, 6, 6), None);
    }

    /// The bytes the live pool actually returned for `get_assets`. Neither
    /// field is byte-aligned, which is the whole reason this is read bit by
    /// bit rather than sliced.
    #[test]
    fn the_pool_assets_decode_to_gram_and_usdt() {
        let native = crate::boc::parse(&base64(b"te6cckEBAQEAAwAAAQiFl+L/")).unwrap();
        assert_eq!(asset_kind(&native).unwrap(), ASSET_NATIVE);
        assert!(jetton_master(&native).is_err(), "native is not a jetton");

        let jetton = crate::boc::parse(&base64(
            b"te6cckEBAQEAJAAAQxALETqZS1AkoWcZ9pE5Mo63WVlsOKJfWQKLFG/s3DYh3+iNJO1L",
        ))
        .unwrap();
        assert_eq!(asset_kind(&jetton).unwrap(), ASSET_JETTON);
        assert_eq!(
            jetton_master(&jetton).unwrap(),
            chain_consts::usdt_master(),
            "the quote side of the pool is USDT"
        );
    }

    /// A pool holding some other token has to yield no price rather than a
    /// wrong one - which means the check has to reject an address that is
    /// merely close.
    #[test]
    fn a_different_jetton_is_not_usdt() {
        let real = chain_consts::usdt_master();
        let mut near = real.hash;
        near[31] ^= 1;
        let other = TonAddress::new(0, near);
        assert_ne!(other, real);

        let mut b = CellBuilder::new();
        b.store_uint(ASSET_JETTON as u64, 4).unwrap();
        b.store_uint(0, 8).unwrap();
        b.store_bytes(&other.hash).unwrap();
        let cell = b.build_arc().unwrap();
        assert_eq!(jetton_master(&cell).unwrap(), other);

        // And the same builder round-trips the genuine one, so the failure
        // above is the address and not the encoding.
        let mut b = CellBuilder::new();
        b.store_uint(ASSET_JETTON as u64, 4).unwrap();
        b.store_uint(0, 8).unwrap();
        b.store_bytes(&real.hash).unwrap();
        assert_eq!(jetton_master(&b.build_arc().unwrap()).unwrap(), real);
    }

    fn base64(s: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        let (mut acc, mut bits) = (0u32, 0u32);
        for ch in s.iter().copied() {
            let v = match ch {
                b'A'..=b'Z' => ch - b'A',
                b'a'..=b'z' => ch - b'a' + 26,
                b'0'..=b'9' => ch - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => break,
                _ => panic!("not base64"),
            } as u32;
            acc = (acc << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        out
    }
}
