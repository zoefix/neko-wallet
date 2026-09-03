//! Password strength estimation, ported from the Go reference's
//! `internal/vault/password.go`.
//!
//! The core idea: take `min(charset_entropy, pattern_entropy)`.
//!
//! Charset entropy (`len × log2(alphabet)`) is only accurate for *randomly
//! generated* passwords. For human-chosen ones it wildly overestimates:
//! `MyWallet2026!` scores 85 bits by charset (13 chars × log2(95)) but is really
//! "two common words + a year + a symbol", about 30 bits — a 50,000× gap. Taking
//! the smaller of the two is the only honest option.
//!
//! **The floor is 70 bits here, not the Go reference's 50.** That 50 was
//! explicitly justified by the server-side pepper making offline cracking
//! impossible. We dropped the pepper so the `.db` can be freely copied, so the
//! justification is gone and the bar has to rise.

/// Minimum length. A floor, not a recommendation.
pub const MIN_LEN: usize = 12;
/// Minimum estimated entropy in bits.
pub const MIN_ENTROPY: f64 = 70.0;

/// Passwords rejected outright. Length and character variety do not save these:
/// `Password123!` is 12 characters across four classes and still sits in the
/// first thousand entries of every cracking dictionary.
const COMMON: &[&str] = &[
    "password",
    "password1",
    "password123",
    "123456789",
    "1234567890",
    "qwertyuiop",
    "iloveyou",
    "admin123",
    "welcome123",
    "letmein123",
    "abc123456",
    "passw0rd",
    "tronvault",
    "nekowallet",
    "changeme",
    "secret123",
    "monkey123",
    "dragon123",
    "football",
    "baseball",
];

/// Machine-readable so the TUI can map each to an i18n key instead of showing
/// a hardcoded string.
#[derive(Debug, Clone, PartialEq)]
pub enum Warning {
    TooShort { need: usize, got: usize },
    CommonPassword,
    ContainsCommonFragment(String),
    RepeatedChars(usize),
    SequentialChars(usize),
    ContainsYear,
    WordPlusDigits,
    NeedsMoreVariety,
}

#[derive(Debug, Clone)]
pub struct Strength {
    pub entropy: f64,
    /// 0-4, for a UI meter.
    pub score: u8,
    pub warnings: Vec<Warning>,
}

impl Strength {
    pub fn acceptable(&self) -> bool {
        self.entropy >= MIN_ENTROPY
            && !self
                .warnings
                .iter()
                .any(|w| matches!(w, Warning::TooShort { .. }))
    }
}

pub fn estimate(pw: &str) -> Strength {
    let r: Vec<char> = pw.chars().collect();
    let mut warnings = Vec::new();

    if r.len() < MIN_LEN {
        warnings.push(Warning::TooShort {
            need: MIN_LEN,
            got: r.len(),
        });
    }

    let (charset_bits, classes, has_wide) = charset_entropy(&r);
    let (pattern_bits, mut pattern_warns) = pattern_entropy(&r);
    warnings.append(&mut pattern_warns);

    let mut entropy = charset_bits.min(pattern_bits);

    let lower = pw.to_lowercase();
    if COMMON.contains(&lower.as_str()) {
        entropy = 10.0;
        warnings.push(Warning::CommonPassword);
    } else if let Some(c) = COMMON.iter().find(|c| c.len() >= 6 && lower.contains(**c)) {
        entropy = entropy.min(25.0);
        warnings.push(Warning::ContainsCommonFragment((*c).to_string()));
    }

    let rep = max_repeat(&r);
    if rep >= 4 {
        entropy *= 0.5;
        warnings.push(Warning::RepeatedChars(rep));
    }
    let seq = max_sequence(&r);
    if seq >= 4 {
        entropy *= 0.5;
        warnings.push(Warning::SequentialChars(seq));
    }
    if classes < 2 && !has_wide && r.len() < 20 {
        warnings.push(Warning::NeedsMoreVariety);
    }

    let score = match entropy {
        e if e < 30.0 => 0,
        e if e < 50.0 => 1,
        e if e < MIN_ENTROPY => 2,
        e if e < 100.0 => 3,
        _ => 4,
    };
    Strength {
        entropy,
        score,
        warnings,
    }
}

