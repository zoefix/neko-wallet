//! There is one chain. These tests exist so a future change cannot quietly
//! reintroduce a network switch, or a badge nobody asked for.

use neko_tui::app::{App, Screen};
use neko_tui::nav::{SettingRow, SETTING_ROWS};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn screen_text(app: &App, w: u16, h: u16) -> String {
    // These assertions are written against the English strings; the app
    // otherwise follows the OS language, which is not the same on every
    // machine that runs the suite.
    neko_i18n::set_locale(neko_i18n::Locale::English);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
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

#[test]
fn mainnet_constants_are_the_only_chain() {
    assert_eq!(neko_tron::DEFAULT_URL, "https://api.trongrid.io");
    assert_eq!(
        neko_tron::USDT_CONTRACT,
        "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t"
    );
    assert_eq!(
        neko_tron::EXPLORER_TX,
        "https://tronscan.org/#/transaction/"
    );
    assert_eq!(
        neko_tron::usdt_address().to_string(),
        neko_tron::USDT_CONTRACT
    );
}

/// Settings must offer no network row at all.
#[test]
fn settings_has_no_network_row() {
    assert!(
        !SETTING_ROWS
            .iter()
            .any(|r| r.label().eq_ignore_ascii_case("network")),
        "a network setting reappeared: {:?}",
        SETTING_ROWS.iter().map(|r| r.label()).collect::<Vec<_>>()
    );
    // The custom node URL is about which server, not which chain.
    assert!(SETTING_ROWS.contains(&SettingRow::NodeUrl));
}

/// No screen may print a network banner. The user asked for one chain and no
/// noise about it.
#[test]
fn no_network_badge_is_rendered() {
    let mut app = App::new(std::path::PathBuf::from("/tmp/neko-x.db"));
    "zoe@example.com".chars().for_each(|c| app.email.push(c));

    let screens = [
        Screen::Wallets(neko_tui::nav::WalletsState::new(Vec::new())),
        Screen::Login {
            email_focused: true,
        },
        Screen::Settings(neko_tui::nav::SettingsState::new()),
    ];
    for s in screens {
        app.screen = s;
        let out = screen_text(&app, 120, 30);
        for needle in ["MAINNET", "REAL FUNDS", "testnet", "nile"] {
            assert!(
                !out.contains(needle),
                "screen still shows {needle:?}:\n{out}"
            );
        }
    }
}

/// A custom node URL is still allowed: it selects a server, not a chain.
#[test]
fn custom_node_url_is_still_configurable() {
    let mut app = App::new(std::path::PathBuf::from("/tmp/neko-x.db"));
    assert!(app
        .setting_value(SettingRow::NodeUrl)
        .contains("api.trongrid.io"));
    app.node_url = Some("https://my-node.example".into());
    assert_eq!(
        app.setting_value(SettingRow::NodeUrl),
        "https://my-node.example"
    );
}
