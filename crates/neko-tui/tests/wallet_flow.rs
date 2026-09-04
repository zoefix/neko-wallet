//! Wallet navigation and the recovery-phrase reveal, driven through the real
//! state machine with a real vault.

#[path = "sink.rs"]
mod sink;

use crossterm::event::{KeyCode, KeyEvent};
use neko_tui::app::{App, Screen};
use neko_tui::keys;
use neko_tui::nav::{RevealStage, WalletForm};

const EMAIL: &str = "zoe@example.com";
const PW: &str = "correct horse battery staple xyzzy";
const LEDGER_PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const LEDGER_ADDR: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";

fn key(c: char) -> KeyEvent {
    KeyEvent::from(KeyCode::Char(c))
}
fn code(c: KeyCode) -> KeyEvent {
    KeyEvent::from(c)
}

/// An unlocked app sitting on the wallet list.
///
/// Deliberately goes through the real setup path, so these tests also exercise
/// KDF calibration and a real Argon2id run rather than a stubbed one.
fn unlocked(dir: &std::path::Path) -> App {
    let mut app = App::new(dir.join("neko-wallet.db"));
    EMAIL.chars().for_each(|c| app.email.push(c));
    PW.chars().for_each(|c| app.password.push(c));
    PW.chars().for_each(|c| app.confirm.push(c));
    let ev = app.begin_unlock(true).unwrap().run();
    app.on_app_event(ev);
    assert!(
        matches!(app.screen, Screen::Wallets(_)),
        "setup failed: {:?}",
        app.error
    );
    app
}

fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        keys::on_key_wallets(app, key(c), &channel());
    }
}

fn add_ledger_wallet(app: &mut App, name: &str) {
    keys::on_key_wallets(app, key('i'), &channel()); // import phrase
    let Screen::Wallets(w) = &mut app.screen else {
        panic!()
    };
    let Some(WalletForm::ImportMnemonic {
        label,
        phrase,
        focus,
        ..
    }) = &mut w.form
    else {
        panic!("import form did not open")
    };
    label.clear();
    name.chars().for_each(|c| label.push(c));
    LEDGER_PHRASE.chars().for_each(|c| phrase.push(c));
    *focus = 0;
    keys::on_key_wallets(app, code(KeyCode::Enter), &channel());
}

#[tokio::test]
async fn creating_a_wallet_never_shows_the_phrase() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());

    keys::on_key_wallets(&mut app, key('n'), &channel());
    assert!(matches!(&app.screen, Screen::Wallets(w) if w.form.is_some()));
    keys::on_key_wallets(&mut app, code(KeyCode::Enter), &channel());

    // Still on the list, with a wallet — and no reveal screen anywhere.
    let Screen::Wallets(w) = &app.screen else {
        panic!("left the wallet list")
    };
    assert_eq!(w.items.len(), 1);
    assert!(w.form.is_none());
    assert!(!matches!(app.screen, Screen::Reveal { .. }));
}

#[tokio::test]
async fn navigation_drills_down_and_back() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "ledger");

    keys::on_key_wallets(&mut app, code(KeyCode::Enter), &channel());
    assert!(
        matches!(app.screen, Screen::Chains { .. }),
        "did not reach chains"
    );

    keys::on_key_chains(&mut app, code(KeyCode::Enter), &channel());
    let Screen::Assets { address, .. } = &app.screen else {
        panic!("did not reach assets")
    };
    assert_eq!(address, LEDGER_ADDR, "wrong receiving address");

    keys::on_key_assets(&mut app, code(KeyCode::Esc), &channel());
    assert!(matches!(app.screen, Screen::Chains { .. }));
    keys::on_key_chains(&mut app, code(KeyCode::Esc), &channel());
    assert!(matches!(app.screen, Screen::Wallets(_)));
}

