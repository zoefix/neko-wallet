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
}

impl fmt::Display for Amount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_display_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
