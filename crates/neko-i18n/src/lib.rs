//! Translations.
//!
//! Hand-rolled rather than a library, for one reason: with `rust-i18n` a typo'd
//! key like `t!("wallets.titel")` renders the literal string `wallets.titel` to
//! somebody holding money, and key parity is checked by an external tool you
//! have to remember to run. Here the keys are an enum generated from the locale
//! files, so a missing or misspelled key does not compile.
//!
//! Switching languages is a single relaxed atomic store. There is no reload, no
//! lock, and no allocation on the lookup path.

use std::sync::atomic::{AtomicUsize, Ordering};

include!(concat!(env!("OUT_DIR"), "/generated.rs"));

pub mod width;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Locale {
    English = 0,
    Simplified = 1,
    Traditional = 2,
    Japanese = 3,
}

pub const LOCALES: [Locale; 4] = [
    Locale::English,
    Locale::Simplified,
    Locale::Traditional,
    Locale::Japanese,
];

impl Locale {
    /// Stored in settings and matched against the OS locale.
    pub fn tag(self) -> &'static str {
        match self {
            Locale::English => "en",
            Locale::Simplified => "zh-Hans",
            Locale::Traditional => "zh-Hant",
            Locale::Japanese => "ja",
        }
    }

    /// Parse a tag we wrote ourselves, as an exact round-trip of [`tag`].
    ///
    /// Deliberately *not* [`from_system_tag`]: that one is a heuristic over
    /// whatever string the OS hands us, and applying it to our own stored
    /// value would let a future locale (say a separate `zh-Hant-HK`) be
    /// silently resolved to a different one than the user picked.
    pub fn from_tag(tag: &str) -> Option<Self> {
        LOCALES.into_iter().find(|l| l.tag() == tag)
    }

    /// Shown in the language picker, in its own language: someone who cannot
    /// read the current UI still has to be able to find their way out.
    pub fn endonym(self) -> &'static str {
        match self {
            Locale::English => "English",
            Locale::Simplified => "简体中文",
            Locale::Traditional => "繁體中文",
            Locale::Japanese => "日本語",
        }
    }

    pub fn parse(tag: &str) -> Option<Self> {
        LOCALES.into_iter().find(|l| l.tag() == tag)
    }

    /// Best match for a BCP-47 tag from the operating system.
    ///
    /// Script subtags decide Chinese, not region alone, but region is the only
    /// signal many systems give: `zh-TW`, `zh-HK` and `zh-MO` are traditional,
    /// everything else Chinese falls back to simplified.
    pub fn from_system_tag(tag: &str) -> Option<Self> {
        let lower = tag.to_ascii_lowercase().replace('_', "-");
        if lower.starts_with("ja") {
            return Some(Locale::Japanese);
        }
        if lower.starts_with("zh") {
            let traditional = lower.contains("hant")
                || lower.contains("-tw")
                || lower.contains("-hk")
                || lower.contains("-mo");
            return Some(if traditional {
                Locale::Traditional
            } else {
                Locale::Simplified
            });
        }
        if lower.starts_with("en") {
            return Some(Locale::English);
        }
        None
    }

    /// What the OS asks for, or English.
    pub fn detect() -> Self {
        sys_locale::get_locale()
            .and_then(|t| Locale::from_system_tag(&t))
            .unwrap_or(Locale::English)
    }
}

static CURRENT: AtomicUsize = AtomicUsize::new(0);

pub fn set_locale(l: Locale) {
    CURRENT.store(l as usize, Ordering::Relaxed);
}

pub fn locale() -> Locale {
    LOCALES[CURRENT.load(Ordering::Relaxed).min(LOCALES.len() - 1)]
}

/// Look up a string in the active language.
///
/// Cannot fail: the key set is generated from the locale files, and build.rs
/// refuses to compile a locale that is missing any of them.
pub fn t(key: Key) -> &'static str {
    TABLE[CURRENT.load(Ordering::Relaxed).min(LOCALE_COUNT - 1)][key as usize]
}

/// Look up a string in a specific language, regardless of the active one.
pub fn t_in(l: Locale, key: Key) -> &'static str {
    TABLE[l as usize][key as usize]
}

/// Substitute `%{name}` placeholders.
///
/// Named, never positional: translators reorder clauses, and a positional
/// scheme silently swaps the arguments when they do.
pub fn interpolate(template: &str, args: &[(&str, &str)]) -> String {
    if args.is_empty() || !template.contains("%{") {
        return template.to_string();
    }
    let mut out = String::with_capacity(template.len() + 16);
    let chars: Vec<char> = template.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() && chars[i + 1] == '{' {
            if let Some(end) = (i + 2..chars.len()).find(|&j| chars[j] == '}') {
                let name: String = chars[i + 2..end].iter().collect();
                match args.iter().find(|(k, _)| *k == name) {
                    Some((_, v)) => out.push_str(v),
                    // An unsubstituted placeholder is left visible rather than
                    // silently dropped: it is a bug, and hiding it helps nobody.
                    None => {
                        out.push_str("%{");
                        out.push_str(&name);
                        out.push('}');
                    }
                }
                i = end + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

pub fn tf(key: Key, args: &[(&str, &str)]) -> String {
    interpolate(t(key), args)
}

/// The key's dotted name, for diagnostics and tests.
pub fn key_name(key: Key) -> &'static str {
    KEY_NAMES[key as usize]
}

pub fn all_keys() -> impl Iterator<Item = Key> {
    ALL_KEYS.into_iter()
}

/// `t!(Send_Total)` or `t!(Common_Of, i = 3, n = 9)`.
#[macro_export]
macro_rules! t {
    ($key:ident) => {
        $crate::t($crate::Key::$key)
    };
    ($key:ident, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::tf(
            $crate::Key::$key,
            &[$((stringify!($name), &::std::string::ToString::to_string(&$value))),+],
        )
    };
}
