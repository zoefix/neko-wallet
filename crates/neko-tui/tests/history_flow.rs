//! The transaction history screen.

#[path = "sink.rs"]
mod sink;

use crossterm::event::{KeyCode, KeyEvent};
use neko_tron::{Direction, HistoryEntry, TxStatus};
use neko_tui::app::{App, Screen};
use neko_tui::keys;
use neko_tui::nav::HistoryState;

fn key(c: char) -> KeyEvent {
    KeyEvent::from(KeyCode::Char(c))
}
fn code(c: KeyCode) -> KeyEvent {
    KeyEvent::from(c)
}
fn channel() -> tokio::sync::mpsc::UnboundedSender<neko_tui::event::AppEvent> {
    tokio::sync::mpsc::unbounded_channel().0
}

const ADDR: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";
const OTHER: &str = "TNYxHL2s6Wjpx86NRwhekYzc27p3oDYrk6";

fn entry(n: i64, dir: Direction, status: TxStatus) -> HistoryEntry {
    HistoryEntry {
        txid: format!("{n:064x}"),
        block_ts: 1_756_000_000_000 + n * 1000,
        symbol: "USDT".into(),
        decimals: 6,
        amount: 1_500_000 * n as i128,
        direction: dir,
        counterparty: OTHER.into(),
        status,
    }
}

fn app_with_history(entries: Vec<HistoryEntry>) -> App {
    let mut app = App::new(std::path::PathBuf::from("/tmp/neko-hist.db"));
    let mut h = HistoryState::new(neko_core::ChainId::Tron, ADDR.into());
    h.entries = Some(entries);
    app.screen = Screen::History(h);
    app
}

fn state(app: &App) -> &HistoryState {
    match &app.screen {
        Screen::History(h) => h,
        _ => panic!("not on the history screen"),
    }
}

#[tokio::test]
async fn navigation_is_clamped_at_both_ends() {
    let mut app = app_with_history(
        (1..=5)
            .map(|n| entry(n, Direction::In, TxStatus::Success))
            .collect(),
    );

    keys::on_key_history(&mut app, key('k'), &channel());
    assert_eq!(state(&app).selected, 0, "moved above the first row");

    for _ in 0..10 {
        keys::on_key_history(&mut app, key('j'), &channel());
    }
    assert_eq!(state(&app).selected, 4, "moved past the last row");
}

/// The window must follow the cursor, or a long history scrolls off screen.
#[tokio::test]
async fn the_visible_window_follows_the_cursor() {
    let mut app = app_with_history(
        (1..=40)
            .map(|n| entry(n, Direction::In, TxStatus::Success))
            .collect(),
    );
    if let Screen::History(h) = &mut app.screen {
        h.page = 10;
    }
    for _ in 0..15 {
        keys::on_key_history(&mut app, key('j'), &channel());
    }
    let h = state(&app);
    assert_eq!(h.selected, 15);
    assert!(h.offset <= h.selected, "cursor is above the window");
    assert!(h.selected < h.offset + h.page, "cursor is below the window");
}

#[tokio::test]
async fn paging_moves_a_whole_screen() {
    let mut app = app_with_history(
        (1..=40)
            .map(|n| entry(n, Direction::In, TxStatus::Success))
            .collect(),
    );
    app.set_viewport(120, 34);
    let page = state(&app).page;
    assert_eq!(
        page,
        neko_tui::nav::DEFAULT_PAGE,
        "a 34-row terminal should page by 20"
    );

    keys::on_key_history(&mut app, code(KeyCode::PageDown), &channel());
    assert_eq!(
        state(&app).selected,
        page,
        "PageDown must move exactly one screenful"
    );
    keys::on_key_history(&mut app, code(KeyCode::PageUp), &channel());
    assert_eq!(state(&app).selected, 0);
}

#[tokio::test]
async fn an_empty_history_does_not_panic() {
    let mut app = app_with_history(Vec::new());
    for k in ['j', 'k', 'y', 'R'] {
        keys::on_key_history(&mut app, key(k), &channel());
    }
    keys::on_key_history(&mut app, code(KeyCode::PageDown), &channel());
    assert_eq!(state(&app).selected, 0);
}

#[tokio::test]
async fn copying_yanks_the_selected_txid() {
    neko_i18n::set_locale(neko_i18n::Locale::English);
    let mut app = app_with_history(
        (1..=3)
            .map(|n| entry(n, Direction::Out, TxStatus::Success))
            .collect(),
    );
    keys::on_key_history(&mut app, key('j'), &channel());
    // Tests must not overwrite the developer's clipboard.
    let dir = tempfile::tempdir().unwrap();
    let sink = dir.path().join("clip.txt");
    app.clipboard = sink::recorder(&sink);
    keys::on_key_history(&mut app, key('y'), &channel());
    assert!(app.toast.is_some(), "no feedback after copying a txid");
    assert!(
        !std::fs::read_to_string(&sink)
            .unwrap_or_default()
            .is_empty(),
        "the txid never reached the clipboard backend"
    );
}

