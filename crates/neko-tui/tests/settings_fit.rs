//! Every settings row is reachable on an ordinary terminal.

use neko_tui::app::{App, Screen};
use neko_tui::nav::{SettingsState, SETTING_ROWS};

fn render(app: &App, w: u16, h: u16) -> String {
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

/// Sixteen rows now. On an 80x24 terminal - the smallest anyone still uses -
/// the row the cursor is on has to be visible, or a setting exists that cannot
/// be seen while it is being edited.
#[test]
fn every_settings_row_is_visible_when_selected() {
    for (i, row) in SETTING_ROWS.iter().enumerate() {
        let mut app = App::new(std::path::PathBuf::from("/tmp/neko-settings.db"));
        let mut st = SettingsState::new();
        st.move_by(i as isize);
        assert_eq!(st.row(), *row, "cursor did not land on row {i}");
        app.screen = Screen::Settings(st);
        app.set_viewport(80, 24);
        let out = render(&app, 80, 24);
        let label = row.label();
        assert!(
            out.contains(label),
            "row {i} ({label}) is off screen at 80x24:\n{out}"
        );
    }
}

/// The window always contains the selection, at every height and every row.
///
/// Checked directly rather than only through a render, because the failure it
/// guards against is a row that can be selected and edited while off screen -
/// which looks like a setting that silently does nothing.
#[test]
fn the_window_always_contains_the_selected_row() {
    let n = SETTING_ROWS.len();
    for height in 8u16..40 {
        for selected in 0..n {
            let (first, shown) = neko_tui::render::settings_window(height, n, selected);
            assert!(shown >= 1, "h={height}: an empty window");
            assert!(first + shown <= n, "h={height}: window runs past the end");
            assert!(
                (first..first + shown).contains(&selected),
                "h={height} sel={selected}: window {first}..{} excludes it",
                first + shown
            );
        }
    }
    // A tall terminal scrolls not at all.
    assert_eq!(neko_tui::render::settings_window(60, n, 0), (0, n));
    // A short one shows a window smaller than the list.
    let (_, shown) = neko_tui::render::settings_window(24, n, 0);
    assert!(shown < n, "24 rows should not fit {n} settings");
}
