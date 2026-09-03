//! Fixed-point token amounts.
//!
//! Never `f64`. Its 53-bit mantissa starts losing integers above ~9e15, which
//! for a 6-decimal token is about 9 billion units — roughly $9,000 of USDT.
//! Everything here is `i128` in minimal units; formatting happens only at the
//! last step, on the way to the screen.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Amount {
    /// Minimal units (sun for TRX, 1e-6 USDT for USDT).
    pub raw: i128,
    pub decimals: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AmountError {
    #[error("not a number")]
    NotANumber,
    #[error("amount must be positive")]
    NotPositive,
    #[error("at most {0} decimal places")]
    TooManyDecimals(u8),
    #[error("amount is too large")]
    Overflow,
}

impl Amount {
    pub fn new(raw: i128, decimals: u8) -> Self {
        Self { raw, decimals }
    }

    pub fn zero(decimals: u8) -> Self {
        Self { raw: 0, decimals }
    }

    fn scale(decimals: u8) -> i128 {
        10i128.pow(decimals as u32)
    }

    /// Parse human input such as `"1.5"` or `"1,234.000001"`.
    ///
    /// Digit-by-digit, so no floating point is involved at any point.
    pub fn parse(s: &str, decimals: u8) -> Result<Self, AmountError> {
        let s = s.trim().replace([',', '_'], "");
        if s.is_empty() {
            return Err(AmountError::NotANumber);
        }
        let (int_part, frac_part) = match s.split_once('.') {
            Some((a, b)) => (a, b),
            None => (s.as_str(), ""),
        };
        if int_part.is_empty() && frac_part.is_empty() {
            return Err(AmountError::NotANumber);
        }
        if !int_part.chars().all(|c| c.is_ascii_digit())
            || !frac_part.chars().all(|c| c.is_ascii_digit())
        {
            return Err(AmountError::NotANumber);
        }
        if frac_part.len() > decimals as usize {
            return Err(AmountError::TooManyDecimals(decimals));
        }

        let mut raw: i128 = 0;
        for c in int_part.chars() {
            raw = raw
                .checked_mul(10)
                .and_then(|v| v.checked_add((c as u8 - b'0') as i128))
                .ok_or(AmountError::Overflow)?;
        }
        raw = raw
            .checked_mul(Self::scale(decimals))
            .ok_or(AmountError::Overflow)?;

        let mut frac: i128 = 0;
        for c in frac_part.chars() {
            frac = frac * 10 + (c as u8 - b'0') as i128;
        }
        // Left-align the fraction to the full precision.
        frac *= 10i128.pow((decimals as usize - frac_part.len()) as u32);
        raw = raw.checked_add(frac).ok_or(AmountError::Overflow)?;

        if raw <= 0 {
            return Err(AmountError::NotPositive);
        }
        Ok(Self { raw, decimals })
    }

    /// Full precision, no thousands separators. Used wherever exactness matters
    /// more than readability — the signing confirmation, for instance.
    pub fn to_exact_string(self) -> String {
        let neg = self.raw < 0;
        let v = self.raw.unsigned_abs();
        let scale = Self::scale(self.decimals) as u128;
        let (int, frac) = (v / scale, v % scale);
        let s = format!(
            "{}{}.{:0width$}",
            if neg { "-" } else { "" },
            int,
            frac,
            width = self.decimals as usize
        );
        s
    }

    /// Grouped for display: `1,234.500000`.
    pub fn to_display_string(self) -> String {
        let exact = self.to_exact_string();
        let (int, frac) = exact.split_once('.').unwrap_or((exact.as_str(), ""));
        let (sign, digits) = match int.strip_prefix('-') {
            Some(d) => ("-", d),
            None => ("", int),
        };
        let mut grouped = String::new();
        for (i, c) in digits.chars().enumerate() {
            if i > 0 && (digits.len() - i) % 3 == 0 {
                grouped.push(',');
            }
            grouped.push(c);
        }
        format!("{sign}{grouped}.{frac}")
    }

    /// Grouped, with at most `max_frac` decimal places.
    ///
    /// Eighteen decimals do not fit in a column and nobody reads them, but
    /// simply cutting the string produces the one output a balance must never
    /// have: `0.000000` for an account that is not empty. So a value that is
    /// non-zero yet rounds away is shown as `<0.000001` - smaller than the
    /// smallest figure this column can express, which is true and is not zero.
    pub fn to_display_string_max(self, max_frac: u8) -> String {
        if self.decimals <= max_frac {
            return self.to_display_string();
        }
        let full = self.to_display_string();
        let Some((int, frac)) = full.split_once('.') else {
            return full;
        };
        let cut: String = frac.chars().take(max_frac as usize).collect();

        let all_zero = int.trim_start_matches('-') == "0" && cut.chars().all(|c| c == '0');
        if all_zero && self.raw != 0 {
            let sign = if self.raw < 0 { "-" } else { "" };
            return format!(
                "{sign}<0.{}1",
                "0".repeat(max_frac.saturating_sub(1) as usize)
            );
        }
        format!("{int}.{cut}")
    }