/// Opening an explorer link must copy it, not launch a browser: a wallet that
/// opens URLs tells whoever handles the click which addresses you own.
#[tokio::test]
async fn explorer_link_is_copied_not_opened() {
    neko_i18n::set_locale(neko_i18n::Locale::English);
    let mut app = app_with_history(vec![entry(1, Direction::In, TxStatus::Success)]);
    let dir = tempfile::tempdir().unwrap();
    let sink = dir.path().join("clip.txt");
    app.clipboard = sink::recorder(&sink);
    keys::on_key_history(&mut app, code(KeyCode::Enter), &channel());
    let t = &app.toast.as_ref().expect("no feedback").text;
    assert!(
        std::fs::read_to_string(&sink)
            .unwrap_or_default()
            .contains("http"),
        "the explorer URL never reached the clipboard backend"
    );
    assert!(
        t.contains("Copy") || t.contains("Copied"),
        "unexpected: {t}"
    );
}

/// A failed transfer still cost a fee, so it must be visible and marked.
#[tokio::test]
async fn failed_and_pending_rows_are_rendered_distinctly() {
    let app = app_with_history(vec![
        entry(1, Direction::In, TxStatus::Success),
        entry(2, Direction::Out, TxStatus::Failed),
        entry(3, Direction::Out, TxStatus::Pending),
    ]);
    let out = render(&app, 120, 30);
    assert!(out.contains("OK"), "missing success marker:\n{out}");
    assert!(
        out.contains("FAILED"),
        "a failed transfer was hidden:\n{out}"
    );
    assert!(out.contains("PENDING"), "missing pending marker:\n{out}");
    assert!(
        out.contains("in") && out.contains("out"),
        "direction not shown"
    );
}

/// Amounts must never lose precision OR be truncated on the way to the screen.
/// A clipped amount is worse than none: it silently misinforms.
///
/// Trailing zeros are the one thing that may go: `100,000,000,000` and
/// `100,000,000,000.000000` are the same number, and the second costs seven
/// columns to say so. The guard against that becoming an excuse to drop real
/// digits is the round trip below - whatever the screen shows has to parse back
/// to the exact amount that was rendered.
#[tokio::test]
async fn large_amounts_render_in_full() {
    for (raw, want) in [
        (9_007_199_254_740_993i128, "9,007,199,254.740993"), // 2^53 + 1
        (100_000_000_000_000_000, "100,000,000,000"),        // whole USDT supply
        // The smallest amount that is NOT filtered as poisoning dust.
        (neko_tron::history::dust_threshold(6), "0.001"),
    ] {
        // What the screen shows must be the amount, not merely something that
        // looks like it.
        let parsed = neko_core::Amount::parse(want, 6).expect("the rendered figure must parse");
        assert_eq!(
            parsed.raw, raw,
            "expectation {want} is not {raw} - the test itself is wrong"
        );

        let mut e = entry(1, Direction::In, TxStatus::Success);
        e.amount = raw;
        let app = app_with_history(vec![e]);
        // Must hold at the narrowest supported terminal, not just a wide one.
        for w in [80u16, 120, 200] {
            let out = render(&app, w, 30);
            assert!(
                out.contains(want),
                "amount {raw} was truncated or malformed at {w} columns (wanted {want}):\n{out}"
            );
        }
    }
}

#[tokio::test]
async fn errors_are_shown_with_a_retry_hint() {
    let mut app = app_with_history(Vec::new());
    if let Screen::History(h) = &mut app.screen {
        h.entries = None;
        h.error = Some("node unreachable".into());
    }
    let out = render(&app, 120, 30);
    assert!(
        out.contains("node unreachable"),
        "error not surfaced:\n{out}"
    );
    assert!(out.contains("press R"), "no retry hint:\n{out}");
}

#[tokio::test]
async fn loading_state_is_distinct_from_empty() {
    let mut app = app_with_history(Vec::new());
    if let Screen::History(h) = &mut app.screen {
        h.entries = None;
    }
    assert!(render(&app, 120, 30).contains("loading"));

    if let Screen::History(h) = &mut app.screen {
        h.entries = Some(Vec::new());
    }
    let out = render(&app, 120, 30);
    assert!(
        out.contains("No transactions"),
        "empty history looks like loading:\n{out}"
    );
}

