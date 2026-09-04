//! The assets screen, for every chain.
//!
//! Three chains carry a native coin and USDT; Bitcoin carries one asset. The
//! screen has three states - loading, loaded, and failed - and the two that
//! show placeholders used to spell the rows out rather than ask the chain. On
//! Bitcoin that put a USDT row on screen for the second before the balances
//! arrived: an asset that does not exist there, presented as though it were
//! still being fetched.

use crossterm::event::{KeyCode, KeyEvent};
use neko_tui::app::{App, Screen};

fn addr_for(chain: neko_core::ChainId) -> &'static str {
    match chain {
        neko_core::ChainId::Tron => "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH",
        neko_core::ChainId::Bsc => "0xA41811CF4D41e306310CB82B47258C22b80475cC",
        neko_core::ChainId::Solana => "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM",
        neko_core::ChainId::Bitcoin => "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu",
        // The same address as BNB Chain's: one phrase, one EVM coin type.
        neko_core::ChainId::Ethereum => "0xA41811CF4D41e306310CB82B47258C22b80475cC",
    }
}

fn app_on_assets(chain: neko_core::ChainId) -> App {
    let mut app = App::new(std::path::PathBuf::from("/tmp/neko-assets.db"));
    app.screen = Screen::Assets {
        wallet_id: 1,
        name: "w".into(),
        chain,
        address: addr_for(chain).into(),
        selected: 0,
    };
    app.set_viewport(110, 26);
    app
}

fn render(app: &App) -> String {
    neko_i18n::set_locale(neko_i18n::Locale::English);
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(110, 26)).unwrap();
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

/// Whatever the screen is doing, the assets it names are the chain's.
#[test]
fn no_state_of_the_screen_invents_an_asset() {
    for chain in neko_core::CHAINS {
        let symbols: Vec<&str> = chain.assets().iter().map(|a| a.symbol()).collect();

        // Loading, failed, and loaded. All three used to be written out by
        // hand; two of them were written out wrongly.
        let mut loading = app_on_assets(chain);
        let mut failed = app_on_assets(chain);
        failed.balances_error = Some("node unreachable".into());
        let mut loaded = app_on_assets(chain);
        loaded.balances = Some(
            chain
                .assets()
                .iter()
                .map(|a| (a.symbol().to_string(), a.decimals(), 0i128))
                .collect(),
        );

        for (state, app) in [
            ("loading", &mut loading),
            ("failed", &mut failed),
            ("loaded", &mut loaded),
        ] {
            let out = render(app);
            for sym in &symbols {
                assert!(
                    out.contains(*sym),
                    "{chain:?} {state}: {sym} is missing\n{out}"
                );
            }
            // Nothing this chain does not carry. USDT is the one that got
            // invented, so it is the one worth naming.
            if !symbols.contains(&"USDT") {
                assert!(
                    !out.contains("USDT"),
                    "{chain:?} {state}: a USDT row appeared on a chain with no USDT\n{out}"
                );
            }
        }
    }
}

/// The cursor may only land on an asset that exists.
///
/// This was `% 2` - which also made "up" and "down" the same expression, a
/// second bug that two rows could never reveal.
#[test]
fn the_cursor_stays_on_assets_that_exist() {
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    for chain in neko_core::CHAINS {
        let n = chain.assets().len();
        let mut app = app_on_assets(chain);

        // Down through every row and back to the start.
        for step in 1..=n * 2 {
            neko_tui::keys::on_key_assets(&mut app, KeyEvent::from(KeyCode::Char('j')), &tx);
            let Screen::Assets { selected, .. } = &app.screen else {
                unreachable!()
            };
            assert!(
                *selected < n,
                "{chain:?}: after {step} downs the cursor is on row {selected} of {n}"
            );
            assert_eq!(
                *selected,
                step % n,
                "{chain:?}: down did not advance by one"
            );
        }

        // And up is the other direction, which is only visible with a count
        // that is not two.
        let mut app = app_on_assets(chain);
        neko_tui::keys::on_key_assets(&mut app, KeyEvent::from(KeyCode::Char('k')), &tx);
        let Screen::Assets { selected, .. } = &app.screen else {
            unreachable!()
        };
        assert_eq!(
            *selected,
            n - 1,
            "{chain:?}: up from the first row should wrap to the last"
        );
    }
}

/// Bitcoin has exactly one row. Spelled out separately because it is the case
/// the general tests above exist to protect, and a regression here is what the
/// user actually saw.
#[test]
fn bitcoin_shows_one_asset_and_only_one() {
    let app = app_on_assets(neko_core::ChainId::Bitcoin);
    let out = render(&app);
    assert!(out.contains("BTC"));
    assert!(!out.contains("USDT"), "there is no USDT on Bitcoin:\n{out}");
    assert!(!out.contains("SOL"));
    assert!(!out.contains("BNB"));
    assert_eq!(neko_core::ChainId::Bitcoin.assets().len(), 1);
    assert!(neko_core::ChainId::Bitcoin.usdt().is_none());
}
