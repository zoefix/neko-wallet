//! Drives the real state machine — including real Argon2id and a real
//! SQLCipher vault — without a terminal.

use neko_tui::app::{App, LockReason, Screen, SetupField};
use std::time::Duration;

const EMAIL: &str = "zoe@example.com";
const PW: &str = "correct horse battery staple xyzzy";

fn app_at(dir: &std::path::Path) -> App {
    App::new(dir.join("neko-wallet.db"))
}

fn type_in(app: &mut App, email: &str, pw: &str, confirm: Option<&str>) {
    app.email.clear();
    app.password.clear();
    app.confirm.clear();
    email.chars().for_each(|c| app.email.push(c));
    pw.chars().for_each(|c| app.password.push(c));
    if let Some(c) = confirm {
        c.chars().for_each(|ch| app.confirm.push(ch));
    }
}

/// Missing file -> first run. Existing file -> login.
#[test]
fn opening_screen_depends_on_whether_a_vault_exists() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(app_at(dir.path()).screen, Screen::FirstRun { .. }));

    std::fs::write(dir.path().join("neko-wallet.db"), b"whatever").unwrap();
    assert!(matches!(app_at(dir.path()).screen, Screen::Login { .. }));
}

/// The whole first-run -> unlocked path, with real key derivation.
#[test]
fn create_then_unlock_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_at(dir.path());

    type_in(&mut app, EMAIL, PW, Some(PW));
    assert_eq!(app.setup_blocker(), None, "form should be valid");

    let job = app.begin_unlock(true).expect("setup should dispatch");
    assert!(matches!(
        app.screen,
        Screen::Deriving { creating: true, .. }
    ));
    app.on_app_event(job.run());
    assert!(
        matches!(app.screen, Screen::Wallets(_)),
        "got {:?}",
        app.error
    );
    assert!(app.session.is_some());
    // Credentials must not linger in the UI after use.
    assert!(app.password.is_empty() && app.confirm.is_empty());

    drop(app);

    // Reopen: same file, now the login path.
    let mut app = app_at(dir.path());
    assert!(matches!(app.screen, Screen::Login { .. }));
    type_in(&mut app, EMAIL, PW, None);
    let job = app.begin_unlock(false).unwrap();
    app.on_app_event(job.run());
    assert!(
        matches!(app.screen, Screen::Wallets(_)),
        "got {:?}",
        app.error
    );
}

#[test]
fn wrong_password_returns_to_login_with_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_at(dir.path());
    type_in(&mut app, EMAIL, PW, Some(PW));
    let ev = app.begin_unlock(true).unwrap().run();
    app.on_app_event(ev);
    drop(app);

    let mut app = app_at(dir.path());
    type_in(&mut app, EMAIL, "wrong horse battery staple xyzzy", None);
    let ev = app.begin_unlock(false).unwrap().run();
    app.on_app_event(ev);

    assert!(matches!(app.screen, Screen::Login { .. }));
    assert!(app.session.is_none());
    assert!(app.error.is_some());
    assert!(
        app.password.is_empty(),
        "password must be cleared after a failed attempt"
    );
}

#[test]
fn weak_password_is_blocked_before_any_work_is_dispatched() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_at(dir.path());
    type_in(&mut app, EMAIL, "Password2026!", Some("Password2026!"));
    assert_eq!(app.setup_blocker(), Some("weak"));
    assert!(
        app.begin_unlock(true).is_none(),
        "weak password must not reach Argon2"
    );
    assert!(!dir.path().join("neko-wallet.db").exists());
}

#[test]
fn mismatched_confirmation_blocks_setup() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_at(dir.path());
    type_in(&mut app, EMAIL, PW, Some("something else entirely 99"));
    assert_eq!(app.setup_blocker(), Some("mismatch"));
    assert!(app.begin_unlock(true).is_none());
}

#[test]
fn malformed_email_blocks_setup() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_at(dir.path());
    type_in(&mut app, "not-an-email", PW, Some(PW));
    assert_eq!(app.setup_blocker(), Some("email"));
}

/// A late reply for a request the user already abandoned must be dropped, not
/// painted over whatever they are looking at now.
#[test]
fn stale_replies_are_ignored() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_at(dir.path());
    type_in(&mut app, EMAIL, PW, Some(PW));

    let stale = app.begin_unlock(true).unwrap();
    let stale_event = stale.run(); // completes, but...
    app.screen = Screen::FirstRun {
        focus: SetupField::Email,
        warned: false,
    }; // user backed out
    app.on_app_event(stale_event);

    assert!(
        matches!(app.screen, Screen::FirstRun { .. }),
        "stale reply hijacked the screen"
    );
    assert!(app.session.is_none());
}

#[test]
fn idle_timeout_locks_and_releases_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_at(dir.path());
    type_in(&mut app, EMAIL, PW, Some(PW));
    let ev = app.begin_unlock(true).unwrap().run();
    app.on_app_event(ev);
    assert!(app.session.is_some());

    app.autolock = Duration::from_millis(1);
    std::thread::sleep(Duration::from_millis(5));
    assert!(app.on_tick(), "tick should report a state change");

    assert!(
        app.session.is_none(),
        "session must be dropped on auto-lock"
    );
    assert!(matches!(
        app.screen,
        Screen::Locked {
            reason: LockReason::Idle
        }
    ));
}

#[test]
fn manual_lock_clears_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = app_at(dir.path());
    type_in(&mut app, EMAIL, PW, Some(PW));
    let ev = app.begin_unlock(true).unwrap().run();
    app.on_app_event(ev);
    app.lock(LockReason::Manual);
    assert!(app.session.is_none());
    assert!(matches!(
        app.screen,
        Screen::Locked {
            reason: LockReason::Manual
        }
    ));
}

/// Nothing in the app's Debug output may print key material.
#[test]
fn debug_output_never_leaks_credentials() {
    let mut app = App::new(std::path::PathBuf::from("/tmp/x.db"));
    type_in(&mut app, EMAIL, PW, Some(PW));
    let dumped = format!("{:?} {:?} {:?}", app.email, app.password, app.confirm);
    assert!(
        !dumped.contains("correct"),
        "password leaked via Debug: {dumped}"
    );
    assert!(dumped.contains("redacted"));
}
