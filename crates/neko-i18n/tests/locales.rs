//! Properties the locale files must hold that build.rs cannot check.

use neko_i18n::{all_keys, key_name, t_in, Key, Locale, LOCALES};

fn placeholder_names(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let c: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i + 1 < c.len() {
        if c[i] == '%' && c[i + 1] == '{' {
            if let Some(end) = (i + 2..c.len()).find(|&j| c[j] == '}') {
                out.push(c[i + 2..end].iter().collect::<String>());
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    out.sort();
    out.dedup();
    out
}

/// build.rs guarantees the keys exist. This catches a file that was copied from
/// English and left untranslated, which the key check cannot see.
#[test]
fn no_locale_is_a_copy_of_english() {
    for l in LOCALES {
        if l == Locale::English {
            continue;
        }
        let identical = all_keys()
            .filter(|k| t_in(l, *k) == t_in(Locale::English, *k))
            .count();
        let total = all_keys().count();
        // Proper nouns and symbols legitimately match ("neko-wallet", "#",
        // "Unicode"). A large overlap means the file was copied, not translated.
        assert!(
            identical * 10 < total,
            "{}: {identical} of {total} strings are identical to English - \
             the file looks copied rather than translated",
            l.tag()
        );
    }
}

/// An empty label is invisible, not translated.
#[test]
fn no_value_is_empty() {
    for l in LOCALES {
        for k in all_keys() {
            assert!(
                !t_in(l, k).trim().is_empty(),
                "{}: `{}` is empty",
                l.tag(),
                key_name(k)
            );
        }
    }
}

/// A dropped placeholder makes the substituted value vanish silently, but only
/// for speakers of that one language.
#[test]
fn placeholders_are_preserved_in_every_locale() {
    for k in all_keys() {
        let reference = placeholder_names(t_in(Locale::English, k));
        for l in LOCALES {
            assert_eq!(
                placeholder_names(t_in(l, k)),
                reference,
                "{}: `{}` lost or renamed a placeholder",
                l.tag(),
                key_name(k)
            );
        }
    }
}

/// Substitution is by name, not position: translators reorder clauses, and a
/// positional scheme silently swaps the arguments when they do.
#[test]
fn interpolation_is_by_name_not_position() {
    assert_eq!(
        neko_i18n::interpolate("%{b} then %{a}", &[("a", "FIRST"), ("b", "SECOND")]),
        "SECOND then FIRST"
    );
}

/// An unknown placeholder stays visible rather than disappearing: it is a bug,
/// and hiding it helps nobody.
#[test]
fn an_unsubstituted_placeholder_stays_visible() {
    assert_eq!(
        neko_i18n::interpolate("a %{missing} b", &[]),
        "a %{missing} b"
    );
    assert_eq!(neko_i18n::interpolate("%{a} %{b}", &[("a", "x")]), "x %{b}");
}

#[test]
fn switching_locale_changes_the_output() {
    neko_i18n::set_locale(Locale::English);
    let en = neko_i18n::t(Key::Common_Password);
    neko_i18n::set_locale(Locale::Japanese);
    let ja = neko_i18n::t(Key::Common_Password);
    assert_ne!(en, ja);
    neko_i18n::set_locale(Locale::English);
}

/// Chinese is decided by script or region; getting it backwards shows the wrong
/// characters to every Chinese user.
#[test]
fn system_locale_tags_map_correctly() {
    use Locale::*;
    for (tag, want) in [
        ("en-US", English),
        ("en", English),
        ("ja-JP", Japanese),
        ("ja", Japanese),
        ("zh-Hans-CN", Simplified),
        ("zh-CN", Simplified),
        ("zh-SG", Simplified),
        ("zh", Simplified),
        ("zh-Hant-TW", Traditional),
        ("zh-TW", Traditional),
        ("zh-HK", Traditional),
        ("zh-MO", Traditional),
        ("zh_TW", Traditional),
    ] {
        assert_eq!(Locale::from_system_tag(tag), Some(want), "tag {tag}");
    }
    // Anything else falls back rather than guessing.
    assert_eq!(Locale::from_system_tag("de-DE"), None);
    assert_eq!(Locale::from_system_tag(""), None);
}

/// The picker names each language in its own script, so someone who cannot read
/// the current UI can still find their way back.
#[test]
fn every_locale_names_itself_in_its_own_script() {
    assert_eq!(Locale::English.endonym(), "English");
    assert_eq!(Locale::Simplified.endonym(), "简体中文");
    assert_eq!(Locale::Traditional.endonym(), "繁體中文");
    assert_eq!(Locale::Japanese.endonym(), "日本語");
    for l in LOCALES {
        assert_eq!(Locale::parse(l.tag()), Some(l));
    }
}

/// Simplified and Traditional must not be the same file under two names.
#[test]
fn simplified_and_traditional_actually_differ() {
    let differing = all_keys()
        .filter(|k| t_in(Locale::Simplified, *k) != t_in(Locale::Traditional, *k))
        .count();
    assert!(
        differing > 40,
        "only {differing} strings differ between zh-Hans and zh-Hant"
    );
}

/// No translation may contain a character whose width depends on the terminal's
/// East Asian setting. build.rs enforces this too; this proves it stayed true
/// through generation.
#[test]
fn no_translation_uses_ambiguous_width_characters() {
    for l in LOCALES {
        for k in all_keys() {
            for c in t_in(l, k).chars() {
                let cp = c as u32;
                let ambiguous = matches!(cp,
                    0x2010..=0x2027 | 0x2030..=0x205E | 0x2100..=0x22FF
                    | 0x2460..=0x24FF | 0x2500..=0x257F | 0x2580..=0x25FF
                    | 0x2600..=0x26FF);
                assert!(
                    !ambiguous,
                    "{}: `{}` contains ambiguous-width {c:?}",
                    l.tag(),
                    key_name(k)
                );
            }
        }
    }
}

/// A stored tag must come back as the same locale, forever. If a tag ever
/// changes, every user who picked that language silently reverts to the OS
/// default on their next launch.
#[test]
fn stored_tags_round_trip() {
    for l in neko_i18n::LOCALES {
        assert_eq!(
            Locale::from_tag(l.tag()),
            Some(l),
            "{} does not round-trip",
            l.tag()
        );
    }
    assert_eq!(Locale::from_tag("klingon"), None);
    assert_eq!(Locale::from_tag(""), None);
    // Exact match only: a near-miss must not silently resolve to a language
    // the user never chose.
    assert_eq!(Locale::from_tag("zh"), None);
    assert_eq!(Locale::from_tag("EN"), None);
}

/// The tags are a storage format. Pin the literals so a refactor that renames
/// one has to change this test on purpose.
#[test]
fn tags_are_stable() {
    let tags: Vec<_> = neko_i18n::LOCALES.iter().map(|l| l.tag()).collect();
    assert_eq!(tags, ["en", "zh-Hans", "zh-Hant", "ja"]);
}
