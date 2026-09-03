//! The transfer flow, up to (but not including) anything that touches the
//! network. Signing is exercised for real; broadcasting is not.

use crossterm::event::{KeyCode, KeyEvent};
use neko_tui::app::{App, Screen};
use neko_tui::send::{self, SendStep};
use neko_tui::{keys, nav};

const EMAIL: &str = "zoe@example.com";
const PW: &str = "correct horse battery staple xyzzy";
const LEDGER_PHRASE: &str =
    "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const LEDGER_ADDR: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";
const TO: &str = "TNYxHL2s6Wjpx86NRwhekYzc27p3oDYrk6";

fn key(c: char) -> KeyEvent {
    KeyEvent::from(KeyCode::Char(c))
}
fn code(c: KeyCode) -> KeyEvent {
    KeyEvent::from(c)
}

fn on_assets(dir: &std::path::Path) -> App {
    let mut app = App::new(dir.join("neko-wallet.db"));
    EMAIL.chars().for_each(|c| app.email.push(c));
    PW.chars().for_each(|c| app.password.push(c));
    PW.chars().for_each(|c| app.confirm.push(c));
    let ev = app.begin_unlock(true).unwrap().run();
    app.on_app_event(ev);

    keys::on_key_wallets(&mut app, key('i'), &channel());
    let Screen::Wallets(w) = &mut app.screen else {
        panic!()
    };
    let Some(nav::WalletForm::ImportMnemonic { label, phrase, .. }) = &mut w.form else {
        panic!()
    };
    label.clear();
    "ledger".chars().for_each(|c| label.push(c));
    LEDGER_PHRASE.chars().for_each(|c| phrase.push(c));
    keys::on_key_wallets(&mut app, code(KeyCode::Enter), &channel());

    keys::on_key_wallets(&mut app, code(KeyCode::Enter), &channel()); // -> chains
    keys::on_key_chains(&mut app, code(KeyCode::Enter), &channel()); // -> assets
    assert!(matches!(app.screen, Screen::Assets { .. }));
    app
}

fn open_send(app: &mut App) {
    keys::on_key_assets(app, key('s'), &channel());
    assert!(
        matches!(app.screen, Screen::Send(_)),
        "send screen did not open"
    );
}

#[tokio::test]
async fn send_opens_with_the_selected_asset() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = on_assets(dir.path());

    open_send(&mut app);
    let Screen::Send(st) = &app.screen else {
        panic!()
    };
    assert_eq!(st.asset_label, "TRX");
    assert_eq!(st.from.to_string(), LEDGER_ADDR);

    // Selecting USDT gives a TRC20 transfer against this network's contract.
    keys::on_key_send(&mut app, code(KeyCode::Esc), &channel());
    keys::on_key_assets(&mut app, key('j'), &channel());
    open_send(&mut app);
    let Screen::Send(st) = &app.screen else {
        panic!()
    };
    assert_eq!(st.asset_label, "USDT");
    assert_eq!(
        st.asset.tron_fee_limit().unwrap(),
        100_000_000,
        "a contract call needs a fee limit"
    );
}

fn channel() -> tokio::sync::mpsc::UnboundedSender<neko_tui::event::AppEvent> {
    tokio::sync::mpsc::unbounded_channel().0
}

fn type_into_send(app: &mut App, s: &str) {
    let tx = channel();
    for c in s.chars() {
        keys::on_key_send(app, key(c), &tx);
    }
}

#[tokio::test]
async fn a_bad_recipient_blocks_progress_and_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = on_assets(dir.path());
    open_send(&mut app);

    type_into_send(&mut app, "not-an-address");
    let Screen::Send(st) = &app.screen else {
        panic!()
    };
    assert!(st.recipient_error().is_some());

    keys::on_key_send(&mut app, code(KeyCode::Enter), &channel());
    let Screen::Send(st) = &app.screen else {
        panic!()
    };
    assert!(
        matches!(st.step, SendStep::Recipient),
        "advanced past an invalid address"
    );
}

/// Sending to yourself is almost always a mistake worth catching.
#[tokio::test]
async fn own_address_is_flagged() {
    neko_i18n::set_locale(neko_i18n::Locale::English);
    let dir = tempfile::tempdir().unwrap();
    let mut app = on_assets(dir.path());
    open_send(&mut app);
    type_into_send(&mut app, LEDGER_ADDR);

    let Screen::Send(st) = &app.screen else {
        panic!()
    };
    assert_eq!(
        st.recipient_error(),
        Some("that is this wallet's own address")
    );
}

