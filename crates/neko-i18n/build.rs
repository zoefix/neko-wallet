//! Turns the locale files into a `Key` enum and a static lookup table.
//!
//! Everything is checked here, at compile time, because the alternative is
//! shipping a missing translation to somebody holding money. A typo'd or absent
//! key must not be a runtime surprise that only speakers of one language ever
//! see — it must stop the build.
//!
//! Four things are enforced against the English file as the reference:
//!   1. every locale has exactly the same key set (no missing, no extra)
//!   2. every value uses exactly the same `%{placeholders}`
//!   3. no value contains an East-Asian-ambiguous-width character
//!   4. values marked `"@todo"` warn instead of failing, and fall back to English

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

const LOCALES: &[&str] = &["en", "zh-Hans", "zh-Hant", "ja"];
const REFERENCE: &str = "en";
const TODO: &str = "@todo";

fn main() {
    println!("cargo::rerun-if-changed=locales");

    let tables: BTreeMap<&str, BTreeMap<String, String>> = LOCALES
        .iter()
        .map(|loc| {
            let path = format!("locales/{loc}.toml");
            let raw = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("cannot read {path}: {e}"));
            let parsed: toml::Value =
                toml::from_str(&raw).unwrap_or_else(|e| panic!("{path} is not valid TOML: {e}"));
            (*loc, flatten(&parsed))
        })
        .collect();

    let reference = &tables[REFERENCE];
    let mut errors: Vec<String> = Vec::new();

    for (loc, table) in &tables {
        for key in reference.keys() {
            if !table.contains_key(key) {
                errors.push(format!("{loc}.toml: MISSING key `{key}`"));
            }
        }
        for key in table.keys() {
            if !reference.contains_key(key) {
                errors.push(format!(
                    "{loc}.toml: EXTRA key `{key}` (not in {REFERENCE}.toml)"
                ));
            }
        }
        for (key, value) in table {
            if value == TODO {
                println!("cargo::warning={loc}.toml: `{key}` is still untranslated");
                continue;
            }
            if let Some(reference_value) = reference.get(key) {
                let (a, b) = (placeholders(value), placeholders(reference_value));
                if a != b {
                    errors.push(format!(
                        "{loc}.toml: `{key}` uses placeholders {a:?} but {REFERENCE} uses {b:?}"
                    ));
                }
            }
            if let Some(c) = first_ambiguous_width(value) {
                errors.push(format!(
                    "{loc}.toml: `{key}` contains the ambiguous-width character {c:?}. \
                     It renders as one column in a Western terminal and two in a CJK one, \
                     which shears every table on the screen. Use an ASCII equivalent."
                ));
            }
        }
    }

    if !errors.is_empty() {
        for e in &errors {
            println!("cargo::error={e}");
        }
        panic!("{} locale problem(s); see the errors above", errors.len());
    }

    emit(&tables, reference);
}

/// `[section] key = "v"` becomes `section.key`.
fn flatten(value: &toml::Value) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    walk(value, String::new(), &mut out);
    out
}

fn walk(value: &toml::Value, prefix: String, out: &mut BTreeMap<String, String>) {
    match value {
        toml::Value::Table(t) => {
            for (k, v) in t {
                let next = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                walk(v, next, out);
            }
        }
        toml::Value::String(s) => {
            out.insert(prefix, s.clone());
        }
        other => panic!("locale values must be strings, found {other:?} at `{prefix}`"),
    }
}

/// The `%{name}` placeholders in a string, sorted so order does not matter —
/// translators legitimately reorder clauses.
fn placeholders(s: &str) -> Vec<String> {
    let mut found = Vec::new();
    let bytes: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == '%' && bytes[i + 1] == '{' {
            let mut j = i + 2;
            let mut name = String::new();
            while j < bytes.len() && bytes[j] != '}' {
                name.push(bytes[j]);
                j += 1;
            }
            if j < bytes.len() {
                found.push(name);
                i = j + 1;
                continue;
            }
        }
        i += 1;
    }
    found.sort();
    found.dedup();
    found
}

