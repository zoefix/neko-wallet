//! Which server each chain talks to.
//!
//! Two chains now have a configurable endpoint and they are different settings,
//! which is exactly the shape of mistake that is invisible: a value that is
//! saved, shown back, and then never passed to the thing that makes requests.
//! Every test here follows the setting all the way to the client.

use neko_tui::app::{App, Screen};
use neko_tui::nav::{SettingRow, SettingsState, SETTING_ROWS};

fn app() -> App {
    App::new(std::path::PathBuf::from("/tmp/neko-node-urls.db"))
}

fn edit(app: &mut App, row: SettingRow, value: &str) {
    use crossterm::event::{KeyCode, KeyEvent};
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    let mut st = SettingsState::new();
    st.selected = SETTING_ROWS.iter().position(|r| *r == row).unwrap();
    app.screen = Screen::Settings(st);
    // Enter opens the row, the characters go in, Enter saves - the same path a
    // user takes, so a change to the key handling breaks this too.
    neko_tui::keys::on_key_settings(app, KeyEvent::from(KeyCode::Enter), &tx);
    for c in value.chars() {
        neko_tui::keys::on_key_settings(app, KeyEvent::from(KeyCode::Char(c)), &tx);
    }
    neko_tui::keys::on_key_settings(app, KeyEvent::from(KeyCode::Enter), &tx);
}

/// Both rows exist and are distinct. One field feeding both would look correct
/// on screen and send every Solana request to a TRON node.
#[test]
fn the_two_node_settings_are_separate() {
    assert!(SETTING_ROWS.contains(&SettingRow::NodeUrl));
    assert!(SETTING_ROWS.contains(&SettingRow::SolanaRpc));

    let mut app = app();
    edit(
        &mut app,
        SettingRow::SolanaRpc,
        "https://solana.my-node.example",
    );
    assert_eq!(
        app.solana_rpc.as_deref(),
        Some("https://solana.my-node.example")
    );
    assert!(app.node_url.is_none(), "the TRON node was overwritten");

    edit(
        &mut app,
        SettingRow::NodeUrl,
        "https://tron.my-node.example",
    );
    assert_eq!(
        app.node_url.as_deref(),
        Some("https://tron.my-node.example")
    );
    assert_eq!(
        app.solana_rpc.as_deref(),
        Some("https://solana.my-node.example"),
        "the Solana RPC was overwritten"
    );
}

/// The whole point: the configured URL is what the client actually uses.
#[test]
fn the_configured_rpc_reaches_the_client() {
    let mut app = app();
    // Default first, so the test can tell a working setting from a coincidence.
    assert_eq!(
        app.chain_client(neko_core::ChainId::Solana).endpoint(),
        Some(neko_solana::DEFAULT_RPC)
    );

    edit(
        &mut app,
        SettingRow::SolanaRpc,
        "https://solana.my-node.example",
    );
    assert_eq!(
        app.chain_client(neko_core::ChainId::Solana).endpoint(),
        Some("https://solana.my-node.example"),
        "the setting was saved but never passed to the client"
    );
}

/// Clearing it goes back to the public cluster rather than to nothing.
///
/// Whitespace counts as clearing, not as a malformed URL: somebody who selects
/// the field and deletes what is in it has said what they meant.
#[test]
fn clearing_it_restores_the_default() {
    for typed in ["", "   "] {
        let mut app = app();
        app.solana_rpc = Some("https://solana.my-node.example".into());
        edit(&mut app, SettingRow::SolanaRpc, typed);
        assert!(app.solana_rpc.is_none(), "{typed:?} did not clear it");
    }

    let mut app = app();
    app.solana_rpc = Some("https://solana.my-node.example".into());
    edit(&mut app, SettingRow::SolanaRpc, "");
    assert!(app.solana_rpc.is_none());
    assert_eq!(
        app.chain_client(neko_core::ChainId::Solana).endpoint(),
        Some(neko_solana::DEFAULT_RPC)
    );
    assert!(app
        .setting_value(SettingRow::SolanaRpc)
        .contains("api.mainnet-beta.solana.com"));
}

/// A URL with no scheme cannot be requested, so it is refused at the point it
/// is typed rather than failing later as a network error that names the wrong
/// thing. Crucially, the previous value survives - a rejected edit must not
/// leave the wallet pointing at nothing.
#[test]
fn a_url_without_a_scheme_is_refused_and_changes_nothing() {
    let mut app = app();
    edit(&mut app, SettingRow::SolanaRpc, "https://good.example");

    for bad in [
        "solana.example.com",
        "ftp://solana.example",
        "://x",
        "localhost:8899",
    ] {
        edit(&mut app, SettingRow::SolanaRpc, bad);
        assert_eq!(
            app.solana_rpc.as_deref(),
            Some("https://good.example"),
            "{bad:?} was accepted, or wiped the previous value"
        );
    }

    // The same guard applies to TRON's, which had none before.
    edit(&mut app, SettingRow::NodeUrl, "api.trongrid.io");
    assert!(
        app.node_url.is_none(),
        "a scheme-less TRON node was accepted"
    );
}

/// http is allowed. Somebody running a node on their own machine has no
/// certificate for it, and refusing that would push them back to a public
/// endpoint - which is worse for both privacy and reliability.
#[test]
fn a_local_node_over_http_is_allowed() {
    let mut app = app();
    edit(&mut app, SettingRow::SolanaRpc, "http://127.0.0.1:8899");
    assert_eq!(app.solana_rpc.as_deref(), Some("http://127.0.0.1:8899"));
}