#[tokio::test]
async fn invalid_amounts_block_progress() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = on_assets(dir.path());
    open_send(&mut app);
    type_into_send(&mut app, TO);
    keys::on_key_send(&mut app, code(KeyCode::Enter), &channel());

    for bad in ["abc", "1.2345678", "0"] {
        let Screen::Send(st) = &mut app.screen else {
            panic!()
        };
        st.amount.clear();
        type_into_send(&mut app, bad);
        keys::on_key_send(&mut app, code(KeyCode::Enter), &channel());
        let Screen::Send(st) = &app.screen else {
            panic!()
        };
        assert!(
            matches!(st.step, SendStep::EnterAmount),
            "advanced with an invalid amount {bad:?}"
        );
    }
}

/// The confirmation gate: a valid transfer must still not broadcast until the
/// destination's trailing characters are retyped correctly.
#[tokio::test]
async fn review_requires_retyping_the_destination_tail() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = on_assets(dir.path());
    open_send(&mut app);
    type_into_send(&mut app, TO);
    keys::on_key_send(&mut app, code(KeyCode::Enter), &channel());
    type_into_send(&mut app, "1.5");

    // Skip the network round trip: inject the quote directly.
    let Screen::Send(st) = &mut app.screen else {
        panic!()
    };
    let req = st.build_request().unwrap();
    st.step = SendStep::Review {
        req: Box::new(req),
        params: Box::new(neko_core::ChainTxParams::Tron(Box::new(
            neko_tron::tx::TxParams {
                ref_block_num: 68_000_123,
                ref_block_hash: [0xab; 32],
                timestamp: 1_756_000_000_000,
                expiration: 1_756_000_060_000,
                fee_limit: 0,
            },
        ))),
        quote: None,
        typed: neko_tui::input::Field::new(false),
    };

    assert!(
        !app_confirm_ok(&app),
        "confirmation satisfied with nothing typed"
    );

    type_into_send(&mut app, "wrong!");
    assert!(
        !app_confirm_ok(&app),
        "confirmation satisfied by the wrong characters"
    );
    keys::on_key_send(&mut app, code(KeyCode::Enter), &channel());
    let Screen::Send(st) = &app.screen else {
        panic!()
    };
    assert!(
        matches!(st.step, SendStep::Review { .. }),
        "broadcast on a wrong confirmation"
    );
    assert!(st.error.is_some(), "no explanation given");

    // Now the real tail.
    let Screen::Send(st) = &mut app.screen else {
        panic!()
    };
    if let SendStep::Review { typed, .. } = &mut st.step {
        typed.clear();
    }
    type_into_send(&mut app, &send::confirm_tail(TO));
    assert!(app_confirm_ok(&app), "correct tail was not accepted");
}

fn app_confirm_ok(app: &App) -> bool {
    match &app.screen {
        Screen::Send(st) => st.confirmation_satisfied(),
        _ => false,
    }
}

/// The tail the user is asked for must belong to the destination, not the
/// sender — the whole point is checking where the money is going.
#[tokio::test]
async fn confirmation_tail_comes_from_the_destination() {
    assert_eq!(send::confirm_tail(TO), "oDYrk6");
    assert_ne!(send::confirm_tail(TO), send::confirm_tail(LEDGER_ADDR));
}

#[tokio::test]
async fn escape_leaves_the_send_flow() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = on_assets(dir.path());
    open_send(&mut app);
    keys::on_key_send(&mut app, code(KeyCode::Esc), &channel());
    assert!(matches!(app.screen, Screen::Assets { .. }));
}

/// Reaching the review gate must NOT be enough to spend. The password is the
/// last gate, and an unlocked terminal someone walked away from must not be
/// able to move funds.
#[tokio::test]
async fn broadcasting_requires_the_password() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = on_assets(dir.path());
    open_send(&mut app);
    type_into_send(&mut app, TO);
    keys::on_key_send(&mut app, code(KeyCode::Enter), &channel());
    type_into_send(&mut app, "1.5");
    inject_review(&mut app);

    // Retyping the tail alone advances to the password gate, not to signing.
    type_into_send(&mut app, &send::confirm_tail(TO));
    keys::on_key_send(&mut app, code(KeyCode::Enter), &channel());

    let Screen::Send(st) = &app.screen else {
        panic!()
    };
    assert!(
        matches!(st.step, SendStep::Authorize { .. }),
        "the retype gate alone let the transfer through"
    );
}