/// Characters whose width depends on the terminal's East Asian setting.
///
/// CJK text itself is unambiguously wide and therefore fine; the problem is
/// symbols and box-drawing, which render as one column for some users and two
/// for others. There is no way to ask the terminal which it chose.
fn first_ambiguous_width(s: &str) -> Option<char> {
    s.chars().find(|c| {
        matches!(*c as u32,
            0x2010..=0x2027   // dashes, quotes, ellipsis
            | 0x2030..=0x205E
            | 0x2100..=0x22FF // letterlike, arrows, maths
            | 0x2460..=0x24FF // enclosed alphanumerics
            | 0x2500..=0x257F // box drawing
            | 0x2580..=0x259F // block elements
            | 0x25A0..=0x25FF // geometric shapes
            | 0x2600..=0x26FF // misc symbols
            | 0x00A1 | 0x00A4 | 0x00A7 | 0x00A8 | 0x00AA | 0x00AD | 0x00AE
            | 0x00B0..=0x00B4 | 0x00B6..=0x00BA | 0x00BC..=0x00BF | 0x00C6 | 0x00D0
        )
    })
}

fn emit(tables: &BTreeMap<&str, BTreeMap<String, String>>, reference: &BTreeMap<String, String>) {
    let keys: Vec<&String> = reference.keys().collect();
    let mut out = String::new();

    out.push_str("// Generated by build.rs from locales/*.toml. Do not edit.\n\n");
    out.push_str("/// Every translatable string. A key that does not exist is a compile error,\n");
    out.push_str("/// which is the entire point of generating this.\n");
    out.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n");
    out.push_str("#[allow(non_camel_case_types)]\n");
    out.push_str("pub enum Key {\n");
    for k in &keys {
        let _ = writeln!(out, "    /// `{}`", reference[*k].replace('\n', " "));
        let _ = writeln!(out, "    {},", variant(k));
    }
    out.push_str("}\n\n");

    let _ = writeln!(out, "pub const KEY_COUNT: usize = {};", keys.len());
    let _ = writeln!(out, "pub const LOCALE_COUNT: usize = {};", LOCALES.len());

    out.push_str("\npub(crate) static TABLE: [[&str; KEY_COUNT]; LOCALE_COUNT] = [\n");
    for loc in LOCALES {
        let table = &tables[loc];
        let _ = writeln!(out, "    // {loc}");
        out.push_str("    [\n");
        for k in &keys {
            // "@todo" falls back to the reference so partial translations can
            // land without breaking the build for everyone.
            let value = match table.get(*k) {
                Some(v) if v != TODO => v,
                _ => &reference[*k],
            };
            let _ = writeln!(out, "        {:?},", value);
        }
        out.push_str("    ],\n");
    }
    out.push_str("];\n\n");

    // A real array rather than a transmute over the discriminant: the enum's
    // repr is not ours to assume, and this is checked by the compiler.
    out.push_str("/// Every key, in declaration order.\n");
    out.push_str("pub static ALL_KEYS: [Key; KEY_COUNT] = [\n");
    for k in &keys {
        let _ = writeln!(out, "    Key::{},", variant(k));
    }
    out.push_str("];\n\n");

    out.push_str("pub(crate) static KEY_NAMES: [&str; KEY_COUNT] = [\n");
    for k in &keys {
        let _ = writeln!(out, "    {:?},", k);
    }
    out.push_str("];\n");

    let dir = std::env::var("OUT_DIR").expect("OUT_DIR");
    std::fs::write(Path::new(&dir).join("generated.rs"), out).expect("write generated.rs");
}

/// `send.confirm_prompt` becomes `Send_ConfirmPrompt`.
fn variant(key: &str) -> String {
    key.split('.')
        .map(|part| {
            part.split('_')
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                        None => String::new(),
                    }
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("_")
}
