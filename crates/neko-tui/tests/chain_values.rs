//! What the chain list says each chain is holding.
//!
//! Three answers, and the difference between them is the whole point. A dash
//! means nothing has been fetched; a question mark means something held here
//! cannot be priced; a figure means the figure. Showing zero for the first
//! would claim an account is empty, and showing a total that quietly omitted
//! an unpriceable holding would understate it - which is the direction that
//! makes somebody think they can afford a transfer they cannot.

use neko_core::ChainId as C;
use neko_tui::app::{App, Screen};

fn row(symbol: &str, amount: i128, decimals: u8) -> neko_store::repo::balances::CachedBalance {
    neko_store::repo::balances::CachedBalance {
        symbol: symbol.into(),
        amount,
        decimals,
        updated_at: 1_788_000_000,
    }
}

fn cached(rows: Vec<neko_store::repo::balances::CachedBalance>) -> neko_core::CachedAssets {
    neko_core::CachedAssets {
        updated_at: Some(1_788_000_000),
        rows,
    }
}

fn render(assets: Vec<(C, neko_core::CachedAssets)>, priced: bool) -> String {
    neko_i18n::set_locale(neko_i18n::Locale::English);
    let mut app = App::new(std::path::PathBuf::from("/tmp/neko-chainvals.db"));
    if priced {
        app.prices.set_native(C::Tron, 330_325, 1);
        app.prices.set_native(C::Bsc, 722_902_400, 1);
        app.prices.set_native(C::Avalanche, 7_611_000, 1);
        app.prices.set_native(C::Linea, 2_489_000_000, 1);
    }
    app.screen = Screen::Chains {
        wallet_id: 1,
        name: "w".into(),
        selected: 0,
        assets,
    };
    app.set_viewport(120, 30);
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).unwrap();
    term.draw(|f| neko_tui::render::draw(f, &app)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..buf.area.height)
        .map(|y| {
            let mut line = String::new();
            let mut x = 0u16;
            while x < buf.area.width {
                let sym = buf[(x, y)].symbol();
                line.push_str(sym);
                x += unicode_width::UnicodeWidthStr::width(sym).max(1) as u16;
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The line for one chain, with the panel border stripped, so an assertion
/// cannot pass on another chain's row or trip over the frame.
fn line_for(out: &str, chain: C) -> String {
    let raw = out
        .lines()
        .find(|l| l.contains(chain.label()))
        .unwrap_or_else(|| panic!("no row for {chain:?}\n{out}"));
    raw.trim_matches(|c: char| c == '\u{2502}' || c == '|' || c.is_whitespace())
        .to_string()
}

/// A chain nothing has been fetched for says so, and does not say zero.
#[test]
fn a_chain_never_looked_at_shows_a_dash_rather_than_zero() {
    let out = render(
        vec![(C::Tron, cached(vec![row("TRX", 8_655_008, 6)]))],
        true,
    );
    // TRON was fetched.
    assert!(line_for(&out, C::Tron).contains("2.85"), "{out}");
    // Bitcoin was not, and must not read as an empty account.
    let btc = line_for(&out, C::Bitcoin);
    assert!(btc.trim_end().ends_with('-'), "Bitcoin: {btc}");
    assert!(!btc.contains("0.00"), "a dash became a zero: {btc}");
}

/// A chain that was fetched and really is empty shows zero, which is a
/// different statement from the dash above.
#[test]
fn an_empty_chain_shows_zero_not_a_dash() {
    let out = render(
        vec![(C::Linea, cached(vec![row("ETH", 0, 18), row("USDC", 0, 6)]))],
        true,
    );
    let linea = line_for(&out, C::Linea);
    assert!(linea.contains("0.00"), "Linea: {linea}");
}

/// A holding this wallet cannot price makes the whole chain unknown.
///
/// Four chains cannot price their own coin at all - HYPE and MNT exist in no
/// pool this wallet talks to, and APT and SUI trade only on exchanges. A total
/// that left the coin out would understate the chain.
#[test]
fn an_unpriceable_holding_makes_the_total_a_question_mark() {
    let out = render(
        vec![
            (
                C::HyperEvm,
                cached(vec![row("HYPE", 2_500_000_000_000_000_000, 18)]),
            ),
            (C::Sui, cached(vec![row("SUI", 243_001_381_957, 9)])),
        ],
        true,
    );
    for c in [C::HyperEvm, C::Sui] {
        let l = line_for(&out, c);
        assert!(l.trim_end().ends_with('?'), "{c:?}: {l}");
    }
}

/// A zero holding of an unpriceable coin is skipped, so the rest still totals.
///
/// Mantle's coin has no price anywhere this wallet reaches, but a wallet
/// holding no MNT and some USDC is not unknowable - the unknown part is worth
/// nothing and is left out without changing the answer.
#[test]
fn a_zero_holding_of_an_unpriceable_coin_does_not_poison_the_total() {
    let out = render(
        vec![(
            C::Mantle,
            cached(vec![row("MNT", 0, 18), row("USDC", 12_340_000, 6)]),
        )],
        true,
    );
    let l = line_for(&out, C::Mantle);
    assert!(l.contains("12.34"), "Mantle: {l}");
    assert!(!l.contains('?'), "a zero holding poisoned the total: {l}");
}

/// Before any price has been fetched, every chain says it does not know.
#[test]
fn with_no_prices_at_all_nothing_claims_a_figure() {
    let out = render(
        vec![(
            C::Bsc,
            cached(vec![row("BNB", 500_000_000_000_000_000, 18)]),
        )],
        false,
    );
    let l = line_for(&out, C::Bsc);
    assert!(l.trim_end().ends_with('-'), "BNB Chain: {l}");
}

/// The arithmetic, against figures worked out by hand.
#[test]
fn the_total_is_the_sum_of_what_is_held() {
    // 8.655008 TRX at 0.330325 = 2.859... plus 15.88 USDT = 18.73.
    let out = render(
        vec![(
            C::Tron,
            cached(vec![row("TRX", 8_655_008, 6), row("USDT", 15_880_000, 6)]),
        )],
        true,
    );
    assert!(line_for(&out, C::Tron).contains("18.73"), "{out}");

    // 0.006 AVAX at 7.611 = 0.0457, plus 4.96 USDT = 5.00.
    let out = render(
        vec![(
            C::Avalanche,
            cached(vec![
                row("AVAX", 6_000_000_000_000_000, 18),
                row("USDT", 4_960_000, 6),
            ]),
        )],
        true,
    );
    assert!(line_for(&out, C::Avalanche).contains("5.00"), "{out}");
}