/// A wrong password must clear the field and sign nothing.
#[tokio::test]
async fn a_wrong_password_signs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = at_authorize(dir.path()).await;

    type_into_send(&mut app, "definitely not the password");
    keys::on_key_send(&mut app, code(KeyCode::Enter), &channel());
    // The check runs on a blocking thread; drive the reply by hand.
    let req = app.inflight.unwrap();
    keys::on_authorized(&mut app, req, false, &channel());

    let Screen::Send(st) = &app.screen else {
        panic!()
    };
    match &st.step {
        SendStep::Authorize {
            password, checking, ..
        } => {
            assert!(
                password.is_empty(),
                "the rejected password was left in the field"
            );
            assert!(!checking, "still stuck in the checking state");
        }
        other => panic!(
            "left the password gate on a wrong password: {:?}",
            std::mem::discriminant(other)
        ),
    }
    assert!(st.error.is_some(), "no explanation for the rejection");
}

/// The gate must ignore keystrokes while Argon2 is running, so a held Enter
/// cannot queue a second attempt.
#[tokio::test]
async fn input_is_ignored_while_verifying() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = at_authorize(dir.path()).await;
    type_into_send(&mut app, PW);
    keys::on_key_send(&mut app, code(KeyCode::Enter), &channel());

    let before = app.inflight;
    type_into_send(&mut app, "xxx");
    keys::on_key_send(&mut app, code(KeyCode::Enter), &channel());
    assert_eq!(
        app.inflight, before,
        "a second check was dispatched mid-verification"
    );
}

/// A stale authorisation reply must not trigger a broadcast.
#[tokio::test]
async fn stale_authorisations_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = at_authorize(dir.path()).await;
    keys::on_authorized(&mut app, neko_tui::event::ReqId(9999), true, &channel());

    let Screen::Send(st) = &app.screen else {
        panic!()
    };
    assert!(
        matches!(st.step, SendStep::Authorize { .. }),
        "a stale authorisation started a broadcast"
    );
}

/// Escape must abandon the transfer without signing.
#[tokio::test]
async fn escape_from_the_password_gate_signs_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = at_authorize(dir.path()).await;
    keys::on_key_send(&mut app, code(KeyCode::Esc), &channel());
    assert!(matches!(app.screen, Screen::Assets { .. }));
}

/// The password must never appear in the rendered buffer.
#[tokio::test]
async fn the_gate_masks_the_password() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = at_authorize(dir.path()).await;
    type_into_send(&mut app, "correct horse battery staple xyzzy");

    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 30)).unwrap();
    term.draw(|f| neko_tui::render::draw(f, &app)).unwrap();
    let buf = term.backend().buffer().clone();
    let text: String = (0..buf.area.height)
        .flat_map(|y| (0..buf.area.width).map(move |x| (x, y)))
        .map(|(x, y)| buf[(x, y)].symbol().to_string())
        .collect();
    assert!(
        !text.contains("correct"),
        "the password was rendered in clear text"
    );
    assert!(text.contains("****"), "the password field is not masked");
}

/// Drive the flow to the password gate.
async fn at_authorize(dir: &std::path::Path) -> App {
    let mut app = on_assets(dir);
    open_send(&mut app);
    type_into_send(&mut app, TO);
    keys::on_key_send(&mut app, code(KeyCode::Enter), &channel());
    type_into_send(&mut app, "1.5");
    inject_review(&mut app);
    type_into_send(&mut app, &send::confirm_tail(TO));
    keys::on_key_send(&mut app, code(KeyCode::Enter), &channel());
    app
}

/// Skip the network round trip and drop straight into the review step.
fn inject_review(app: &mut App) {
    let Screen::Send(st) = &mut app.screen else {
        panic!()
    };
    let req = st.build_request().unwrap();
    st.step = SendStep::Review {
        req: Box::new(req),
        params: Box::new(neko_core::ChainTxParams::Tron(Box::new(
            neko_tron::tx::TxParams {
                ref_block_num: 68_000_123,
                ref_block_hash: [0xab; 32],
                timestamp: 1_756_000_000_000,
                expiration: 1_756_000_060_000,
                fee_limit: 0,
            },
        ))),
        quote: None,
        typed: neko_tui::input::Field::new(false),
    };
}

/// Drives the REAL verification path — spawn_blocking and all — instead of
/// calling `on_authorized` by hand. Every other password-gate test stubs the
/// reply, so none of them would notice if the check never came back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_password_check_actually_completes() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = at_authorize(dir.path()).await;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    for c in PW.chars() {
        keys::on_key_send(&mut app, key(c), &tx);
    }
    keys::on_key_send(&mut app, code(KeyCode::Enter), &tx);
    assert!(app.inflight.is_some(), "no verification was dispatched");

    let ev = tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv())
        .await
        .expect("password verification never returned - the gate is stuck")
        .expect("channel closed");

    match ev {
        neko_tui::event::AppEvent::Authorized { ok, .. } => {
            assert!(ok, "the correct password was rejected")
        }
        other => panic!("unexpected event: {other:?}"),
    }
}

