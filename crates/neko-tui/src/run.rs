//! Terminal setup, the event loop, and teardown.

use std::io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyModifiers};
use futures_util::StreamExt;
use ratatui::DefaultTerminal;
use tokio::sync::mpsc;
use tokio::time::MissedTickBehavior;

use crate::app::{App, LockReason, Screen, SetupField};
use crate::event::AppEvent;
use crate::render;

/// Never let a burst of key repeats or replies drive more than ~60 fps.
const FRAME_FLOOR: Duration = Duration::from_millis(16);
const TICK: Duration = Duration::from_millis(100);

pub async fn run(db_path: PathBuf) -> io::Result<()> {
    run_with(db_path, None).await
}

/// `warning` is shown inside the TUI rather than printed before it. Anything
/// written to the terminal before `ratatui::init()` is immediately hidden by
/// the alternate screen, so a startup warning printed the obvious way is a
/// warning nobody sees.
pub async fn run_with(db_path: PathBuf, warning: Option<String>) -> io::Result<()> {
    let mut terminal = ratatui::init();
    let out = main_loop(&mut terminal, db_path, warning).await;
    // Restore unconditionally, including on error, or we leave the user's
    // terminal in raw mode with no cursor.
    ratatui::restore();
    out
}

async fn main_loop(
    term: &mut DefaultTerminal,
    db_path: PathBuf,
    warning: Option<String>,
) -> io::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let mut app = App::new(db_path);
    // The locale is process-global for `t()`; apply the detected one now.
    app.apply_locale();
    app.startup_warning = warning;
    let mut keys = EventStream::new();
    let mut tick = tokio::time::interval(TICK);
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    let mut dirty = true;
    let mut last_draw = Instant::now() - FRAME_FLOOR;

    loop {
        if dirty && last_draw.elapsed() >= FRAME_FLOOR {
            let size = term.size()?;
            app.set_viewport(size.width, size.height);
            term.draw(|f| render::draw(f, &app))?;
            last_draw = Instant::now();
            dirty = false;
        }

        tokio::select! {
            biased; // app events, then keys, then ticks: never starve on key repeat

            Some(ev) = rx.recv() => { handle_event(&mut app, ev, &tx); dirty = true; }

            // Full match, not a refutable pattern: a failing pattern disables
            // the branch for that poll, and if every branch disables, select!
            // panics.
            ev = keys.next() => match ev {
                Some(Ok(Event::Key(k))) if k.is_press() => {
                    on_key(&mut app, k, &tx);
                    dirty = true;
                }
                Some(Ok(Event::Resize(_, _))) => dirty = true,
                Some(Ok(_)) => {}
                Some(Err(e)) => return Err(e),
                None => break,
            },

            _ = tick.tick() => {
                dirty |= app.on_tick();
                // Deferred to the tick so the list paints from cache first and
                // the network work never delays the first frame.
                if app.assets_stale {
                    app.assets_stale = false;
                    app.refresh_wallet_assets(&tx);
                    dirty = true;
                }
            }
        }

        // Coalesce: drain whatever is already queued so a burst produces one
        // repaint rather than one per message.
        while let Ok(ev) = rx.try_recv() {
            handle_event(&mut app, ev, &tx);
            dirty = true;
        }
        if app.should_quit {
            break;
        }
    }
    Ok(())
}

/// Authorisation continues into a broadcast, which needs the event sender, so
/// it is routed here rather than inside `App::on_app_event`.
fn handle_event(app: &mut App, ev: AppEvent, tx: &mpsc::UnboundedSender<AppEvent>) {
    match ev {
        AppEvent::Authorized { req, ok } => crate::keys::on_authorized(app, req, ok, tx),
        AppEvent::Blockhash { req, res } => crate::keys::on_blockhash(app, req, res, tx),
        other => app.on_app_event(other),
    }
}

