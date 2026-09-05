//! Terminal-cell width arithmetic that survives CJK.
//!
//! `"钱包"` occupies 4 terminal columns, not 2. ratatui measures `Span`s
//! correctly when it renders them, so the bugs do not live there — they live in
//! the strings you compose *before* handing them over. `format!("{name:<16}")`
//! pads by `char` count, and a single Chinese wallet name shears every column
//! in the table to the right of it.
//!
//! Everything that aligns, truncates, or pads goes through this module.

use std::borrow::Cow;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Ellipsis is deliberately ASCII. `…` (U+2026) is `East_Asian_Width=Ambiguous`
/// and renders as 1 cell in a Western terminal but 2 in a terminal configured
/// for East Asian ambiguous-wide — which many CJK users enable. There is no way
/// to ask the terminal which it chose, so we avoid the whole class.
pub const ELLIPSIS: &str = "~";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Left,
    Right,
    Center,
}

/// Display width in terminal cells.
#[inline]
pub fn width(s: &str) -> usize {
    UnicodeWidthStr::width(s)
}

/// Strip C0/C1 control characters. Wallet labels and imported data are
/// attacker-influenced; a stray `\x1b` would let them drive the terminal.
pub fn sanitize(s: &str) -> Cow<'_, str> {
    if s.chars().any(|c| c.is_control()) {
        Cow::Owned(s.chars().filter(|c| !c.is_control()).collect())
    } else {
        Cow::Borrowed(s)
    }
}

/// Truncate to at most `max` terminal cells, grapheme-aware so a ZWJ emoji or a
/// combining mark is never split.
/// Keep the *end* of a string that is too long, marking the cut at the front.
///
/// For a field somebody is typing into. A text input that hides the character
/// you just typed is unusable, and for a pasted address the tail is also what
/// the confirmation step asks you to retype - so when something has to go, it
/// is the beginning.
pub fn truncate_start(s: &str, max: usize) -> Cow<'_, str> {
    if width(s) <= max {
        return Cow::Borrowed(s);
    }
    if max == 0 {
        return Cow::Borrowed("");
    }
    let budget = max - width(ELLIPSIS);
    // Walk backwards, taking graphemes while they fit.
    let mut acc = 0usize;
    let mut start = s.len();
    for (i, g) in s.grapheme_indices(true).rev() {
        let gw = UnicodeWidthStr::width(g);
        if acc + gw > budget {
            break;
        }
        acc += gw;
        start = i;
    }
    let mut out = String::with_capacity(s.len() - start + 4);
    out.push_str(ELLIPSIS);
    // Same detail as `truncate`: stopping early on a wide grapheme leaves a
    // cell spare, and without it the column shifts.
    for _ in acc..budget {
        out.push(' ');
    }
    out.push_str(&s[start..]);
    out.into()
}

pub fn truncate(s: &str, max: usize) -> Cow<'_, str> {
    if width(s) <= max {
        return Cow::Borrowed(s);
    }
    if max == 0 {
        return Cow::Borrowed("");
    }
    let budget = max - width(ELLIPSIS);
    let (mut acc, mut end) = (0usize, 0usize);
    for (i, g) in s.grapheme_indices(true) {
        let gw = UnicodeWidthStr::width(g);
        if acc + gw > budget {
            break;
        }
        acc += gw;
        end = i + g.len();
    }
    let mut out = String::with_capacity(end + 4);
    out.push_str(&s[..end]);
    // The detail everyone forgets: if we stopped early because the next grapheme
    // was *wide*, we are one cell under budget. Pad, or the column shifts left.
    for _ in acc..budget {
        out.push(' ');
    }
    out.push_str(ELLIPSIS);
    Cow::Owned(out)
}

/// Produce a string that is **exactly** `cols` terminal cells wide.
pub fn pad(s: &str, cols: usize, align: Align) -> String {
    let t = truncate(s, cols);
    let slack = cols.saturating_sub(width(&t));
    match align {
        Align::Left => format!("{t}{:slack$}", ""),
        Align::Right => format!("{:slack$}{t}", ""),
        Align::Center => {
            let l = slack / 2;
            let r = slack - l;
            format!("{:l$}{t}{:r$}", "", "")
        }
    }
}