/// And the wrong password must come back too, not hang.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_password_also_returns() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = at_authorize(dir.path()).await;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    for c in "not the right password at all".chars() {
        keys::on_key_send(&mut app, key(c), &tx);
    }
    keys::on_key_send(&mut app, code(KeyCode::Enter), &tx);

    let ev = tokio::time::timeout(std::time::Duration::from_secs(20), rx.recv())
        .await
        .expect("a wrong password never returned")
        .expect("channel closed");
    match ev {
        neko_tui::event::AppEvent::Authorized { ok, .. } => assert!(!ok),
        other => panic!("unexpected event: {other:?}"),
    }
}

/// The same check at the profile a real vault actually uses.
///
/// Every other test runs at TESTONLY (64 MiB); a real vault calibrates to
/// PARANOID (1 GiB), and the difference is exactly the kind of thing that only
/// shows up in production.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_password_check_completes_at_the_real_profile() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("neko-wallet.db");

    let mut app = App::new(path.clone());
    EMAIL.chars().for_each(|c| app.email.push(c));
    PW.chars().for_each(|c| app.password.push(c));
    PW.chars().for_each(|c| app.confirm.push(c));
    // The real setup path, including KDF calibration.
    let ev = app.begin_unlock(true).unwrap().run();
    app.on_app_event(ev);
    assert!(app.session.is_some(), "setup failed");

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let verifier = app.password_verifier();
    let pw = PW.to_string();
    let t0 = std::time::Instant::now();
    tokio::task::spawn_blocking(move || {
        let ok = verifier(&path, &pw);
        let _ = tx.send(ok);
    });

    let ok = tokio::time::timeout(std::time::Duration::from_secs(30), rx.recv())
        .await
        .expect("verification never returned at the production KDF profile")
        .expect("channel closed");
    eprintln!("  verification took {:?}", t0.elapsed());
    assert!(
        ok,
        "the correct password was rejected at the production profile"
    );
}

/// The full chain: dispatch, real verification, then the reply handler — with
/// no step stubbed. If the gate is stuck in production, it is stuck here.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_correct_password_leaves_the_gate() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = at_authorize(dir.path()).await;

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    for c in PW.chars() {
        keys::on_key_send(&mut app, key(c), &tx);
    }
    keys::on_key_send(&mut app, code(KeyCode::Enter), &tx);

    let ev = tokio::time::timeout(std::time::Duration::from_secs(30), rx.recv())
        .await
        .expect("verification never returned")
        .expect("channel closed");
    let neko_tui::event::AppEvent::Authorized { req, ok } = ev else {
        panic!("unexpected event")
    };
    assert!(ok, "correct password rejected");

    keys::on_authorized(&mut app, req, ok, &tx);

    let Screen::Send(st) = &app.screen else {
        panic!("left the send screen entirely")
    };
    assert!(
        !matches!(st.step, SendStep::Authorize { .. }),
        "an accepted password left the flow stuck on 'verifying...' - \
         the user sees a spinner forever and nothing is ever signed"
    );
}

/// No abort path may leave the spinner running. "Verifying..." forever reads as
/// "still working" when it means "gave up", and the user waits indefinitely for
/// a transaction that was never signed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn aborting_always_reaches_a_visible_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = at_authorize(dir.path()).await;
    let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

    // Authorisation succeeds, but the vault locked in the meantime.
    let req = neko_tui::event::ReqId(999);
    app.inflight = Some(req);
    app.session = None;
    keys::on_authorized(&mut app, req, true, &tx);

    let Screen::Send(st) = &app.screen else {
        panic!()
    };
    match &st.step {
        SendStep::Failed(msg) => assert!(!msg.is_empty(), "failed with no explanation"),
        other => panic!(
            "left the user on a spinner instead of reporting: {}",
            neko_tui::send::step_name(other)
        ),
    }
}

/// Every spinner must keep animating, or the UI looks frozen exactly when the
/// user most wants to know something is happening.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spinner_states_request_repaints() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = at_authorize(dir.path()).await;

    // Idle at the password prompt: no repaint needed.
    assert!(!app.on_tick(), "an idle gate should not force repaints");

    if let Screen::Send(st) = &mut app.screen {
        if let SendStep::Authorize { checking, .. } = &mut st.step {
            *checking = true;
        }
    }
    assert!(app.on_tick(), "the verifying spinner would not animate");

    if let Screen::Send(st) = &mut app.screen {
        st.step = SendStep::Broadcasting;
    }
    assert!(app.on_tick(), "the broadcasting spinner would not animate");

    if let Screen::Send(st) = &mut app.screen {
        st.step = SendStep::Quoting;
    }
    assert!(app.on_tick(), "the quoting spinner would not animate");
}