fn on_key(app: &mut App, k: KeyEvent, tx: &mpsc::UnboundedSender<AppEvent>) {
    app.last_input = Instant::now();

    // In raw mode ISIG is off, so Ctrl-C/Ctrl-Q arrive as ordinary key events.
    if k.modifiers.contains(KeyModifiers::CONTROL) {
        match k.code {
            KeyCode::Char('q') | KeyCode::Char('c') => {
                app.should_quit = true;
                return;
            }
            KeyCode::Char('l') => {
                if app.session.is_some() {
                    app.lock(LockReason::Manual);
                }
                return;
            }
            _ => {}
        }
    }

    match &app.screen {
        Screen::Deriving { .. } => {} // input is ignored while Argon2 runs
        Screen::FirstRun { focus, .. } => on_key_first_run(app, k, *focus, tx),
        Screen::Login { .. } => on_key_login(app, k, tx),
        Screen::Locked { .. } => on_key_locked(app, k, tx),
        Screen::Wallets(_) => crate::keys::on_key_wallets(app, k, tx),
        Screen::Chains { .. } => crate::keys::on_key_chains(app, k, tx),
        Screen::Assets { .. } => crate::keys::on_key_assets(app, k, tx),
        Screen::Send(_) => crate::keys::on_key_send(app, k, tx),
        Screen::History(_) => crate::keys::on_key_history(app, k, tx),
        Screen::Settings(_) => crate::keys::on_key_settings(app, k, tx),
        Screen::Reveal { .. } => crate::keys::on_key_reveal(app, k),
    }
}

fn dispatch(app: &mut App, creating: bool, tx: &mpsc::UnboundedSender<AppEvent>) {
    let Some(job) = app.begin_unlock(creating) else {
        return;
    };
    let tx = tx.clone();
    // Argon2id is CPU-bound. On the async runtime it would occupy a worker for
    // a second or more; spawn_blocking is exactly what this is for.
    tokio::task::spawn_blocking(move || {
        let _ = tx.send(job.run());
    });
}

fn on_key_first_run(
    app: &mut App,
    k: KeyEvent,
    focus: SetupField,
    tx: &mpsc::UnboundedSender<AppEvent>,
) {
    match k.code {
        KeyCode::Tab | KeyCode::Down => {
            let next = match focus {
                SetupField::Email => SetupField::Password,
                SetupField::Password => SetupField::Confirm,
                SetupField::Confirm => SetupField::Email,
            };
            app.screen = Screen::FirstRun {
                focus: next,
                warned: true,
            };
        }
        KeyCode::BackTab | KeyCode::Up => {
            let prev = match focus {
                SetupField::Email => SetupField::Confirm,
                SetupField::Password => SetupField::Email,
                SetupField::Confirm => SetupField::Password,
            };
            app.screen = Screen::FirstRun {
                focus: prev,
                warned: true,
            };
        }
        KeyCode::Enter => dispatch(app, true, tx),
        KeyCode::Backspace => field_mut(app, focus).backspace(),
        KeyCode::Char(c) => field_mut(app, focus).push(c),
        _ => {}
    }
}

fn field_mut(app: &mut App, focus: SetupField) -> &mut crate::input::Field {
    match focus {
        SetupField::Email => &mut app.email,
        SetupField::Password => &mut app.password,
        SetupField::Confirm => &mut app.confirm,
    }
}

fn on_key_login(app: &mut App, k: KeyEvent, tx: &mpsc::UnboundedSender<AppEvent>) {
    let email_focused = matches!(
        app.screen,
        Screen::Login {
            email_focused: true
        }
    );
    match k.code {
        KeyCode::Tab | KeyCode::Down | KeyCode::Up | KeyCode::BackTab => {
            app.screen = Screen::Login {
                email_focused: !email_focused,
            };
        }
        KeyCode::Enter => {
            if email_focused && app.password.is_empty() {
                app.screen = Screen::Login {
                    email_focused: false,
                };
            } else {
                dispatch(app, false, tx);
            }
        }
        KeyCode::Backspace => {
            if email_focused {
                app.email.backspace()
            } else {
                app.password.backspace()
            }
        }
        KeyCode::Char(c) => {
            if email_focused {
                app.email.push(c)
            } else {
                app.password.push(c)
            }
        }
        _ => {}
    }
}

fn on_key_locked(app: &mut App, k: KeyEvent, tx: &mpsc::UnboundedSender<AppEvent>) {
    match k.code {
        KeyCode::Enter => dispatch(app, false, tx),
        KeyCode::Backspace => app.password.backspace(),
        KeyCode::Char(c) => app.password.push(c),
        _ => {}
    }
}