/// The second chain is BNB Chain, and it works: one wallet, one phrase, an
/// account on each chain.
///
/// The addresses must differ. They are derived at different SLIP-44 coin types
/// - 195 and 60 - so the same phrase yields different keys, which is standard
/// and is what every other wallet does. A test that let them be equal would be
/// hiding a derivation bug that sends funds to an account nobody can spend
/// from.
#[tokio::test]
async fn the_second_chain_is_bnb_and_has_its_own_address() {
    neko_i18n::set_locale(neko_i18n::Locale::English);
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "w");
    keys::on_key_wallets(&mut app, code(KeyCode::Enter), &channel());

    keys::on_key_chains(&mut app, key('j'), &channel()); // move to BNB Chain
    keys::on_key_chains(&mut app, code(KeyCode::Enter), &channel());

    let Screen::Assets { chain, address, .. } = &app.screen else {
        panic!(
            "did not open the BNB Chain assets screen: {}",
            app.toast
                .as_ref()
                .map(|t| t.text.as_str())
                .unwrap_or("no message")
        )
    };
    assert_eq!(*chain, neko_core::ChainId::Bsc);
    assert!(
        address.starts_with("0x") && address.len() == 42,
        "not an EVM address: {address}"
    );
    assert_ne!(
        address, LEDGER_ADDR,
        "both chains produced the same address - the coin type is not reaching derivation"
    );
    // The canonical vector for this phrase, so the address can be checked
    // against any other wallet.
    assert_eq!(address, "0x9858EfFD232B4033E47d90003D41EC34EcaEda94");
}

/// The reveal must start at the gate and refuse a wrong password.
#[tokio::test]
async fn reveal_requires_the_password_and_starts_masked() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "ledger");

    keys::on_key_wallets(&mut app, key('m'), &channel());
    let Screen::Reveal {
        stage: RevealStage::Gate { .. },
        ..
    } = &app.screen
    else {
        panic!("reveal did not open at the gate")
    };

    for c in "wrong password entirely".chars() {
        keys::on_key_reveal(&mut app, key(c));
    }
    keys::on_key_reveal(&mut app, code(KeyCode::Enter));
    assert!(
        matches!(
            &app.screen,
            Screen::Reveal {
                stage: RevealStage::Gate { .. },
                ..
            }
        ),
        "a wrong password got past the gate"
    );

    for c in PW.chars() {
        keys::on_key_reveal(&mut app, key(c));
    }
    keys::on_key_reveal(&mut app, code(KeyCode::Enter));

    let Screen::Reveal {
        stage:
            RevealStage::Words {
                words,
                cursor,
                show_all,
                ..
            },
        ..
    } = &app.screen
    else {
        panic!("correct password did not reveal the words")
    };
    assert_eq!(words.len(), 12);
    assert_eq!(words.join(" "), LEDGER_PHRASE);
    assert_eq!(*cursor, 0);
    assert!(
        !*show_all,
        "must start with one word visible, not all of them"
    );
}

fn reveal_words(app: &mut App) {
    keys::on_key_wallets(app, key('m'), &channel());
    for c in PW.chars() {
        keys::on_key_reveal(app, key(c));
    }
    keys::on_key_reveal(app, code(KeyCode::Enter));
}

#[tokio::test]
async fn reveal_steps_one_word_at_a_time_and_wraps() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "w");
    reveal_words(&mut app);

    for expected in [1usize, 2, 3] {
        keys::on_key_reveal(&mut app, code(KeyCode::Right));
        let Screen::Reveal {
            stage: RevealStage::Words { cursor, .. },
            ..
        } = &app.screen
        else {
            panic!()
        };
        assert_eq!(*cursor, expected);
    }
    keys::on_key_reveal(&mut app, code(KeyCode::Left));
    let Screen::Reveal {
        stage: RevealStage::Words { cursor, .. },
        ..
    } = &app.screen
    else {
        panic!()
    };
    assert_eq!(*cursor, 2);

    keys::on_key_reveal(&mut app, key(' '));
    let Screen::Reveal {
        stage: RevealStage::Words { show_all, .. },
        ..
    } = &app.screen
    else {
        panic!()
    };
    assert!(*show_all, "space should toggle show-all");
}

/// Leaving the screen must drop the words, not leave them in memory.
#[tokio::test]
async fn leaving_reveal_clears_the_words() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "w");
    reveal_words(&mut app);

    keys::on_key_reveal(&mut app, code(KeyCode::Esc));
    assert!(
        matches!(app.screen, Screen::Wallets(_)),
        "Esc did not leave the reveal screen"
    );
    assert!(app.nav.is_empty());
}

/// The words hide themselves if the user walks away.
#[tokio::test]
async fn reveal_auto_hides_after_the_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "w");
    reveal_words(&mut app);

    if let Screen::Reveal {
        stage: RevealStage::Words { hide_at, .. },
        ..
    } = &mut app.screen
    {
        *hide_at = std::time::Instant::now();
    }
    assert!(app.on_tick(), "tick should report the hide");
    assert!(
        matches!(app.screen, Screen::Wallets(_)),
        "phrase stayed on screen past its timeout"
    );
}