#[tokio::test]
async fn escape_returns_to_the_previous_screen() {
    let mut app = app_with_history(vec![entry(1, Direction::In, TxStatus::Success)]);
    app.nav.push(Screen::Wallets(
        neko_tui::nav::WalletsState::new(Vec::new()),
    ));
    keys::on_key_history(&mut app, code(KeyCode::Esc), &channel());
    assert!(matches!(app.screen, Screen::Wallets(_)));
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

/// Page size follows the terminal, so a tall window is not wasted on ten rows.
#[tokio::test]
async fn page_size_follows_the_terminal_height() {
    use neko_tui::nav::{history_page_for, DEFAULT_PAGE, MIN_PAGE};

    // The user's 118x34 window should show a full screenful.
    assert_eq!(
        history_page_for(34),
        DEFAULT_PAGE,
        "a 34-row terminal should fill up"
    );
    assert_eq!(history_page_for(60), DEFAULT_PAGE, "capped, not unbounded");
    // The 80x24 minimum still gets a usable list.
    assert!(history_page_for(24) >= MIN_PAGE);
    assert!(
        history_page_for(24) < DEFAULT_PAGE,
        "a short window must show fewer"
    );
    // Absurdly short must not underflow or return zero.
    assert_eq!(history_page_for(0), MIN_PAGE);
    assert_eq!(history_page_for(1), MIN_PAGE);
}

/// Resizing must re-page immediately, or PageDown jumps by the wrong amount.
#[tokio::test]
async fn resizing_repages_the_open_list() {
    let mut app = app_with_history(
        (1..=40)
            .map(|n| entry(n, Direction::In, TxStatus::Success))
            .collect(),
    );
    app.set_viewport(120, 34);
    assert_eq!(state(&app).page, neko_tui::nav::DEFAULT_PAGE);

    keys::on_key_history(&mut app, code(KeyCode::PageDown), &channel());
    assert_eq!(state(&app).selected, neko_tui::nav::DEFAULT_PAGE);

    // Shrink: the cursor must stay inside the smaller window.
    app.set_viewport(80, 24);
    let h = state(&app);
    assert!(h.page < neko_tui::nav::DEFAULT_PAGE);
    assert!(
        h.selected < h.offset + h.page,
        "cursor fell outside the resized window"
    );
}

/// Every row the page claims to show must actually fit on screen.
#[tokio::test]
async fn all_paged_rows_are_visible_on_screen() {
    let mut app = app_with_history(
        (1..=40)
            .map(|n| entry(n, Direction::In, TxStatus::Success))
            .collect(),
    );
    for h in [24u16, 34, 50] {
        app.set_viewport(120, h);
        let page = state(&app).page;
        let out = render(&app, 120, h);
        let rows = out
            .lines()
            .filter(|l| l.contains(" in ") && l.contains("USDT"))
            .count();
        assert_eq!(
            rows, page,
            "at height {h}: claimed {page} rows, rendered {rows}\n{out}"
        );
    }
}

/// Columns must never run together.
///
/// `out` is three characters and `in` is two, so a direction column sized to
/// the shorter value leaves a gap after `in` but none after `out` — which is
/// how `outUSDT` reached the screen. Check the values that actually collide.
#[tokio::test]
async fn columns_never_run_together() {
    let mut entries = Vec::new();
    for (i, (dir, sym)) in [
        (Direction::Out, "TRX"),
        (Direction::Out, "USDT"),
        (Direction::In, "TRX"),
        (Direction::In, "USDT"),
    ]
    .iter()
    .enumerate()
    {
        let mut e = entry(i as i64 + 1, *dir, TxStatus::Success);
        e.symbol = (*sym).to_string();
        entries.push(e);
    }
    let mut app = app_with_history(entries);
    app.set_viewport(120, 34);
    let out = render(&app, 120, 34);

    for glued in ["outTRX", "outUSDT", "inTRX", "inUSDT", "AmountStatus"] {
        assert!(
            !out.contains(glued),
            "columns collided as {glued:?}:\n{out}"
        );
    }
    for spaced in ["out TRX", "out USDT", "in  TRX", "in  USDT"] {
        assert!(out.contains(spaced), "expected {spaced:?} in:\n{out}");
    }
}

/// The header must sit directly above the values it labels. They are drawn by
/// separate code paths, so drift is only caught by comparing them.
#[tokio::test]
async fn header_columns_line_up_with_the_rows() {
    let mut app = app_with_history(vec![entry(1, Direction::Out, TxStatus::Success)]);
    app.set_viewport(120, 34);
    let out = render(&app, 120, 34);
    let lines: Vec<&str> = out.lines().collect();

    let header = lines
        .iter()
        .find(|l| l.contains("When") && l.contains("Asset"))
        .unwrap();
    let row = lines
        .iter()
        .find(|l| l.contains("USDT") && l.contains("OK"))
        .unwrap();

    assert_eq!(
        header.find("Asset"),
        row.find("USDT"),
        "the Asset header does not sit above the symbol\nheader: {header}\nrow:    {row}"
    );
    // "Amount" is right-aligned, so compare its right edge to the value's. The
    // figure is derived rather than spelled out, so that changing how balances
    // are formatted moves the expectation with it instead of failing here.
    let shown =
        neko_core::Amount::new(1_500_000, 6).to_display_string_trim(neko_tui::chain::BALANCE_FRAC);
    let header_amount_end = header.find("Amount").unwrap() + "Amount".len();
    let row_amount_end = row
        .find(&shown)
        .unwrap_or_else(|| panic!("no amount {shown:?} in row: {row}"))
        + shown.len();
    assert_eq!(
        header_amount_end, row_amount_end,
        "the Amount column is not right-aligned with its header\nheader: {header}\nrow:    {row}"
    );
    assert_eq!(
        header.find("Status"),
        row.find("OK"),
        "the Status header does not sit above the status\nheader: {header}\nrow:    {row}"
    );
}