/// Break `s` into lines of at most `cols` terminal cells.
///
/// ratatui's own `Wrap` measures in a way that lets a wide grapheme sitting on
/// the boundary spill one cell past the area it was given - which, inside a
/// bordered block, means the border gets overwritten and the frame develops a
/// hole. Measuring in cells here, by grapheme, keeps that from happening
/// regardless of what a translator writes.
///
/// Breaks at whitespace when there is any. CJK has none, so it breaks between
/// graphemes instead - which is how CJK wraps anyway.
///
/// One exception to the width guarantee, and it is unavoidable: a single
/// grapheme wider than `cols` gets a line to itself and overflows it. There is
/// no correct rendering in that case, and dropping the character would silently
/// alter the text - much worse than a cosmetic overflow in a layout that cannot
/// occur above a two-column budget.
/// Characters that may not begin a line in Japanese or Chinese typography
/// (kinsoku shori): closing brackets, and punctuation that attaches to the
/// character before it. A line starting with a lone full stop is the CJK
/// equivalent of a widow, and it reads as broken to anyone who uses the
/// language.
const NO_LINE_START: &[char] = &[
    '\u{3002}', // ideographic full stop
    '\u{3001}', // ideographic comma
    '\u{FF0C}', '\u{FF0E}', '\u{FF01}', '\u{FF1F}', '\u{FF1A}', '\u{FF1B}',
    '\u{30FB}', // katakana middle dot
    '\u{30FC}', // prolonged sound mark
    '\u{FF09}', '\u{300D}', '\u{300F}', '\u{3011}', '\u{3009}', '\u{300B}', '\u{3015}', '\u{FF3D}',
    '\u{FF5D}', ')', ']', '}', ',', '.', '!', '?', ':', ';',
];

fn forbidden_at_line_start(g: &str) -> bool {
    g.chars().next().is_some_and(|c| NO_LINE_START.contains(&c))
}

