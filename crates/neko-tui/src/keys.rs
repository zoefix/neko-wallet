//! Key handling for the wallet screens.

use crossterm::event::{KeyCode, KeyEvent};
use neko_core::NewWalletSpec;

use crate::app::{App, Screen};

/// Shorthand for the event channel the async tasks report back on.
pub type Sender = tokio::sync::mpsc::UnboundedSender<crate::event::AppEvent>;
use crate::input::Field;
use crate::nav::{RevealStage, WalletForm, CHAINS};

const REVEAL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

pub fn on_key_wallets(app: &mut App, k: KeyEvent, tx: &Sender) {
    // A form, when open, swallows everything.
    if matches!(&app.screen, Screen::Wallets(w) if w.form.is_some()) {
        return on_key_wallet_form(app, k);
    }
    match k.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if let Screen::Wallets(w) = &mut app.screen {
                w.move_by(1)
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Screen::Wallets(w) = &mut app.screen {
                w.move_by(-1)
            }
        }
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => app.open_chains(),
        KeyCode::Char('n') => app.wallet_form(WalletForm::New {
            label: prefilled_name(app),
            words: 12,
        }),
        KeyCode::Char('i') => app.wallet_form(WalletForm::ImportMnemonic {
            label: prefilled_name(app),
            phrase: Field::new(false),
            passphrase: Field::new(true),
            focus: 0,
        }),
        KeyCode::Char('I') => app.wallet_form(WalletForm::ImportPrivkey {
            label: prefilled_name(app),
            hex: Field::new(true),
            focus: 0,
        }),
        KeyCode::Char('r') => {
            if let Screen::Wallets(w) = &app.screen {
                if let Some(c) = w.current() {
                    let (id, mut label) = (c.id, Field::new(false));
                    c.label.chars().for_each(|ch| label.push(ch));
                    app.wallet_form(WalletForm::Rename { id, label });
                }
            }
        }
        KeyCode::Char('d') => {
            if let Screen::Wallets(w) = &app.screen {
                if let Some(c) = w.current() {
                    let (id, name) = (c.id, c.label.clone());
                    app.wallet_form(WalletForm::Delete {
                        id,
                        name,
                        typed: Field::new(false),
                    });
                }
            }
        }
        KeyCode::Char('m') => app.open_reveal(),
        KeyCode::Char(',') => app.open_settings(),
        KeyCode::Char('R') => app.refresh_wallet_assets(tx),
        KeyCode::Char('y') => {
            // A wallet has an address per chain, so copying "the" address is
            // ambiguous here. The chain screen is where one is unambiguous;
            // this copies the list's own chain-labelled pair rather than
            // silently picking one.
            if let Screen::Wallets(w) = &app.screen {
                if let Some(c) = w.current() {
                    let text = c
                        .addresses
                        .iter()
                        .map(|(chain, a)| format!("{}: {a}", chain.label()))
                        .collect::<Vec<_>>()
                        .join("\n");
                    app.copy_to_clipboard(&text);
                }
            }
        }
        _ => {}
    }
}

fn prefilled_name(app: &App) -> Field {
    let n = match &app.screen {
        Screen::Wallets(w) => w.items.len() + 1,
        _ => 1,
    };
    let mut f = Field::new(false);
    format!("Wallet {n}").chars().for_each(|c| f.push(c));
    f
}

