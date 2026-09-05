//! Turning a pile of balances into one number.
//!
//! The number is a convenience and is labelled as an estimate, but the
//! arithmetic behind it is the same fixed-point integer work as everything
//! else here: no floats, and no silent rounding into a figure that looks
//! authoritative.
//!
//! Two rules shape the design, and both are about not lying:
//!
//! * **A missing price for an asset that is actually held makes the whole
//!   total unavailable.** Quietly leaving that asset out would understate the
//!   portfolio, and understating it is exactly the direction that makes
//!   somebody think they can afford a transfer they cannot.
//! * **The unit is USDT, not dollars.** They track each other and are not the
//!   same thing, and the price came from a swap pool rather than a currency
//!   market. The interface says so rather than printing a dollar sign and
//!   hoping.

use std::collections::BTreeMap;

use crate::amount::Amount;
use crate::chain::ChainId;

/// Decimal places prices and totals are carried at.
///
/// Six matches TRON's USDT, is finer than any figure a person reads, and keeps
/// the products well inside `i128` for balances far larger than anyone holds.
pub const PRICE_SCALE: u8 = 6;

/// What one unit of each native coin is worth, in USDT, at [`PRICE_SCALE`].
#[derive(Debug, Clone, Default)]
pub struct Prices {
    native: BTreeMap<ChainId, i128>,
    /// Unix seconds of the newest quote, or `None` if nothing is known.
    pub fetched_at: Option<i64>,
}

impl Prices {
    pub fn set_native(&mut self, chain: ChainId, price: i128, at: i64) {
        self.native.insert(chain, price);
        self.fetched_at = Some(self.fetched_at.map_or(at, |cur| cur.max(at)));
    }

    pub fn native(&self, chain: ChainId) -> Option<i128> {
        self.native.get(&chain).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.native.is_empty()
    }

    /// The price of one unit of `symbol` on `chain`.
    ///
    /// A dollar-pegged token is taken as one unit of account rather than
    /// quoted against itself - asking a pool what one USDT is worth in USDT
    /// would spend a call to be told "about one, minus the fee".
    ///
    /// The chain's own stablecoin, whichever it is: seven of them carry USDT
    /// and Base carries USDC. Comparing against the literal `"USDT"` valued
    /// every USDC balance at nothing.
    pub fn of(&self, chain: ChainId, symbol: &str) -> Option<i128> {
        if chain.stable().is_some_and(|a| a.symbol() == symbol) {
            return Some(10i128.pow(PRICE_SCALE as u32));
        }
        if symbol == chain.native_symbol() {
            return self.native(chain);
        }
        None
    }
}

/// What one holding is worth, at [`PRICE_SCALE`].
///
/// `None` on overflow rather than a wrapped number: a balance large enough to
/// overflow is absurd, and a wrapped total would be displayed as real.
pub fn value_of(amount: i128, decimals: u8, price: i128) -> Option<i128> {
    amount
        .checked_mul(price)
        .map(|v| v / 10i128.checked_pow(decimals as u32).unwrap_or(i128::MAX))
}

/// The total of several holdings, or `None` if any *non-zero* one cannot be
/// priced.
///
/// Zero holdings are skipped, so a wallet with no BNB still gets a total even
/// when the BNB quote failed - nothing is being left out of it.
pub fn total<'a>(
    holdings: impl IntoIterator<Item = (ChainId, &'a str, i128, u8)>,
    prices: &Prices,
) -> Option<Amount> {
    let mut sum: i128 = 0;
    for (chain, symbol, amount, decimals) in holdings {
        if amount == 0 {
            continue;
        }
        let price = prices.of(chain, symbol)?;
        sum = sum.checked_add(value_of(amount, decimals, price)?)?;
    }
    Some(Amount::new(sum, PRICE_SCALE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prices() -> Prices {
        let mut p = Prices::default();
        // 1 TRX = 0.330325 USDT, 1 BNB = 722.902400 USDT - real quotes.
        p.set_native(ChainId::Tron, 330_325, 100);
        p.set_native(ChainId::Bsc, 722_902_400, 200);
        p
    }

    #[test]
    fn a_portfolio_adds_up() {
        // 8.655008 TRX + 15.88 USDT on TRON, 0.5 BNB + 100 USDT on BNB Chain.
        let t = total(
            [
                (ChainId::Tron, "TRX", 8_655_008i128, 6u8),
                (ChainId::Tron, "USDT", 15_880_000, 6),
                (ChainId::Bsc, "BNB", 500_000_000_000_000_000, 18),
                (ChainId::Bsc, "USDT", 100_000_000_000_000_000_000, 18),
            ],
            &prices(),
        )
        .unwrap();
        // 2.859... + 15.88 + 361.4512 + 100
        assert_eq!(t.decimals, PRICE_SCALE);
        let s = t.to_display_string();
        assert!(s.starts_with("480."), "unexpected total: {s}");
    }

    /// The rule that keeps the figure honest: an asset that is held but cannot
    /// be priced makes the total unavailable rather than smaller.
    #[test]
    fn an_unpriceable_holding_withholds_the_total() {
        let mut p = Prices::default();
        p.set_native(ChainId::Tron, 330_325, 100);
        // No BNB price, and the wallet holds BNB.
        assert!(total([(ChainId::Bsc, "BNB", 1, 18)], &p).is_none());

        // ...but holding none of it is not a gap.
        assert!(total([(ChainId::Bsc, "BNB", 0, 18)], &p).is_some());
        assert_eq!(
            total(
                [
                    (ChainId::Tron, "TRX", 1_000_000, 6),
                    (ChainId::Bsc, "BNB", 0, 18)
                ],
                &p
            )
            .unwrap()
            .raw,
            330_325
        );
    }

    /// A dollar-pegged token is the unit of account, not something to quote
    /// against itself.
    #[test]
    fn usdt_is_one_by_definition() {
        let p = Prices::default();
        assert_eq!(p.of(ChainId::Tron, "USDT"), Some(1_000_000));
        assert_eq!(p.of(ChainId::Bsc, "USDT"), Some(1_000_000));
        assert_eq!(
            p.of(ChainId::Bsc, "CAKE"),
            None,
            "an unknown token has no price"
        );
    }

    /// Precision differs per chain; the same real amount must value the same.
    #[test]
    fn the_same_value_prices_identically_at_either_precision() {
        let p = prices();
        let on_tron = total([(ChainId::Tron, "USDT", 5_000_000i128, 6u8)], &p).unwrap();
        let on_bsc = total([(ChainId::Bsc, "USDT", 5_000_000_000_000_000_000, 18)], &p).unwrap();
        assert_eq!(
            on_tron.raw, on_bsc.raw,
            "5 USDT valued differently by chain"
        );
    }

    /// An absurd balance must not wrap into a plausible number.
    #[test]
    fn overflow_withholds_the_total_rather_than_wrapping() {
        let p = prices();
        assert!(value_of(i128::MAX, 18, 722_902_400).is_none());
        assert!(total([(ChainId::Bsc, "BNB", i128::MAX, 18)], &p).is_none());
    }

    #[test]
    fn an_empty_wallet_is_zero_not_unknown() {
        assert_eq!(total([], &prices()).unwrap().raw, 0);
    }
}