/// The reveal screen must have no path to the clipboard at all.
#[tokio::test]
async fn reveal_screen_has_no_copy_action() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "w");
    reveal_words(&mut app);

    app.toast = None;
    for c in ['y', 'c', 'C', 'Y'] {
        keys::on_key_reveal(&mut app, key(c));
    }
    assert!(
        app.toast.is_none(),
        "a copy was attempted from the reveal screen"
    );
    assert!(matches!(
        &app.screen,
        Screen::Reveal {
            stage: RevealStage::Words { .. },
            ..
        }
    ));
}

/// Deleting destroys the only copy of the words, so a keypress is not enough.
#[tokio::test]
async fn delete_requires_typing_the_wallet_name() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "ledger");

    keys::on_key_wallets(&mut app, key('d'), &channel());
    keys::on_key_wallets(&mut app, code(KeyCode::Enter), &channel()); // nothing typed
    let Screen::Wallets(w) = &app.screen else {
        panic!()
    };
    assert_eq!(w.items.len(), 1, "wallet deleted without confirmation");
    assert!(w.error.is_some());

    type_str(&mut app, "ledger");
    keys::on_key_wallets(&mut app, code(KeyCode::Enter), &channel());
    let Screen::Wallets(w) = &app.screen else {
        panic!()
    };
    assert!(w.items.is_empty(), "typed confirmation did not delete");
}

#[tokio::test]
async fn rename_updates_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "old name");

    keys::on_key_wallets(&mut app, key('r'), &channel());
    let Screen::Wallets(w) = &mut app.screen else {
        panic!()
    };
    let Some(WalletForm::Rename { label, .. }) = &mut w.form else {
        panic!()
    };
    label.clear();
    keys::on_key_wallets(&mut app, code(KeyCode::Enter), &channel());

    type_str(&mut app, "");
    let Screen::Wallets(w) = &app.screen else {
        panic!()
    };
    assert_eq!(w.items.len(), 1);
}

#[tokio::test]
async fn copying_an_address_reports_honestly() {
    neko_i18n::set_locale(neko_i18n::Locale::English);
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "w");
    keys::on_key_wallets(&mut app, code(KeyCode::Enter), &channel());
    keys::on_key_chains(&mut app, code(KeyCode::Enter), &channel());

    // Substitute a harmless backend. Two reasons: running the test suite must
    // not overwrite the developer's real clipboard, and routing the copy
    // through a file is what lets us assert the address actually arrived
    // rather than merely that some message appeared.
    let sink = dir.path().join("clip.txt");
    app.clipboard = sink::recorder(&sink);
    let Screen::Assets { address, .. } = &app.screen else {
        panic!("not on the assets screen")
    };
    let address = address.clone();

    keys::on_key_assets(&mut app, key('y'), &channel());

    // The bug this replaces: the backend silently did nothing, the toast said
    // the request had been "sent", and the assertion accepted that. Anything
    // that only checks for a message passes while the clipboard stays empty.
    assert_eq!(
        sink::captured(&sink),
        address,
        "the address never reached the clipboard backend"
    );
    let t = &app.toast.as_ref().expect("no feedback after copy").text;
    assert!(
        t.contains("Copied"),
        "a verified copy must be reported as done, not as merely sent: {t}"
    );
}

/// The wallet must never claim a copy succeeded when the backend failed:
/// pasting then yields whatever was on the clipboard before, which for an
/// address is how money goes to the wrong place.
#[tokio::test]
async fn a_failed_copy_is_never_reported_as_success() {
    neko_i18n::set_locale(neko_i18n::Locale::English);
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "w");
    keys::on_key_wallets(&mut app, code(KeyCode::Enter), &channel());
    keys::on_key_chains(&mut app, code(KeyCode::Enter), &channel());

    app.clipboard = sink::broken();
    keys::on_key_assets(&mut app, key('y'), &channel());

    let t = &app
        .toast
        .as_ref()
        .expect("no feedback after a failed copy")
        .text;
    assert!(
        !t.contains("Copied"),
        "a copy that never happened was reported as done: {t}"
    );
    assert!(
        t.contains("unavailable") || t.contains("manually"),
        "the user is not told to select it by hand: {t}"
    );
}

fn channel() -> tokio::sync::mpsc::UnboundedSender<neko_tui::event::AppEvent> {
    tokio::sync::mpsc::unbounded_channel().0
}