fn on_key_wallet_form(app: &mut App, k: KeyEvent) {
    if k.code == KeyCode::Esc {
        app.close_form();
        return;
    }
    if k.code == KeyCode::Enter {
        return submit_form(app);
    }

    let Screen::Wallets(w) = &mut app.screen else {
        return;
    };
    let Some(form) = &mut w.form else { return };

    // Tab cycles focus (and doubles as the 12/24 toggle on the new-wallet form).
    if matches!(k.code, KeyCode::Tab | KeyCode::BackTab) {
        match form {
            WalletForm::New { words, .. } => *words = if *words == 12 { 24 } else { 12 },
            WalletForm::ImportMnemonic { focus, .. } => *focus = (*focus + 1) % 3,
            WalletForm::ImportPrivkey { focus, .. } => *focus = (*focus + 1) % 2,
            _ => {}
        }
        return;
    }

    let field: &mut Field = match form {
        WalletForm::New { label, .. } => label,
        WalletForm::Rename { label, .. } => label,
        WalletForm::Delete { typed, .. } => typed,
        WalletForm::ImportMnemonic {
            label,
            phrase,
            passphrase,
            focus,
        } => match focus {
            0 => label,
            1 => phrase,
            _ => passphrase,
        },
        WalletForm::ImportPrivkey { label, hex, focus } => {
            if *focus == 0 {
                label
            } else {
                hex
            }
        }
    };
    match k.code {
        KeyCode::Backspace => field.backspace(),
        KeyCode::Char(c) => field.push(c),
        _ => {}
    }
}

