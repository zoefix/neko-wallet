//! The frame must stay closed.
//!
//! A wide grapheme landing on the right edge of a bordered block overwrites the
//! border character, punching a hole in the frame. The existing width tests
//! cannot see this: the row is still exactly as many cells as the terminal, so
//! everything measures correctly while looking broken.
//!
//! This caught exactly that in the Japanese settings screen, where the
//! sentence explaining that releases are signed was one cell too long.

use neko_i18n::Locale;
use neko_tui::app::{App, Screen};
use neko_tui::nav::{SettingRow, SETTING_ROWS};
use neko_tui::render;

static LOCALE: std::sync::Mutex<()> = std::sync::Mutex::new(());

const EMAIL: &str = "zoe@example.com";
const PW: &str = "correct horse battery staple xyzzy";

/// Both border sets the app can draw.
const VERTICAL: [&str; 2] = ["│", "|"];
const CORNERS: [&str; 10] = ["╭", "╮", "╰", "╯", "┌", "┐", "└", "┘", "+", "|"];

fn unlocked(dir: &std::path::Path) -> App {
    let mut app = App::new(dir.join("neko-wallet.db"));
    EMAIL.chars().for_each(|c| app.email.push(c));
    PW.chars().for_each(|c| app.password.push(c));
    PW.chars().for_each(|c| app.confirm.push(c));
    let ev = app.begin_unlock(true).unwrap().run();
    app.on_app_event(ev);
    app
}

/// Assert the left and right edges of the body block are unbroken.
fn assert_frame_intact(app: &App, w: u16, h: u16, what: &str) {
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
    term.draw(|f| render::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();

    // The last row is the footer, which is deliberately unbordered.
    let body_bottom = h - 2;
    for y in 0..=body_bottom {
        let left = buf[(0, y)].symbol();
        let right = buf[(w - 1, y)].symbol();
        let edge = |s: &str| VERTICAL.contains(&s) || CORNERS.contains(&s) || s == "─" || s == "-";
        assert!(
            edge(left),
            "{what} at {w}x{h}: row {y} has {left:?} at the left edge, not a border"
        );
        assert!(
            edge(right),
            "{what} at {w}x{h}: row {y} has {right:?} at the right edge - \
             something overflowed and overwrote the frame"
        );
    }
}

/// Every screen, every language, at several widths - including ones chosen to
/// land a wide character exactly on the boundary.
#[tokio::test]
async fn no_screen_overflows_its_frame_in_any_language() {
    let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());

    // Odd and even widths both matter: a two-cell glyph straddles the edge in
    // one and not the other.
    let sizes = [
        (80u16, 24u16),
        (81, 24),
        (96, 26),
        (99, 30),
        (100, 26),
        (101, 30),
        (120, 40),
        (137, 28),
    ];

    for loc in neko_i18n::LOCALES {
        app.set_language(loc);

        for (w, h) in sizes {
            app.nav.clear();
            app.screen = app.wallets_screen();
            assert_frame_intact(&app, w, h, &format!("{} wallets", loc.tag()));

            app.open_settings();
            for (i, row) in SETTING_ROWS.iter().enumerate() {
                if let Screen::Settings(st) = &mut app.screen {
                    st.selected = i;
                }
                assert_frame_intact(&app, w, h, &format!("{} settings row {row:?}", loc.tag()));
            }
            app.pop();
        }
    }
    neko_i18n::set_locale(Locale::English);
}

/// The rollback banner is prose of unpredictable length in a bordered block -
/// the same shape as the bug this file exists for.
#[tokio::test]
async fn the_update_banner_does_not_overflow_its_frame() {
    let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    app.startup_warning = Some(
        "neko-wallet 0.2.0 has failed to start before. The previous version is saved at \
         /Users/zoe/.local/bin/.neko-wallet.bak-0.1.0. Run `neko-wallet --rollback` to restore it."
            .into(),
    );
    for loc in neko_i18n::LOCALES {
        app.set_language(loc);
        for (w, h) in [(80u16, 24u16), (99, 30), (100, 26), (120, 40)] {
            assert_frame_intact(&app, w, h, &format!("{} banner", loc.tag()));
        }
    }
    neko_i18n::set_locale(Locale::English);
}

/// The specific regression. A Japanese hint was 99 cells inside a 98-cell
/// block, and the trailing full stop overwrote the right border. The sentence
/// that triggered it has since been removed with the updater, but the
/// mechanism that broke - wrapping translated prose inside a bordered block -
/// is unchanged, so the geometry stays pinned against the row that still
/// carries long Japanese text.
#[tokio::test]
async fn japanese_hint_prose_stays_inside_the_frame() {
    let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    app.set_language(Locale::Japanese);
    app.open_settings();
    let i = SETTING_ROWS
        .iter()
        .position(|r| *r == SettingRow::ApiKey)
        .unwrap();
    if let Screen::Settings(st) = &mut app.screen {
        st.selected = i;
    }
    // 100x26 is the exact size at which the original overflow appeared.
    assert_frame_intact(&app, 100, 26, "the original failure geometry");
    neko_i18n::set_locale(Locale::English);
}