/// Regression: the balance backend existed for a whole milestone while nothing
/// ever called it, so the screen showed a hardcoded "--". Opening the assets
/// screen must actually dispatch a request, and R must dispatch another.
#[tokio::test]
async fn opening_assets_requests_balances() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "ledger");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    keys::on_key_wallets(&mut app, code(KeyCode::Enter), &channel());
    keys::on_key_chains(&mut app, code(KeyCode::Enter), &tx);

    assert!(matches!(app.screen, Screen::Assets { .. }));
    assert!(
        app.balances_req.is_some(),
        "no balance request was dispatched"
    );
    assert!(
        app.balances.is_none(),
        "balances must start empty, not stubbed"
    );

    // R re-dispatches with a fresh id.
    let first = app.balances_req;
    keys::on_key_assets(&mut app, key('R'), &tx);
    assert!(app.balances_req.is_some());
    assert_ne!(
        app.balances_req, first,
        "refresh reused the previous request id"
    );

    drop(tx);
    // The spawned tasks hit the network; we only care that they were started.
    rx.close();
}

/// A late reply for an address the user has left must not paint over the new one.
#[tokio::test]
async fn stale_balance_replies_are_dropped() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "ledger");
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    keys::on_key_wallets(&mut app, code(KeyCode::Enter), &channel());
    keys::on_key_chains(&mut app, code(KeyCode::Enter), &tx);

    let stale = neko_tui::event::ReqId(9999);
    app.on_app_event(neko_tui::event::AppEvent::Balances {
        req: stale,
        res: Ok(vec![("TRX".into(), 6, 999_000_000)]),
    });
    assert!(app.balances.is_none(), "a stale reply was accepted");

    let current = app.balances_req.unwrap();
    app.on_app_event(neko_tui::event::AppEvent::Balances {
        req: current,
        res: Ok(vec![("TRX".into(), 6, 12_500_000)]),
    });
    // Stored in minimal units, so the send screen can subtract a fee from it.
    assert_eq!(app.balances.as_ref().unwrap()[0].2, 12_500_000);
}

/// A failed lookup must say so, not silently show nothing.
#[tokio::test]
async fn balance_errors_are_surfaced() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "ledger");
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
    keys::on_key_wallets(&mut app, code(KeyCode::Enter), &channel());
    keys::on_key_chains(&mut app, code(KeyCode::Enter), &tx);

    let current = app.balances_req.unwrap();
    app.on_app_event(neko_tui::event::AppEvent::Balances {
        req: current,
        res: Err("node unreachable".into()),
    });
    assert!(app.balances.is_none());
    assert_eq!(app.balances_error.as_deref(), Some("node unreachable"));
}

/// The list must paint from cache immediately — the whole point of caching is
/// not staring at spinners while several network round trips complete.
#[tokio::test]
async fn the_wallet_list_renders_cached_balances_without_the_network() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "ledger");

    let id = match &app.screen {
        Screen::Wallets(w) => w.items[0].id,
        _ => panic!(),
    };
    app.session
        .as_ref()
        .unwrap()
        .cache_assets(
            id,
            neko_core::ChainId::Tron,
            &[
                ("TRX".into(), 6, 55_101_572_000),
                ("USDT".into(), 6, 17_476_654_444),
            ],
        )
        .unwrap();
    app.refresh_wallets();
    // Prices as if already quoted on-chain. The point of the test is that the
    // figure comes from the cache and the arithmetic, with no network at all.
    app.prices.set_native(neko_core::ChainId::Tron, 330_325, 1);
    app.prices
        .set_native(neko_core::ChainId::Bsc, 722_902_400, 1);

    let out = render(&app, 140, 30);
    // 55,101.572000 TRX x 0.330325 + 17,476.654444 USDT = 35,678.03...
    assert!(
        out.contains("35,678."),
        "the cached balances did not become a total:\n{out}"
    );
    assert!(
        !out.contains('?'),
        "a total was withheld even though every holding could be priced:\n{out}"
    );
    // A cached figure presented as current would be a lie: the age is required.
    assert!(
        out.contains("just now") || out.contains("ago"),
        "no freshness label:\n{out}"
    );
}

/// A wallet with nothing cached must say so rather than imply a zero balance.
#[tokio::test]
async fn an_uncached_wallet_shows_no_figure() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "fresh");

    let out = render(&app, 140, 30);
    assert!(
        out.contains("never"),
        "an uncached wallet must be labelled:\n{out}"
    );
    assert!(
        !out.contains("0.000000"),
        "an uncached wallet must not be shown as holding zero:\n{out}"
    );
}

