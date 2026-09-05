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
