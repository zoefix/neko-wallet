//! Address-poisoning defences.
//!
//! The attack: generate a vanity address whose first and last characters match
//! someone you actually pay, send a fraction of a cent so it lands in your
//! history, then wait for you to copy the wrong one. The dust is harmless; the
//! entry in your history is the payload.

use crossterm::event::{KeyCode, KeyEvent};
use neko_tron::{Direction, HistoryEntry, TxStatus};
use neko_tui::app::{App, Screen};
use neko_tui::keys;
use neko_tui::nav::HistoryState;

const MINE: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";
const REAL: &str = "TNYxHL2s6Wjpx86NRwhekYzc27p3oDYrk6";

fn key(c: char) -> KeyEvent {
    KeyEvent::from(KeyCode::Char(c))
}
fn channel() -> tokio::sync::mpsc::UnboundedSender<neko_tui::event::AppEvent> {
    tokio::sync::mpsc::unbounded_channel().0
}

fn entry(counterparty: &str, amount: i128, dir: Direction) -> HistoryEntry {
    HistoryEntry {
        txid: format!("{amount:064x}"),
        block_ts: 1_756_000_000_000 + amount as i64,
        symbol: "TRX".into(),
        decimals: 6,
        amount,
        direction: dir,
        counterparty: counterparty.into(),
        status: TxStatus::Success,
    }
}

fn history_app(entries: Vec<HistoryEntry>) -> App {
    let mut app = App::new(std::path::PathBuf::from("/tmp/neko-poison.db"));
    let mut h = HistoryState::new(neko_core::ChainId::Tron, MINE.into());
    h.entries = Some(entries);
    app.screen = Screen::History(h);
    app
}

fn state(app: &App) -> &HistoryState {
    match &app.screen {
        Screen::History(h) => h,
        _ => panic!(),
    }
}

fn render(app: &App, w: u16, h: u16) -> String {
    // These assertions are written against the English strings; the app
    // otherwise follows the OS language, which is not the same on every
    // machine that runs the suite.
    neko_i18n::set_locale(neko_i18n::Locale::English);
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
    term.draw(|f| neko_tui::render::draw(f, app)).unwrap();
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

/// Dust must not clutter the list by default; that clutter IS the attack.
#[tokio::test]
async fn dust_is_hidden_by_default() {
    let app = history_app(vec![
        entry(REAL, 1_500_000, Direction::In),
        entry("TTkmE3aaaaaaaaaaaaaaaaaaaaaaaa1bNi", 5, Direction::In),
        entry("TWsdxAaaaaaaaaaaaaaaaaaaaaaaaaSNNi", 1, Direction::In),
        entry("THQk9xaaaaaaaaaaaaaaaaaaaaaaaabbNi", 9, Direction::In),
    ]);
    assert_eq!(state(&app).len(), 1, "dust was not filtered out");
    assert_eq!(state(&app).dust_count(), 3);
}

/// But it must be inspectable: hiding evidence of an attack is not the goal.
#[tokio::test]
async fn dust_can_be_shown_and_is_marked() {
    let mut app = history_app(vec![
        entry(REAL, 1_500_000, Direction::In),
        entry("TTkmE3aaaaaaaaaaaaaaaaaaaaaaaa1bNi", 5, Direction::In),
    ]);
    keys::on_key_history(&mut app, key('D'), &channel());
    assert_eq!(state(&app).len(), 2);
    let out = render(&app, 140, 30);
    assert!(out.contains("DUST?"), "shown dust is not marked:\n{out}");
}

/// Toggling must not leave the cursor pointing past the end of the list.
#[tokio::test]
async fn toggling_dust_keeps_the_cursor_in_range() {
    let mut app = history_app(vec![
        entry(REAL, 1_500_000, Direction::In),
        entry("TTkmE3aaaaaaaaaaaaaaaaaaaaaaaa1bNi", 5, Direction::In),
        entry("TWsdxAaaaaaaaaaaaaaaaaaaaaaaaaSNNi", 1, Direction::In),
    ]);
    keys::on_key_history(&mut app, key('D'), &channel()); // show
    for _ in 0..5 {
        keys::on_key_history(&mut app, key('j'), &channel());
    }
    assert_eq!(state(&app).selected, 2);
    keys::on_key_history(&mut app, key('D'), &channel()); // hide again
    assert!(state(&app).selected < state(&app).len().max(1));
    assert!(state(&app).current().is_some(), "cursor left dangling");
}

/// The abbreviation that makes two different addresses look identical must not
/// appear in the history list.
#[tokio::test]
async fn counterparties_are_not_abbreviated_in_history() {
    let app = history_app(vec![entry(REAL, 1_500_000, Direction::In)]);
    let out = render(&app, 140, 30);
    assert!(
        out.contains(REAL),
        "counterparty is not shown in full:\n{out}"
    );
    assert!(
        !out.contains("TNYxHL..Yrk6"),
        "head..tail abbreviation is back:\n{out}"
    );
}

/// The money-saving check: a destination that imitates a known address must be
/// called out before anything is signed.
#[tokio::test]
async fn a_lookalike_destination_is_flagged() {
    // Same first four and last four as REAL, different middle. A real
    // attacker brute-forces a valid one; the check works on the text either way.
    let poison = "TNYxaaaaaaaaaaaaaaaaaaaaaaaaaaYrk6";
    assert_eq!(poison.len(), REAL.len());
    let mut st = neko_tui::send::SendState::new(
        1,
        "w".into(),
        neko_core::ChainAddress::parse(neko_core::ChainId::Tron, MINE).unwrap(),
        neko_core::Asset::Trx,
        "TRX".into(),
    );
    st.known = vec![REAL.to_string()];

    // The real address raises nothing.
    REAL.chars().for_each(|c| st.to.push(c));
    assert!(
        st.lookalike_warning().is_none(),
        "the genuine address was flagged"
    );

    st.to.clear();
    poison.chars().for_each(|c| st.to.push(c));
    assert_eq!(
        st.lookalike_warning().as_deref(),
        Some(REAL),
        "a crafted lookalike slipped through"
    );
}

/// An address unlike anything in the history is not flagged: crying wolf would
/// train the user to ignore the warning.
#[tokio::test]
async fn unrelated_destinations_are_not_flagged() {
    let mut st = neko_tui::send::SendState::new(
        1,
        "w".into(),
        neko_core::ChainAddress::parse(neko_core::ChainId::Tron, MINE).unwrap(),
        neko_core::Asset::Trx,
        "TRX".into(),
    );
    st.known = vec![REAL.to_string()];
    "TXYZopYRdj2D9XRtbG411XZZ3kM5VkAeBf"
        .chars()
        .for_each(|c| st.to.push(c));
    assert!(st.lookalike_warning().is_none());
}
