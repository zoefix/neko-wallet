//! Terminal-cell width helpers.
//!
//! Lives here rather than in the TUI crate because it is inseparable from
//! translation: `"钱包"` occupies four columns, not two, and every table that
//! pads by character count shears the moment a CJK string appears in it.

use unicode_width::UnicodeWidthStr;

/// Display width in terminal cells.
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}