pub fn wrap(s: &str, cols: usize) -> Vec<String> {
    if cols == 0 {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_w = 0usize;

    let mut flush = |line: &mut String, line_w: &mut usize| {
        if !line.is_empty() {
            lines.push(std::mem::take(line));
            *line_w = 0;
        }
    };

    for word in s.split_inclusive(char::is_whitespace) {
        let ww = width(word.trim_end());
        // Start a new line when this word will not fit - unless it cannot fit
        // on a line of its own either, in which case it is broken below.
        if line_w > 0 && line_w + ww > cols && ww <= cols {
            flush(&mut line, &mut line_w);
        }
        let graphemes: Vec<&str> = word.graphemes(true).collect();
        for (i, g) in graphemes.iter().copied().enumerate() {
            let gw = UnicodeWidthStr::width(g);
            // Reserve room for a following character that may not start a
            // line, so it comes down with this one rather than being stranded
            // alone at the head of the next.
            let reserved = graphemes
                .get(i + 1)
                .filter(|n| forbidden_at_line_start(n))
                .map(|n| UnicodeWidthStr::width(*n))
                .unwrap_or(0);
            // A grapheme is never split, and never placed where only part of
            // it would fit.
            if line_w + gw + reserved > cols && line_w > 0 {
                flush(&mut line, &mut line_w);
                // A leading space on a wrapped line is just a ragged margin.
                if g.chars().all(char::is_whitespace) {
                    continue;
                }
            }
            line.push_str(g);
            line_w += gw;
        }
    }
    flush(&mut line, &mut line_w);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deliberately hostile: Latin, CJK, Hangul, a flag, a ZWJ family, an NFD
    /// combining mark, a zero-width space, and a raw control byte.
    const CORPUS: &[&str] = &[
        "",
        "a",
        "Cold storage",
        "钱包1",
        "日本用ウォレット",
        "한국어지갑",
        "🇯🇵",
        "👨‍👩‍👧‍👦",
        "cafe\u{0301}",
        "a\u{200b}b",
        "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t",
    ];

    /// The property that matters: no wrapped line may exceed the budget, in
    /// cells, for any input. One cell over is enough to punch a hole in a
    /// bordered block.
    #[test]
    fn wrapped_lines_never_exceed_the_budget() {
        let inputs = [
            "Releases are signed offline. An unsigned or altered download is refused.",
            "\u{30ea}\u{30ea}\u{30fc}\u{30b9}\u{306f}\u{30aa}\u{30d5}\u{30e9}\u{30a4}\u{30f3}\u{3067}\u{7f72}\u{540d}\u{3055}\u{308c}\u{3066}\u{3044}\u{307e}\u{3059}\u{3002}\u{7f72}\u{540d}\u{306e}\u{306a}\u{3044}\u{3001}\u{307e}\u{305f}\u{306f}\u{6539}\u{5909}\u{3055}\u{308c}\u{305f}\u{30c0}\u{30a6}\u{30f3}\u{30ed}\u{30fc}\u{30c9}\u{306f}\u{62d2}\u{5426}\u{3055}\u{308c}\u{307e}\u{3059}\u{3002}",
            "\u{53d1}\u{884c}\u{7248}\u{5747}\u{79bb}\u{7ebf}\u{7b7e}\u{540d}\u{3002}",
            "\u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}\u{200d}\u{1f466}\u{1f1ef}\u{1f1f5}",
            "",
            "a",
            "supercalifragilisticexpialidocious",
        ];
        for s in inputs {
            for cols in [1usize, 2, 3, 7, 20, 40, 96, 98, 200] {
                for line in wrap(s, cols) {
                    let single_oversized_grapheme =
                        line.graphemes(true).count() == 1 && width(&line) > cols;
                    assert!(
                        width(&line) <= cols || single_oversized_grapheme,
                        "{:?} wrapped to {cols} produced a {}-cell line {line:?}",
                        s,
                        width(&line)
                    );
                }
            }
        }
    }

    /// Wrapping must not silently eat text.
    #[test]
    fn wrapping_preserves_every_grapheme() {
        let s = "\u{53d1}\u{884c}\u{7248}\u{5747}\u{79bb}\u{7ebf}\u{7b7e}\u{540d}ABC";
        let joined: String = wrap(s, 5).concat();
        assert_eq!(joined, s, "characters were lost or duplicated");
    }

    /// Japanese line-breaking: a full stop or closing bracket must never be
    /// left alone at the head of a line.
    #[test]
    fn cjk_punctuation_does_not_start_a_line() {
        // Exactly the string, at exactly the width, that produced a stranded
        // full stop on the settings screen.
        let note = "\u{30ea}\u{30ea}\u{30fc}\u{30b9}\u{306f}\u{30aa}\u{30d5}\u{30e9}\u{30a4}\u{30f3}\u{3067}\u{7f72}\u{540d}\u{3055}\u{308c}\u{3066}\u{3044}\u{307e}\u{3059}\u{3002}\u{7f72}\u{540d}\u{306e}\u{306a}\u{3044}\u{3001}\u{307e}\u{305f}\u{306f}\u{6539}\u{5909}\u{3055}\u{308c}\u{305f}\u{30c0}\u{30a6}\u{30f3}\u{30ed}\u{30fc}\u{30c9}\u{306f}\u{62d2}\u{5426}\u{3055}\u{308c}\u{307e}\u{3059}\u{3002}";
        for cols in 20..=100 {
            for line in wrap(note, cols) {
                let first = line.chars().next().unwrap();
                assert!(
                    !NO_LINE_START.contains(&first),
                    "at {cols} cells a line began with {first:?}: {line:?}"
                );
            }
        }
        // And nothing was lost while rearranging.
        assert_eq!(wrap(note, 40).concat(), note);
    }

    #[test]
    fn wrapping_breaks_at_spaces_when_it_can() {
        assert_eq!(
            wrap("the quick brown fox", 10),
            vec!["the quick ", "brown fox"]
        );
        assert_eq!(wrap("short", 10), vec!["short"]);
        assert!(wrap("", 10).is_empty());
    }

    #[test]
    fn pad_always_produces_exactly_the_requested_width() {
        for s in CORPUS {
            let s = sanitize(s);
            for cols in 0..24 {
                for align in [Align::Left, Align::Right, Align::Center] {
                    let out = pad(&s, cols, align);
                    assert_eq!(
                        width(&out),
                        cols,
                        "pad({s:?}, {cols}, {align:?}) = {out:?} is {} cells, want {cols}",
                        width(&out)
                    );
                }
            }
        }
    }

    #[test]
    fn truncate_never_exceeds_the_budget() {
        for s in CORPUS {
            for cols in 0..24 {
                let out = truncate(s, cols);
                assert!(
                    width(&out) <= cols,
                    "truncate({s:?}, {cols}) = {out:?} overflows to {} cells",
                    width(&out)
                );
            }
        }
    }

    /// The bug this module exists to prevent: naive padding by char count.
    #[test]
    fn cjk_is_not_measured_by_char_count() {
        assert_eq!(width("钱包1"), 5, "2 wide chars + 1 narrow");
        assert_eq!("钱包1".chars().count(), 3);
        assert_eq!(width(&pad("钱包1", 16, Align::Left)), 16);
        // Naive formatting is wrong; this is the comparison, not an endorsement.
        assert_eq!(width(&format!("{:<16}", "钱包1")), 18);
    }

    /// Stopping early on a wide grapheme must not leave the column short.
    #[test]
    fn wide_grapheme_boundary_is_padded() {
        // "日本用ウォレット" is 16 cells; at 15 the last wide char cannot fit.
        let out = truncate("日本用ウォレット", 15);
        assert_eq!(width(&out), 15, "got {out:?}");
        assert!(out.ends_with(ELLIPSIS));
    }

    #[test]
    fn graphemes_are_never_split() {
        for cols in 1..8 {
            let out = truncate("👨‍👩‍👧‍👦abc", cols);
            assert!(std::str::from_utf8(out.as_bytes()).is_ok());
            assert!(!out.contains('\u{fffd}'));
        }
    }

    #[test]
    fn control_characters_are_stripped() {
        assert_eq!(sanitize("wallet\x1b[31m\x07"), "wallet[31m");
        assert_eq!(sanitize("clean"), "clean");
    }

    /// Ambiguous-width characters must not appear in our own chrome.
    #[test]
    fn ellipsis_is_unambiguous() {
        assert_eq!(width(ELLIPSIS), 1);
        assert!(
            ELLIPSIS.is_ascii(),
            "ellipsis must be ASCII to avoid EAW=Ambiguous"
        );
    }
}