    /// Grouped, capped at `max_frac`, and with trailing zeros removed.
    ///
    /// For a figure you scan rather than one you sign. `0.00000000` and
    /// `250.00000000` say exactly as much as `0` and `250` while costing eight
    /// columns, and an eighteen-decimal chain turns every balance into a row of
    /// zeros wide enough to push the column off the screen. Nothing is lost:
    /// trailing zeros in a fixed-point fraction carry no information, and the
    /// figure that actually gets signed comes from `to_exact_string`, which this
    /// cannot reach.
    ///
    /// The below-the-cap marker keeps its zeros. `<0.00000001` trimmed to `<0.1`
    /// would claim the value is ten million times larger than it is.
    pub fn to_display_string_trim(self, max_frac: u8) -> String {
        let capped = self.to_display_string_max(max_frac);
        if capped.contains('<') {
            return capped;
        }
        let Some((int, frac)) = capped.split_once('.') else {
            return capped;
        };
        let frac = frac.trim_end_matches('0');
        if frac.is_empty() {
            int.to_string()
        } else {
            format!("{int}.{frac}")
        }
    }
}

impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_display_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eighteen decimals have to fit a column without ever reading as zero
    /// when the account is not empty.
    #[test]
    fn capped_display_never_turns_a_balance_into_zero() {
        // Fits: shown in full.
        assert_eq!(
            Amount::new(8_655_008, 6).to_display_string_max(6),
            "8.655008"
        );
        assert_eq!(
            Amount::new(50_000_000_000_000_000, 18).to_display_string_max(6),
            "0.050000"
        );
        assert_eq!(
            Amount::new(1_234_500_000_000_000_000_000, 18).to_display_string_max(6),
            "1,234.500000"
        );
        // Zero really is zero.
        assert_eq!(Amount::new(0, 18).to_display_string_max(6), "0.000000");

        // Non-zero but below what the column can show: never "0.000000".
        let dust = Amount::new(183_016_821, 18); // 0.000000000183016821
        assert_ne!(dust.to_display_string_max(6), "0.000000");
        assert_eq!(dust.to_display_string_max(6), "<0.000001");
        assert_eq!(
            Amount::new(-183_016_821, 18).to_display_string_max(6),
            "-<0.000001"
        );

        // One unit below the threshold and one unit above it.
        assert_eq!(
            Amount::new(999_999_999_999, 18).to_display_string_max(6),
            "<0.000001"
        );
        assert_eq!(
            Amount::new(1_000_000_000_000, 18).to_display_string_max(6),
            "0.000001"
        );
    }

    /// The capped form must stay a prefix of the exact one, so a user
    /// comparing the list against the send screen sees no contradiction.
    #[test]
    fn capped_display_agrees_with_the_exact_value() {
        for raw in [
            0i128,
            1,
            999,
            50_000_000_000_000_000,
            1_234_500_000_000_000_000_000,
        ] {
            let a = Amount::new(raw, 18);
            let capped = a.to_display_string_max(6);
            if capped.starts_with('<') {
                continue;
            }
            let exact = a.to_display_string();
            assert!(
                exact.starts_with(&capped),
                "{capped} is not a prefix of {exact}"
            );
        }
    }

    #[test]
    fn parses_whole_and_fractional_values() {
        assert_eq!(Amount::parse("1", 6).unwrap().raw, 1_000_000);
        assert_eq!(Amount::parse("1.5", 6).unwrap().raw, 1_500_000);
        assert_eq!(Amount::parse("0.000001", 6).unwrap().raw, 1);
        assert_eq!(Amount::parse("1234.000001", 6).unwrap().raw, 1_234_000_001);
        assert_eq!(Amount::parse("  1,234.5  ", 6).unwrap().raw, 1_234_500_000);
        assert_eq!(Amount::parse(".5", 6).unwrap().raw, 500_000);
    }

    #[test]
    fn rejects_malformed_input() {
        for bad in [
            "",
            "  ",
            "abc",
            "1.2.3",
            "-1",
            "1e6",
            "0",
            "0.0",
            "1.2345678",
        ] {
            assert!(Amount::parse(bad, 6).is_err(), "accepted {bad:?}");
        }
        assert_eq!(
            Amount::parse("1.2345678", 6),
            Err(AmountError::TooManyDecimals(6))
        );
        assert_eq!(Amount::parse("0", 6), Err(AmountError::NotPositive));
    }

    /// The reason this module exists: f64 cannot represent these exactly.
    #[test]
    fn survives_amounts_that_break_f64() {
        // 2^53 + 1 minimal units. As f64 this rounds to 2^53.
        let a = Amount::parse("9007199254.740993", 6).unwrap();
        assert_eq!(a.raw, 9_007_199_254_740_993);
        assert_eq!(a.to_exact_string(), "9007199254.740993");
        assert_ne!(
            a.raw as f64 as i128, a.raw,
            "test premise: f64 must lose this"
        );

        // Well past any real supply, still exact.
        let big = Amount::parse("999999999999.999999", 6).unwrap();
        assert_eq!(big.to_exact_string(), "999999999999.999999");
    }

    #[test]
    fn round_trips_through_formatting() {
        for s in ["1.000000", "0.000001", "1234.500000", "9007199254.740993"] {
            let a = Amount::parse(s, 6).unwrap();
            assert_eq!(a.to_exact_string(), s);
            assert_eq!(Amount::parse(&a.to_exact_string(), 6).unwrap(), a);
        }
    }

    #[test]
    fn display_groups_thousands() {
        assert_eq!(
            Amount::new(1_234_567_000_000, 6).to_display_string(),
            "1,234,567.000000"
        );
        assert_eq!(Amount::new(1_000_000, 6).to_display_string(), "1.000000");
        assert_eq!(Amount::new(1, 6).to_display_string(), "0.000001");
        assert_eq!(Amount::new(0, 6).to_display_string(), "0.000000");
    }

    /// The exact TRC20 amount from the transaction vectors.
    #[test]
    fn matches_the_transaction_vector_amount() {
        assert_eq!(Amount::parse("2.5", 6).unwrap().raw, 2_500_000);
        assert_eq!(Amount::parse("1.5", 6).unwrap().raw, 1_500_000);
    }

    /// The assets column is scanned, not signed, and eighteen trailing zeros
    /// are pure noise there. What the trim must never do is change the value.
    #[test]
    fn trimmed_display_drops_only_zeros_that_say_nothing() {
        // Zero is one character, not nineteen.
        assert_eq!(Amount::new(0, 18).to_display_string_trim(8), "0");
        assert_eq!(Amount::new(0, 6).to_display_string_trim(8), "0");

        // A round figure loses its tail, not its magnitude.
        assert_eq!(
            Amount::new(250_000_000_000_000_000_000, 18).to_display_string_trim(8),
            "250"
        );
        assert_eq!(
            Amount::new(1_500_000_000_000_000_000, 18).to_display_string_trim(8),
            "1.5"
        );

        // Thousands separators survive; the grouped zeros are in the integer
        // part and must not be touched.
        assert_eq!(
            Amount::new(1_200_000_000_000_000_000_000, 18).to_display_string_trim(8),
            "1,200"
        );

        // Significant digits are kept to the cap.
        assert_eq!(
            Amount::new(12_345_678_901_234_567_890, 18).to_display_string_trim(8),
            "12.34567890".trim_end_matches('0')
        );
        assert_eq!(
            Amount::new(8_655_008, 6).to_display_string_trim(8),
            "8.655008"
        );

        // Below the cap keeps every zero: `<0.1` would be a lie by a factor of
        // ten million.
        assert_eq!(
            Amount::new(183_016_821, 18).to_display_string_trim(8),
            "<0.00000001"
        );
        assert_eq!(
            Amount::new(-183_016_821, 18).to_display_string_trim(8),
            "-<0.00000001"
        );

        // Negative values keep their sign through the trim.
        assert_eq!(
            Amount::new(-1_500_000_000_000_000_000, 18).to_display_string_trim(8),
            "-1.5"
        );
    }

    /// A trimmed figure has to parse back to the same amount whenever it was
    /// not capped, otherwise the column and the ledger disagree.
    #[test]
    fn trimmed_display_round_trips_when_nothing_was_cut() {
        // `parse` rejects zero by design, so it cannot appear here; the case
        // above pins it instead. Everything else has to come back exactly, and
        // a parse failure is a failure, not a zero.
        for raw in [
            1_500_000_000_000_000_000i128,
            250_000_000_000_000_000_000,
            1_200_000_000_000_000_000_000,
            30_000_000_000_000_000,
            12_345_678_900_000_000_000,
        ] {
            let shown = Amount::new(raw, 18).to_display_string_trim(8);
            assert!(!shown.contains('<'), "{shown} should not have been capped");
            let back = Amount::parse(&shown, 18)
                .unwrap_or_else(|e| panic!("{shown} no longer parses: {e}"));
            assert_eq!(back.raw, raw, "{shown} did not survive the round trip");
        }
    }
}