fn charset_entropy(r: &[char]) -> (f64, usize, bool) {
    let (mut lo, mut up, mut di, mut sy, mut wide) = (false, false, false, false, false);
    for &c in r {
        if c as u32 >= 128 {
            wide = true;
        } else if c.is_lowercase() {
            lo = true;
        } else if c.is_uppercase() {
            up = true;
        } else if c.is_ascii_digit() {
            di = true;
        } else {
            sy = true;
        }
    }
    let mut charset = 0usize;
    let mut classes = 0usize;
    for (on, n) in [(lo, 26), (up, 26), (di, 10), (sy, 33), (wide, 1000)] {
        if on {
            charset += n;
            classes += 1;
        }
    }
    if charset == 0 {
        charset = 1;
    }
    ((r.len() as f64) * (charset as f64).log2(), classes, wide)
}

// Realistic search space per fragment, in bits. Deliberately conservative:
// underestimating is safe, overestimating lets a weak password through.
const BITS_PER_COMMON_WORD: f64 = 11.0; // ~2000 common words
const BITS_PER_LONG_WORD: f64 = 2.0; // per letter beyond 8
const BITS_PER_YEAR: f64 = 7.0; // ~80 plausible years
const BITS_PER_DIGIT: f64 = 3.32; // log2(10)
const BITS_PER_SYMBOL: f64 = 4.5; // people reuse ~20 symbols
const BITS_CAP_PATTERN: f64 = 1.0; // Title Case adds almost nothing

fn pattern_entropy(r: &[char]) -> (f64, Vec<Warning>) {
    let mut bits = 0.0f64;
    let mut warns = Vec::new();
    let (mut words, mut digit_runs) = (0usize, 0usize);
    let mut i = 0usize;

    while i < r.len() {
        let c = r[i];
        if c as u32 >= 128 {
            // Non-ASCII (CJK etc.) has a far larger per-character space.
            bits += 1000f64.log2();
            i += 1;
        } else if c.is_alphabetic() {
            let mut j = i;
            while j < r.len() && (r[j] as u32) < 128 && r[j].is_alphabetic() {
                j += 1;
            }
            let n = j - i;
            words += 1;
            if n <= 2 {
                bits += n as f64 * 26f64.log2();
            } else {
                bits += BITS_PER_COMMON_WORD;
                if n > 8 {
                    bits += (n - 8) as f64 * BITS_PER_LONG_WORD;
                }
                if irregular_case(&r[i..j]) {
                    bits += n as f64 * 0.5;
                } else {
                    bits += BITS_CAP_PATTERN;
                }
            }
            i = j;
        } else if c.is_ascii_digit() {
            let mut j = i;
            while j < r.len() && r[j].is_ascii_digit() {
                j += 1;
            }
            let n = j - i;
            digit_runs += 1;
            if n == 4 && looks_like_year(&r[i..j]) {
                bits += BITS_PER_YEAR;
                warns.push(Warning::ContainsYear);
            } else {
                bits += n as f64 * BITS_PER_DIGIT;
            }
            i = j;
        } else {
            bits += BITS_PER_SYMBOL;
            i += 1;
        }
    }
    if words > 0 && words <= 2 && digit_runs <= 1 {
        warns.push(Warning::WordPlusDigits);
    }
    (bits, warns)
}

fn irregular_case(r: &[char]) -> bool {
    if r.is_empty() {
        return false;
    }
    let (mut all_lower, mut all_upper) = (true, true);
    let mut title = r[0].is_uppercase();
    for (i, &c) in r.iter().enumerate() {
        if c.is_uppercase() {
            all_lower = false;
            if i > 0 {
                title = false;
            }
        }
        if c.is_lowercase() {
            all_upper = false;
        }
    }
    !all_lower && !all_upper && !title
}

fn looks_like_year(r: &[char]) -> bool {
    r.len() == 4 && ((r[0] == '1' && r[1] == '9') || (r[0] == '2' && r[1] == '0'))
}

fn max_repeat(r: &[char]) -> usize {
    let (mut best, mut run) = (0usize, 1usize);
    for w in r.windows(2) {
        if w[0] == w[1] {
            run += 1;
        } else {
            best = best.max(run);
            run = 1;
        }
    }
    best.max(run).min(r.len())
}

fn max_sequence(r: &[char]) -> usize {
    if r.is_empty() {
        return 0;
    }
    let (mut best, mut run) = (1usize, 1usize);
    for w in r.windows(2) {
        let d = w[1] as i32 - w[0] as i32;
        if d == 1 || d == -1 {
            run += 1;
        } else {
            best = best.max(run);
            run = 1;
        }
    }
    best.max(run)
}