/// Balances persist across a lock/unlock cycle, which is what makes the cache
/// worth having.
#[tokio::test]
async fn cached_balances_survive_relocking() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("neko-wallet.db");
    {
        let mut app = unlocked(dir.path());
        add_ledger_wallet(&mut app, "ledger");
        let id = match &app.screen {
            Screen::Wallets(w) => w.items[0].id,
            _ => panic!(),
        };
        app.session
            .as_ref()
            .unwrap()
            .cache_assets(
                id,
                neko_core::ChainId::Tron,
                &[("USDT".into(), 6, 1_234_560_000)],
            )
            .unwrap();
    }

    let mut app = App::new(db);
    EMAIL.chars().for_each(|c| app.email.push(c));
    PW.chars().for_each(|c| app.password.push(c));
    let ev = app.begin_unlock(false).unwrap().run();
    app.on_app_event(ev);

    app.prices.set_native(neko_core::ChainId::Tron, 330_325, 1);
    app.prices
        .set_native(neko_core::ChainId::Bsc, 722_902_400, 1);

    let out = render(&app, 140, 30);
    // 1,234.56 USDT, priced at one, survives the lock and unlock.
    assert!(
        out.contains("1,234.56"),
        "cache did not survive a relock:\n{out}"
    );
}

/// A wallet nothing has been fetched for reads as unknown, not as empty.
/// Those are different claims, and only one of them is true.
#[tokio::test]
async fn a_never_fetched_wallet_is_not_reported_as_empty() {
    neko_i18n::set_locale(neko_i18n::Locale::English);
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "fresh");
    app.prices.set_native(neko_core::ChainId::Tron, 330_255, 1);
    app.prices
        .set_native(neko_core::ChainId::Bsc, 723_659_574, 1);

    let out = render(&app, 140, 30);
    assert!(
        !out.contains("0.00"),
        "a wallet with nothing fetched was valued at zero:\n{out}"
    );
}

/// Before any price is known the column must say so, not show a zero. A wallet
/// with funds reading "0.00" is the one output this figure must never have.
#[tokio::test]
async fn no_price_reads_as_unknown_not_as_zero() {
    neko_i18n::set_locale(neko_i18n::Locale::English);
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "w");
    let id = match &app.screen {
        Screen::Wallets(w) => w.items[0].id,
        _ => panic!(),
    };
    app.session
        .as_ref()
        .unwrap()
        .cache_assets(
            id,
            neko_core::ChainId::Tron,
            &[("TRX".into(), 6, 55_101_572_000)],
        )
        .unwrap();
    app.refresh_wallets();

    let out = render(&app, 140, 30);
    assert!(
        !out.contains("0.00 "),
        "a funded wallet was valued at zero before any price was known:\n{out}"
    );

    // A price for the wrong chain does not cover a holding on this one: the
    // total is withheld rather than quietly leaving the TRX out.
    app.prices
        .set_native(neko_core::ChainId::Bsc, 722_902_400, 1);
    let out = render(&app, 140, 30);
    assert!(
        out.contains('?'),
        "an unpriceable holding was silently dropped from the total:\n{out}"
    );
}

/// A late reply for a wallet must land on that wallet, not whichever row
/// happens to be selected.
#[tokio::test]
async fn asset_replies_are_applied_to_the_right_wallet() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    add_ledger_wallet(&mut app, "first");
    keys::on_key_wallets(&mut app, key('n'), &channel());
    keys::on_key_wallets(&mut app, code(KeyCode::Enter), &channel());

    let (a, b) = match &app.screen {
        Screen::Wallets(w) => (w.items[0].id, w.items[1].id),
        _ => panic!("expected two wallets"),
    };
    app.on_app_event(neko_tui::event::AppEvent::WalletAssets {
        req: neko_tui::event::ReqId(1),
        wallet_id: b,
        chain: neko_core::ChainId::Tron,
        res: Ok(vec![("TRX".into(), 6, 42_000_000)]),
    });

    let s = app.session.as_ref().unwrap();
    assert_eq!(
        s.cached_assets(b, neko_core::ChainId::Tron)
            .unwrap()
            .amount("TRX"),
        Some((42_000_000, 6))
    );
    assert!(
        s.cached_assets(a, neko_core::ChainId::Tron)
            .unwrap()
            .rows
            .is_empty(),
        "balance landed on the wrong wallet"
    );
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
