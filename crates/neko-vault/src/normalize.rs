//! Credential normalization.
//!
//! NFKC first, then whitespace folding. The Go reference folds whitespace but
//! does NOT normalize Unicode — a password containing `é` typed on macOS (NFD)
//! versus Windows (NFC) is a *different byte string*. For a zero-recovery
//! credential that is a catastrophic, unfixable failure mode: the user will
//! insist the password is right, and they will be right, and the funds are
//! still gone. So we normalize, and pin the behaviour with tests.

use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

/// Collapse all Unicode whitespace runs to a single ASCII space and trim.
fn fold_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut pending_space = false;
    for c in s.chars() {
        if c.is_whitespace() {
            pending_space = !out.is_empty();
        } else {
            if pending_space {
                out.push(' ');
                pending_space = false;
            }
            out.push(c);
        }
    }
    out
}

/// Passwords: NFKC + whitespace fold. **Case is preserved** — passwords are
/// case-sensitive.
pub fn password(s: &str) -> Zeroizing<String> {
    Zeroizing::new(fold_whitespace(&s.nfkc().collect::<String>()))
}

/// Emails: NFKC + whitespace fold + lowercase. Used as a KDF input and as a
/// display value, never for delivery.
pub fn email(s: &str) -> String {
    fold_whitespace(&s.nfkc().collect::<String>()).to_lowercase()
}

/// Mnemonics: NFKC + whitespace fold + lowercase, matching the Go reference.
pub fn mnemonic(s: &str) -> Zeroizing<String> {
    Zeroizing::new(fold_whitespace(&s.nfkc().collect::<String>()).to_lowercase())
}