fn submit_form(app: &mut App) {
    let Screen::Wallets(w) = &app.screen else {
        return;
    };
    let Some(form) = &w.form else { return };

    // Build the request first so the session borrow is short.
    enum Action {
        Create(String, usize),
        ImportPhrase(String, String, Option<String>),
        ImportKey(String, String),
        Rename(i64, String),
        Delete(i64),
        Reject(&'static str),
    }
    let action = match form {
        WalletForm::New { label, words } => Action::Create(label.value().into(), *words),
        WalletForm::ImportMnemonic {
            label,
            phrase,
            passphrase,
            ..
        } => Action::ImportPhrase(
            label.value().into(),
            phrase.value().into(),
            Some(passphrase.value().to_string()).filter(|p| !p.is_empty()),
        ),
        WalletForm::ImportPrivkey { label, hex, .. } => {
            Action::ImportKey(label.value().into(), hex.value().into())
        }
        WalletForm::Rename { id, label } => Action::Rename(*id, label.value().into()),
        WalletForm::Delete { id, name, typed } => {
            // Typing the name is the only confirmation that proves intent.
            if typed.value() == name {
                Action::Delete(*id)
            } else {
                Action::Reject(neko_i18n::t(neko_i18n::Key::Form_NameMismatch))
            }
        }
    };

    let Some(session) = app.session.as_mut() else {
        return;
    };
    let result = match action {
        Action::Create(label, words) => session
            .create_wallet(&label, NewWalletSpec::Generate { words })
            .map(|_| ()),
        Action::ImportPhrase(label, phrase, pass) => session
            .create_wallet(
                &label,
                NewWalletSpec::ImportMnemonic {
                    phrase: &phrase,
                    passphrase: pass.as_deref(),
                },
            )
            .map(|_| ()),
        Action::ImportKey(label, hex) => session
            .create_wallet(&label, NewWalletSpec::ImportPrivateKey { hex: &hex })
            .map(|_| ()),
        Action::Rename(id, label) => session.rename_wallet(id, &label),
        Action::Delete(id) => session.delete_wallet(id),
        Action::Reject(msg) => {
            if let Screen::Wallets(w) = &mut app.screen {
                w.error = Some(msg.to_string());
            }
            return;
        }
    };

    match result {
        Ok(()) => {
            app.close_form();
            app.refresh_wallets();
        }
        Err(e) => {
            if let Screen::Wallets(w) = &mut app.screen {
                w.error = Some(e.to_string());
            }
        }
    }
}

pub fn on_key_chains(app: &mut App, k: KeyEvent, tx: &Sender) {
    match k.code {
        KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
            app.pop();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if let Screen::Chains { selected, .. } = &mut app.screen {
                *selected = (*selected + 1) % CHAINS.len();
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Screen::Chains { selected, .. } = &mut app.screen {
                *selected = (*selected + CHAINS.len() - 1) % CHAINS.len();
            }
        }
        KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => app.open_assets(tx),
        KeyCode::Char('m') => app.open_reveal(),
        _ => {}
    }
}

pub fn on_key_assets(app: &mut App, k: KeyEvent, tx: &Sender) {
    match k.code {
        KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
            app.pop();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if let Screen::Assets { selected, .. } = &mut app.screen {
                *selected = (*selected + 1) % 2;
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Screen::Assets { selected, .. } = &mut app.screen {
                *selected = (*selected + 1) % 2;
            }
        }
        KeyCode::Char('y') => {
            if let Screen::Assets { address, .. } = &app.screen {
                let a = address.clone();
                app.copy_to_clipboard(&a);
            }
        }
        KeyCode::Char('s') => app.open_send(tx),
        KeyCode::Char('t') => app.open_history(tx),
        KeyCode::Char('R') => app.fetch_balances(tx),
        _ => {}
    }
}

/// The reveal screen deliberately has **no copy action wired up at all**. That
/// is stronger than refusing to copy: there is no code path from here to any
/// clipboard backend, so no future bug can invert a check and enable it.
pub fn on_key_reveal(app: &mut App, k: KeyEvent) {
    let Screen::Reveal {
        wallet_id, stage, ..
    } = &mut app.screen
    else {
        return;
    };
    let wallet_id = *wallet_id;

    match stage {
        RevealStage::Gate { password, checking } => match k.code {
            KeyCode::Esc => {
                app.pop();
            }
            KeyCode::Enter => {
                if password.is_empty() {
                    return;
                }
                *checking = true;
                let pw = password.value().to_string();
                let path = app.db_path.clone();
                // Runs the FULL Argon2id derivation even though the session is
                // already unlocked: the threat is an unlocked terminal someone
                // walked away from, and reusing the in-memory key would make
                // this check decorative.
                let res = app
                    .session
                    .as_ref()
                    .map(|s| s.reveal_mnemonic(&path, wallet_id, &pw))
                    .unwrap_or(Err(neko_core::CoreError::Locked));
                match res {
                    Ok(phrase) => {
                        let words: Vec<String> =
                            phrase.split_whitespace().map(str::to_string).collect();
                        if let Screen::Reveal { stage, .. } = &mut app.screen {
                            *stage = RevealStage::Words {
                                words,
                                cursor: 0,
                                show_all: false,
                                hide_at: std::time::Instant::now() + REVEAL_TIMEOUT,
                            };
                        }
                    }
                    Err(e) => {
                        app.toast(e.to_string());
                        if let Screen::Reveal { stage, .. } = &mut app.screen {
                            *stage = RevealStage::Gate {
                                password: Field::new(true),
                                checking: false,
                            };
                        }
                    }
                }
            }
            KeyCode::Backspace => password.backspace(),
            KeyCode::Char(c) => password.push(c),
            _ => {}
        },
        RevealStage::Words {
            words,
            cursor,
            show_all,
            hide_at,
        } => match k.code {
            KeyCode::Esc => {
                app.pop();
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
                *cursor = (*cursor + 1) % words.len();
                *hide_at = std::time::Instant::now() + REVEAL_TIMEOUT;
            }
            KeyCode::Left | KeyCode::Char('h') => {
                *cursor = (*cursor + words.len() - 1) % words.len();
                *hide_at = std::time::Instant::now() + REVEAL_TIMEOUT;
            }
            KeyCode::Char(' ') => {
                *show_all = !*show_all;
                *hide_at = std::time::Instant::now() + REVEAL_TIMEOUT;
            }
            _ => {}
        },
    }
}

/// The transfer flow.
///
/// Signing happens right here on the main thread, where the session lives. Only
/// the finished bytes are handed to the runtime for broadcast, so the private
/// key never crosses a thread boundary.
pub fn on_key_send(app: &mut App, k: KeyEvent, tx: &Sender) {
    use crate::send::SendStep;

    if k.code == KeyCode::Esc {
        // Cancelling is always safe before "broadcast"; after it, the network
        // has the transaction and Esc just closes the screen.
        app.pop();
        return;
    }

    let Screen::Send(st) = &mut app.screen else {
        return;
    };

    match &mut st.step {
        SendStep::Recipient => match k.code {
            KeyCode::Enter | KeyCode::Tab | KeyCode::Down => {
                if st.recipient_error().is_none() && !st.to.is_empty() {
                    st.step = SendStep::EnterAmount;
                }
            }
            KeyCode::Backspace => st.to.backspace(),
            KeyCode::Char(c) => st.to.push(c),
            _ => {}
        },

        SendStep::EnterAmount => match k.code {
            KeyCode::Tab | KeyCode::Up | KeyCode::BackTab => st.step = SendStep::Recipient,
            KeyCode::Enter => {
                if st.amount_error().is_some() || st.amount.is_empty() {
                    return;
                }
                match st.build_request() {
                    Ok(req) => {
                        st.step = SendStep::Quoting;
                        let id = app.next_req();
                        app.inflight = Some(id);
                        let client = app.chain_client(req.chain());
                        let tx = tx.clone();
                        tokio::spawn(async move {
                            let res = crate::chain::quote(&client, &req).await.map(Box::new);
                            let _ = tx.send(crate::event::AppEvent::Quoted { req: id, res });
                        });
                    }
                    Err(e) => st.error = Some(e.to_string()),
                }
            }
            // No amount contains an `m`, so this cannot swallow a keystroke
            // somebody meant as input.
            KeyCode::Char('m') | KeyCode::Char('M') => st.request_max(),
            KeyCode::Backspace => {
                // Editing by hand withdraws the request: the amount is now
                // whatever is typed, not the balance minus a fee.
                st.max_requested = false;
                st.held_back = None;
                st.amount.backspace();
            }
            KeyCode::Char(c) => {
                st.max_requested = false;
                st.held_back = None;
                st.amount.push(c);
            }
            _ => {}
        },

        SendStep::Review { typed, .. } => match k.code {
            KeyCode::Backspace => typed.backspace(),
            KeyCode::Char(c) => typed.push(c),
            KeyCode::Enter => {
                // The retyped tail must match, or nothing happens.
                if !st.confirmation_satisfied() {
                    st.error = Some(neko_i18n::t(neko_i18n::Key::Send_ErrNoMatch).to_string());
                    return;
                }
                advance_to_authorize(app);
            }
            _ => {}
        },

        SendStep::Authorize {
            password, checking, ..
        } => {
            if *checking {
                return; // Argon2 is running; ignore input
            }
            match k.code {
                KeyCode::Backspace => password.backspace(),
                KeyCode::Char(c) => password.push(c),
                KeyCode::Enter => {
                    if password.is_empty() {
                        return;
                    }
                    *checking = true;
                    let pw = password.value().to_string();
                    let path = app.db_path.clone();
                    let id = app.next_req();
                    app.inflight = Some(id);
                    let tx = tx.clone();
                    // Argon2id is CPU-bound: on the async runtime it would
                    // stall a worker for hundreds of milliseconds.
                    let verifier = app.password_verifier();
                    tokio::task::spawn_blocking(move || {
                        let ok = verifier(&path, &pw);
                        let _ = tx.send(crate::event::AppEvent::Authorized { req: id, ok });
                    });
                }
                _ => {}
            }
        }

        SendStep::Done { txid, .. } => {
            if k.code == KeyCode::Char('y') {
                let t = txid.clone();
                app.copy_to_clipboard(&t);
            }
        }

        SendStep::Failed(_) => {
            if k.code == KeyCode::Enter {
                app.pop();
            }
        }

        SendStep::Quoting | SendStep::Broadcasting => {}
    }
}

fn sign_and_broadcast(app: &mut App, tx: &Sender) {
    let Screen::Send(st) = &app.screen else {
        return;
    };
    // Reached only from the password gate. Matching the wrong step here is how
    // the flow once hung on "verifying..." forever: nothing signed, nothing
    // said. Every exit below now reports something rather than returning
    // silently, because a spinner that never resolves tells the user nothing.
    let SendStepRef { req, params } = match &st.step {
        crate::send::SendStep::Authorize { req, params, .. } => SendStepRef {
            req: (**req).clone(),
            params: (**params).clone(),
        },
        other => {
            let name = crate::send::step_name(other);
            fail_send(
                app,
                neko_i18n::tf(neko_i18n::Key::Send_CannotSign, &[("step", name)]),
            );
            return;
        }
    };

    // Sign here, on the thread that owns the session.
    let signed = match app.session.as_ref().map(|s| s.sign_transfer(&req, &params)) {
        Some(Ok(s)) => s,
        Some(Err(e)) => {
            fail_send(app, e.to_string());
            return;
        }
        None => {
            fail_send(
                app,
                neko_i18n::t(neko_i18n::Key::Send_VaultLocked).to_string(),
            );
            return;
        }
    };

    if let Screen::Send(st) = &mut app.screen {
        st.error = None;
        st.step = crate::send::SendStep::Broadcasting;
    }

    let id = app.next_req();
    app.inflight = Some(id);
    let client = app.chain_client(req.chain());
    let tx = tx.clone();
    let raw = signed.raw;
    tokio::spawn(async move {
        let res = crate::chain::broadcast(&client, raw).await;
        let _ = tx.send(crate::event::AppEvent::Broadcast { req: id, res });
    });
}

struct SendStepRef {
    req: neko_core::TransferRequest,
    params: neko_core::ChainTxParams,
}

pub fn on_key_settings(app: &mut App, k: KeyEvent, tx: &Sender) {
    if let Screen::Settings(st) = &mut app.screen {
        // A text row being edited swallows input.
        if st.editing.is_some() {
            let row = st.row();
            match k.code {
                KeyCode::Esc => st.editing = None,
                KeyCode::Backspace => st.editing.as_mut().unwrap().backspace(),
                KeyCode::Char(c) => st.editing.as_mut().unwrap().push(c),
                KeyCode::Enter => {
                    let value = st.editing.take().unwrap().value().to_string();
                    apply_text_setting(app, row, &value);
                }
                _ => {}
            }
            return;
        }
    }

    match k.code {
        KeyCode::Esc | KeyCode::Char('h') => {
            app.pop();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if let Screen::Settings(st) = &mut app.screen {
                st.move_by(1)
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Screen::Settings(st) = &mut app.screen {
                st.move_by(-1)
            }
        }
        KeyCode::Right | KeyCode::Char('>') => cycle_setting(app, true, tx),
        KeyCode::Left | KeyCode::Char('<') => cycle_setting(app, false, tx),
        KeyCode::Enter => begin_edit(app, tx),
        _ => {}
    }
}

fn cycle_setting(app: &mut App, forward: bool, tx: &Sender) {
    use crate::nav::SettingRow;
    let Screen::Settings(st) = &app.screen else {
        return;
    };
    match st.row() {
        SettingRow::Language => app.cycle_language(forward),
        SettingRow::AutoLock => {
            let mins = app.autolock.as_secs() / 60;
            let next = match mins {
                1 => 5,
                5 => 15,
                15 => 30,
                30 => 60,
                _ => 1,
            };
            app.autolock = std::time::Duration::from_secs(next * 60);
            if let Some(s) = app.session.as_ref() {
                let _ = s.set_setting(
                    neko_store::repo::settings::keys::AUTOLOCK_SECS,
                    &(next * 60).to_string(),
                );
            }
        }
        SettingRow::BorderStyle => {
            app.border = match app.border {
                crate::theme::BorderStyle::Unicode => crate::theme::BorderStyle::Ascii,
                crate::theme::BorderStyle::Ascii => crate::theme::BorderStyle::Unicode,
            };
        }
        SettingRow::ApiKey | SettingRow::BscApiKey | SettingRow::NodeUrl => begin_edit(app, tx),
    }
}

fn begin_edit(app: &mut App, _tx: &Sender) {
    use crate::nav::SettingRow;
    let Screen::Settings(st) = &mut app.screen else {
        return;
    };
    match st.row() {
        // The API key is a credential: masked while typing, never rendered back.
        SettingRow::ApiKey | SettingRow::BscApiKey => st.editing = Some(Field::new(true)),
        SettingRow::NodeUrl => st.editing = Some(Field::new(false)),
        _ => {}
    }
}

fn apply_text_setting(app: &mut App, row: crate::nav::SettingRow, value: &str) {
    use crate::nav::SettingRow;
    match row {
        SettingRow::BscApiKey => {
            app.set_bsc_api_key(value.trim());
            app.toast(if value.trim().is_empty() {
                "BNB Chain history key cleared"
            } else {
                "BNB Chain history key saved"
            });
        }
        SettingRow::ApiKey => {
            app.set_api_key(value.trim());
            app.toast(if value.trim().is_empty() {
                "API key cleared"
            } else {
                "API key saved"
            });
        }
        SettingRow::NodeUrl => {
            let v = value.trim();
            app.node_url = if v.is_empty() {
                None
            } else {
                Some(v.to_string())
            };
            if let Some(s) = app.session.as_ref() {
                let _ = s.set_setting(neko_store::repo::settings::keys::NODE_URL, v);
            }
            app.toast(neko_i18n::t(neko_i18n::Key::Settings_NodeSaved));
        }
        _ => {}
    }
}

/// Move from the retype gate to the password gate.
fn advance_to_authorize(app: &mut App) {
    let Screen::Send(st) = &mut app.screen else {
        return;
    };
    let (req, params) = match &st.step {
        crate::send::SendStep::Review { req, params, .. } => (req.clone(), params.clone()),
        _ => return,
    };
    st.error = None;
    st.step = crate::send::SendStep::Authorize {
        req,
        params,
        password: Field::new(true),
        checking: false,
    };
}

/// Called on the main thread once the password check comes back.
pub fn on_authorized(app: &mut App, req: crate::event::ReqId, ok: bool, tx: &Sender) {
    if app.inflight != Some(req) {
        return;
    }
    app.inflight = None;
    if !ok {
        if let Screen::Send(st) = &mut app.screen {
            if let crate::send::SendStep::Authorize {
                password, checking, ..
            } = &mut st.step
            {
                password.clear();
                *checking = false;
                st.error = Some(neko_i18n::t(neko_i18n::Key::Send_WrongPassword).to_string());
            }
        }
        return;
    }
    // Authorised: sign here, on the thread that owns the session.
    sign_and_broadcast(app, tx);
}

pub fn on_key_history(app: &mut App, k: KeyEvent, tx: &Sender) {
    match k.code {
        KeyCode::Esc | KeyCode::Char('h') | KeyCode::Left => {
            app.pop();
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if let Screen::History(h) = &mut app.screen {
                h.move_by(1)
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if let Screen::History(h) = &mut app.screen {
                h.move_by(-1)
            }
        }
        KeyCode::PageDown => {
            if let Screen::History(h) = &mut app.screen {
                let p = h.page as isize;
                h.move_by(p)
            }
        }
        KeyCode::PageUp => {
            if let Screen::History(h) = &mut app.screen {
                let p = h.page as isize;
                h.move_by(-p)
            }
        }
        KeyCode::Char('R') => app.fetch_history(tx),
        KeyCode::Char('D') => {
            if let Screen::History(h) = &mut app.screen {
                h.toggle_dust()
            }
        }
        KeyCode::Char('y') => {
            let txid = match &app.screen {
                Screen::History(h) => h.current().map(|e| e.txid.clone()),
                _ => None,
            };
            if let Some(t) = txid {
                app.copy_to_clipboard(&t);
            }
        }
        KeyCode::Enter | KeyCode::Char('o') => {
            // No browser is launched: opening a URL from a wallet is a way to
            // leak which addresses you own to whatever handles the click.
            let url = match &app.screen {
                Screen::History(h) => h
                    .current()
                    .map(|e| format!("{}{}", neko_tron::EXPLORER_TX, e.txid)),
                _ => None,
            };
            if let Some(u) = url {
                app.copy_to_clipboard(&u);
            }
        }
        _ => {}
    }
}

/// Put the send flow into a visible failed state.
///
/// Every abort path goes through here. A silent `return` leaves the spinner
/// running forever, which reads as "still working" when it means "gave up".
fn fail_send(app: &mut App, message: String) {
    if let Screen::Send(st) = &mut app.screen {
        st.error = None;
        st.step = crate::send::SendStep::Failed(message);
    }
}
