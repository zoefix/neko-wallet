//! Drawing. Pure functions of `App` -> `Frame`, so every screen can be snapshot
//! tested against `TestBackend` without a terminal.

use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use ratatui::Frame;

use crate::app::{App, LockReason, Screen, SetupField, MIN_COLS, MIN_ROWS};
use crate::nav::{
    HistoryState, RevealStage, SettingRow, SettingsState, WalletForm, WalletsState, CHAINS,
    SETTING_ROWS,
};
use crate::send::{self, SendState, SendStep};
use crate::theme;
use crate::ui::width::{self, Align};
use neko_i18n::{t, tf, Key};
use neko_store::repo::wallets::Origin;
use neko_vault::password::Warning;

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    if area.width < MIN_COLS || area.height < MIN_ROWS {
        draw_too_small(f, area);
        return;
    }
    // Reserve the footer row up front. Drawing hints into the same rect as the
    // bordered block paints them straight over the bottom border.
    // A failed update gets its own band above everything else. It has to
    // survive being ignored: whatever screen the user is on, this stays until
    // they act on it.
    let (banner, body_area) = match &app.startup_warning {
        Some(w) => {
            let [top, rest] =
                Layout::vertical([Constraint::Length(4), Constraint::Min(0)]).areas(area);
            (Some((top, w.clone())), rest)
        }
        None => (None, area),
    };
    let [body, bar] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(body_area);
    if let Some((top, w)) = banner {
        f.render_widget(
            Paragraph::new(w)
                .style(theme::danger())
                .wrap(Wrap { trim: false })
                .block(Block::bordered().border_type(app.border.border_type())),
            top,
        );
    }

    let hints: &str = match &app.screen {
        Screen::FirstRun { focus, .. } => {
            draw_first_run(f, body, app, *focus);
            t(Key::FirstRun_Hint)
        }
        Screen::Login { .. } => {
            draw_login(f, body, app);
            t(Key::Login_Hint)
        }
        Screen::Deriving {
            started, creating, ..
        } => {
            draw_deriving(f, body, app, started.elapsed().as_secs(), *creating);
            ""
        }
        Screen::Wallets(w) => {
            draw_wallets(f, body, app, w);
            if w.form.is_some() {
                t(Key::Form_Hint)
            } else {
                t(Key::Wallets_Hint)
            }
        }
        Screen::Chains { name, selected, .. } => {
            draw_chains(f, body, app, name, *selected);
            t(Key::Chains_Hint)
        }
        Screen::Assets {
            name,
            chain,
            address,
            selected,
            ..
        } => {
            draw_assets(f, body, app, name, *chain, address, *selected);
            t(Key::Assets_Hint)
        }
        Screen::Send(st) => {
            draw_send(f, body, app, st);
            match &st.step {
                crate::send::SendStep::Review { .. } => t(Key::Send_HintReview),
                crate::send::SendStep::Authorize { .. } => t(Key::Send_HintAuthorize),
                crate::send::SendStep::Done { .. } => t(Key::Send_HintDone),
                crate::send::SendStep::Quoting | crate::send::SendStep::Broadcasting => "",
                _ => t(Key::Send_HintEntry),
            }
        }
        Screen::Reveal { name, stage, .. } => {
            draw_reveal(f, body, app, name, stage);
            match stage {
                RevealStage::Gate { .. } => t(Key::Reveal_HintGate),
                RevealStage::Words { .. } => t(Key::Reveal_HintWords),
            }
        }
        Screen::History(h) => {
            draw_history(f, body, app, h);
            t(Key::History_Hint)
        }
        Screen::Settings(st) => {
            draw_settings(f, body, app, st);
            if st.editing.is_some() {
                t(Key::Settings_HintEditing)
            } else {
                t(Key::Settings_Hint)
            }
        }
        Screen::Locked { reason } => {
            draw_locked(f, body, app, *reason);
            t(Key::Locked_Hint)
        }
    };
    match &app.toast {
        Some(t) => footer(f, bar, &format!("  {}", t.text)),
        None => footer(f, bar, hints),
    }
}

/// One check that kills a whole class of layout bug reports.
fn draw_too_small(f: &mut Frame, area: Rect) {
    let msg = format!(
        "{}\n\n{}\n{}",
        t(Key::Resize_TooSmall),
        tf(
            Key::Resize_Need,
            &[
                ("cols", &MIN_COLS.to_string()),
                ("rows", &MIN_ROWS.to_string())
            ]
        ),
        tf(
            Key::Resize_Now,
            &[
                ("cols", &area.width.to_string()),
                ("rows", &area.height.to_string())
            ]
        ),
    );
    f.render_widget(Paragraph::new(msg).centered().style(theme::danger()), area);
}

/// Takes an owned title so callers can pass a `format!` temporary.
fn shell<'a>(app: &App, title: impl AsRef<str>) -> Block<'a> {
    Block::bordered()
        .border_type(app.border.border_type())
        .title_top(
            Line::from(format!(" {} ", title.as_ref()))
                .centered()
                .style(theme::title()),
        )
}

fn field_line<'a>(label: &str, value: String, focused: bool) -> Line<'a> {
    let marker = if focused { ">" } else { " " };
    let style = if focused {
        Style::default().fg(theme::ACCENT)
    } else {
        Style::default()
    };
    Line::from(vec![
        Span::raw(format!("{marker} ")),
        Span::styled(width::pad(label, 10, Align::Left), theme::hint()),
        Span::styled(
            format!("[ {} ]", width::pad(&value, 40, Align::Left)),
            style,
        ),
    ])
}

fn warning_text(w: &Warning) -> String {
    match w {
        Warning::TooShort { need, got } => tf(
            Key::PasswordWarning_TooShort,
            &[("need", &need.to_string()), ("got", &got.to_string())],
        ),
        Warning::CommonPassword => t(Key::PasswordWarning_Common).to_string(),
        Warning::ContainsCommonFragment(s) => tf(Key::PasswordWarning_Fragment, &[("fragment", s)]),
        Warning::RepeatedChars(n) => tf(Key::PasswordWarning_Repeated, &[("n", &n.to_string())]),
        Warning::SequentialChars(n) => {
            tf(Key::PasswordWarning_Sequential, &[("n", &n.to_string())])
        }
        Warning::ContainsYear => t(Key::PasswordWarning_Year).to_string(),
        Warning::WordPlusDigits => t(Key::PasswordWarning_WordDigits).to_string(),
        Warning::NeedsMoreVariety => t(Key::PasswordWarning_Variety).to_string(),
    }
}

fn strength_bar(app: &App) -> Line<'static> {
    if app.password.is_empty() {
        return Line::from(Span::styled(
            format!("            {}", t(Key::FirstRun_EnterPassword)),
            theme::hint(),
        ));
    }
    let s = app.strength();
    let filled = (s.score as usize).min(4);
    let colour = match s.score {
        0 | 1 => theme::DANGER,
        2 => theme::WARN,
        _ => theme::OK,
    };
    Line::from(vec![
        Span::raw("            "),
        Span::styled("#".repeat(filled * 4), Style::default().fg(colour)),
        Span::styled("-".repeat((4 - filled) * 4), theme::hint()),
        Span::styled(
            format!(
                "  {:.0} bits (need {:.0})",
                s.entropy,
                neko_vault::password::MIN_ENTROPY
            ),
            theme::hint(),
        ),
    ])
}

fn draw_first_run(f: &mut Frame, area: Rect, app: &App, focus: SetupField) {
    let block = shell(app, t(Key::App_FirstRun));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("  {}", t(Key::FirstRun_WarnKey)),
            theme::danger(),
        )),
        Line::from(Span::styled(
            format!("  {}", t(Key::FirstRun_WarnForget)),
            theme::hint(),
        )),
        Line::from(""),
        field_line(
            t(Key::Common_Email),
            app.email.display(),
            focus == SetupField::Email,
        ),
        Line::from(""),
        field_line(
            "Password",
            app.password.display(),
            focus == SetupField::Password,
        ),
        strength_bar(app),
        Line::from(""),
        field_line(
            "Confirm",
            app.confirm.display(),
            focus == SetupField::Confirm,
        ),
        Line::from(""),
    ];

    for w in app.warnings.iter().take(3) {
        lines.push(Line::from(Span::styled(
            format!("  ! {}", warning_text(w)),
            theme::hint(),
        )));
    }
    if !app.password.is_empty()
        && !app.confirm.is_empty()
        && app.password.value() != app.confirm.value()
    {
        lines.push(Line::from(Span::styled(
            format!("  ! {}", t(Key::FirstRun_Mismatch)),
            theme::danger(),
        )));
    }
    if let Some(e) = &app.error {
        lines.push(Line::from(Span::styled(
            format!("  ! {e}"),
            theme::danger(),
        )));
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_login(f: &mut Frame, area: Rect, app: &App) {
    let block = shell(app, t(Key::App_Name));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let email_focused = matches!(
        app.screen,
        Screen::Login {
            email_focused: true
        }
    );
    let mut lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            format!("   /\\_/\\   {}", t(Key::App_Name)),
            theme::title(),
        )),
        Line::from(Span::styled(
            format!("  ( o.o )  {}", t(Key::Login_Tagline)),
            theme::hint(),
        )),
        Line::from(""),
        Line::from(""),
        field_line(t(Key::Common_Email), app.email.display(), email_focused),
        Line::from(""),
        field_line(
            t(Key::Common_Password),
            app.password.display(),
            !email_focused,
        ),
        Line::from(""),
    ];
    if let Some(e) = &app.error {
        lines.push(Line::from(Span::styled(
            format!("  ! {e}"),
            theme::danger(),
        )));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_deriving(f: &mut Frame, area: Rect, app: &App, secs: u64, creating: bool) {
    let block = shell(
        app,
        if creating {
            "creating vault"
        } else {
            "unlocking"
        },
    );
    let inner = block.inner(area);
    f.render_widget(block, area);

    let p = app
        .session
        .as_ref()
        .and_then(|s| s.header().profile().ok())
        .unwrap_or(neko_vault::profile::DEFAULT);
    let lines = vec![
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled(
            format!("   {}  Deriving key... {secs}s", app.spinner()),
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        // Users read a frozen UI as a crash and a labelled wait as security.
        Line::from(Span::styled(
            format!("   {}", t(Key::Login_DerivingNote)),
            theme::hint(),
        )),
        Line::from(Span::styled(
            format!(
                "   Argon2id  m={} MiB  t={}  p={}",
                p.params.mem_kib / 1024,
                p.params.iters,
                p.params.par
            ),
            theme::hint(),
        )),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_locked(f: &mut Frame, area: Rect, app: &App, reason: LockReason) {
    let block = shell(app, t(Key::App_Locked));
    let inner = block.inner(area);
    f.render_widget(block, area);
    let why = match reason {
        LockReason::Idle => t(Key::Locked_Idle),
        LockReason::Manual => t(Key::Locked_Manual),
    };
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(format!("   {why}"), theme::title())),
        Line::from(""),
        field_line(t(Key::Common_Password), app.password.display(), true),
        Line::from(""),
    ];
    f.render_widget(Paragraph::new(lines), inner);
}

fn footer(f: &mut Frame, bar: Rect, hints: &str) {
    if hints.is_empty() {
        return;
    }
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(format!(" {hints}"), theme::hint()))),
        bar,
    );
}

// ── Wallet screens ─────────────────────────────────────────────────────────

fn draw_wallets(f: &mut Frame, area: Rect, app: &App, w: &WalletsState) {
    let block = shell(app, t(Key::App_Wallets));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![
        Line::from(""),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            Span::styled(
                width::pad(t(Key::Wallets_ColNum), 4, Align::Left),
                theme::hint(),
            ),
            Span::styled(
                width::pad(t(Key::Wallets_ColName), 22, Align::Left),
                theme::hint(),
            ),
            Span::styled(
                width::pad(t(Key::Wallets_ColType), 10, Align::Left),
                theme::hint(),
            ),
            // One figure, not four columns. Per-asset balances live one level
            // down, where the chain is unambiguous; here the list is a chooser
            // and a single total is what answers "which wallet".
            //
            // Headed USDT rather than USD on purpose: the price came from a
            // swap pool on the chain itself, so the unit really is the
            // stablecoin, and the two are not the same thing.
            Span::styled(
                width::pad(t(Key::Wallets_ColValue), VALUE_COL, Align::Right),
                theme::hint(),
            ),
            Span::styled(
                width::pad(
                    &format!("  {}", t(Key::Wallets_ColUpdated)),
                    14,
                    Align::Left,
                ),
                theme::hint(),
            ),
        ]),
    ];

    if w.items.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("   {}", t(Key::Wallets_Empty)),
            theme::hint(),
        )));
    }

    for (i, item) in w.items.iter().enumerate() {
        let selected = i == w.selected;
        let marker = if selected { " > " } else { "   " };
        let style = if selected {
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let kind = match item.origin {
            Origin::Generated => "generated",
            Origin::ImportedMnemonic => "phrase",
            Origin::ImportedPrivkey => "key only",
        };
        lines.push(Line::from(vec![
            Span::raw(marker),
            Span::styled(width::pad(&(i + 1).to_string(), 4, Align::Left), style),
            // Labels are user-supplied and may be CJK: pad by cells, not chars.
            Span::styled(
                width::pad(&width::sanitize(&item.label), 22, Align::Left),
                style,
            ),
            Span::styled(width::pad(kind, 10, Align::Left), theme::hint()),
            Span::styled(
                width::pad(&value_cell(item, &app.prices), VALUE_COL, Align::Right),
                style,
            ),
            Span::styled(
                width::pad(&format!("  {}", freshness(item)), 14, Align::Left),
                theme::hint(),
            ),
        ]));
    }

    if let Some(e) = &w.error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("   ! {e}"),
            theme::danger(),
        )));
    }
    f.render_widget(Paragraph::new(lines), inner);

    if let Some(form) = &w.form {
        draw_wallet_form(f, area, app, form);
    }
}

fn draw_wallet_form(f: &mut Frame, area: Rect, app: &App, form: &WalletForm) {
    let h = match form {
        WalletForm::ImportMnemonic { .. } => 14,
        WalletForm::ImportPrivkey { .. } => 12,
        _ => 10,
    };
    let popup = centered(area, 72, h);
    f.render_widget(Clear, popup);
    let block = shell(app, form.title());
    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let lines = match form {
        WalletForm::New { label, words } => vec![
            Line::from(""),
            field_line(t(Key::Common_Name), label.display(), true),
            Line::from(""),
            Line::from(Span::styled(
                format!("  {words}-word recovery phrase (Tab switches 12/24)"),
                theme::hint(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}", t(Key::Form_PhraseNotShown)),
                theme::hint(),
            )),
        ],
        WalletForm::ImportMnemonic {
            label,
            phrase,
            passphrase,
            focus,
        } => vec![
            Line::from(""),
            field_line(t(Key::Common_Name), label.display(), *focus == 0),
            Line::from(""),
            field_line(t(Key::Form_Phrase), phrase.display(), *focus == 1),
            Line::from(Span::styled(
                format!(
                    "            {} words",
                    phrase.value().split_whitespace().count()
                ),
                theme::hint(),
            )),
            Line::from(""),
            field_line(t(Key::Form_Passphrase), passphrase.display(), *focus == 2),
            Line::from(Span::styled(
                format!("            {}", t(Key::Form_PassphraseNote)),
                theme::hint(),
            )),
        ],
        WalletForm::ImportPrivkey { label, hex, focus } => vec![
            Line::from(""),
            field_line(t(Key::Common_Name), label.display(), *focus == 0),
            Line::from(""),
            field_line(t(Key::Form_KeyHex), hex.display(), *focus == 1),
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}", t(Key::Form_KeyNote)),
                theme::hint(),
            )),
        ],
        WalletForm::Rename { label, .. } => {
            vec![
                Line::from(""),
                field_line(t(Key::Common_Name), label.display(), true),
            ]
        }
        WalletForm::Delete { name, typed, .. } => vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("  {}", t(Key::Form_DeleteWarn)),
                theme::danger(),
            )),
            Line::from(Span::styled(
                format!("  {}", t(Key::Form_DeleteWarn2)),
                theme::hint(),
            )),
            Line::from(""),
            Line::from(Span::styled(
                format!("  Type the wallet name to confirm: {name}"),
                theme::hint(),
            )),
            Line::from(""),
            field_line(t(Key::Common_Name), typed.display(), true),
        ],
    };
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_chains(f: &mut Frame, area: Rect, app: &App, name: &str, selected: usize) {
    let block = shell(app, format!("{name} . chains"));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![Line::from("")];
    for (i, c) in CHAINS.iter().enumerate() {
        let sel = i == selected;
        let style = if sel {
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        // The chain's name alone. A ticker here was redundant - the assets
        // screen one keypress away lists what is actually held - and a list
        // this short reads better without it.
        lines.push(Line::from(vec![
            Span::raw(if sel { " > " } else { "   " }),
            Span::styled(c.label(), style),
        ]));
    }
    f.render_widget(Paragraph::new(lines), inner);
}

fn draw_assets(
    f: &mut Frame,
    area: Rect,
    app: &App,
    name: &str,
    chain: crate::nav::Chain,
    address: &str,
    selected: usize,
) {
    let block = shell(app, format!("{name} . {}", chain.label()));
    let inner = block.inner(area);
    f.render_widget(block, area);

    // The receiving address is never truncated. A wallet that shows a shortened
    // address invites people to transcribe the wrong one.
    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("   {}   ", t(Key::Assets_Address)), theme::hint()),
            Span::styled(address.to_string(), Style::default().fg(theme::ACCENT)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            Span::styled(
                width::pad(t(Key::Assets_ColAsset), 10, Align::Left),
                theme::hint(),
            ),
            Span::styled(
                width::pad(t(Key::Assets_ColBalance), 24, Align::Right),
                theme::hint(),
            ),
        ]),
    ];

    // Three states, and each one says something different: still loading, the
    // real number, or why there is no number.
    // The native coin is named by the chain, not assumed. Showing "TRX" while
    // standing on BNB Chain would be a placeholder that misidentifies the
    // asset a user is about to send.
    let native = chain.native_symbol();
    let rows: Vec<(String, String)> = match &app.balances {
        Some(b) => b.clone(),
        None if app.balances_error.is_some() => {
            vec![(native.into(), "?".into()), ("USDT".into(), "?".into())]
        }
        None => vec![
            (native.into(), format!("{} loading", app.spinner())),
            ("USDT".into(), format!("{} loading", app.spinner())),
        ],
    };

    for (i, (sym, bal)) in rows.iter().enumerate() {
        let sel = i == selected;
        let style = if sel {
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let bal_style = if app.balances.is_some() {
            style
        } else {
            theme::hint()
        };
        lines.push(Line::from(vec![
            Span::raw(if sel { " > " } else { "   " }),
            Span::styled(width::pad(sym, 10, Align::Left), style),
            Span::styled(width::pad(bal, 24, Align::Right), bal_style),
        ]));
    }

    if let Some(e) = &app.balances_error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("   ! could not read balances: {e}"),
            theme::danger(),
        )));
        lines.push(Line::from(Span::styled(
            format!("     {}", t(Key::Assets_Retry)).to_string(),
            theme::hint(),
        )));
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn draw_reveal(f: &mut Frame, area: Rect, app: &App, name: &str, stage: &RevealStage) {
    match stage {
        RevealStage::Gate { password, checking } => {
            let block = shell(app, format!("reveal recovery phrase . {name}"));
            let inner = block.inner(area);
            f.render_widget(block, area);

            let mut lines = vec![
                Line::from(""),
                Line::from(Span::styled(
                    format!("  {}", t(Key::Reveal_Owns)),
                    theme::danger(),
                )),
                Line::from(""),
                Line::from(vec![
                    Span::styled(
                        width::pad(&format!("  {}", t(Key::Reveal_DoesHeader)), 40, Align::Left),
                        theme::hint(),
                    ),
                    Span::styled(t(Key::Reveal_CannotHeader), theme::hint()),
                ]),
                Line::from(vec![
                    Span::styled(
                        width::pad(&format!("  {}", t(Key::Reveal_DoesCopy)), 40, Align::Left),
                        Style::default().fg(theme::OK),
                    ),
                    Span::styled(
                        t(Key::Reveal_CannotScreenshot),
                        Style::default().fg(theme::WARN),
                    ),
                ]),
                Line::from(vec![
                    Span::styled(
                        width::pad(
                            &format!("  {}", t(Key::Reveal_DoesAltscreen)),
                            40,
                            Align::Left,
                        ),
                        Style::default().fg(theme::OK),
                    ),
                    Span::styled(
                        t(Key::Reveal_CannotShoulder),
                        Style::default().fg(theme::WARN),
                    ),
                ]),
                Line::from(vec![
                    Span::styled(
                        width::pad(
                            &format!("  {}", t(Key::Reveal_DoesAltscreen2)),
                            40,
                            Align::Left,
                        ),
                        Style::default().fg(theme::OK),
                    ),
                    Span::styled(
                        t(Key::Reveal_CannotSelect),
                        Style::default().fg(theme::WARN),
                    ),
                ]),
                Line::from(vec![
                    Span::styled(
                        width::pad(
                            &format!("  {}", t(Key::Reveal_DoesOneWord)),
                            40,
                            Align::Left,
                        ),
                        Style::default().fg(theme::OK),
                    ),
                    Span::styled(
                        t(Key::Reveal_CannotCapture),
                        Style::default().fg(theme::WARN),
                    ),
                ]),
                Line::from(vec![
                    Span::styled(
                        width::pad(
                            &format!("  {}", t(Key::Reveal_DoesAutohide)),
                            40,
                            Align::Left,
                        ),
                        Style::default().fg(theme::OK),
                    ),
                    Span::styled(
                        t(Key::Reveal_CannotMalware),
                        Style::default().fg(theme::WARN),
                    ),
                ]),
                Line::from(""),
            ];

            for warn in environment_warnings() {
                lines.push(Line::from(Span::styled(
                    format!("  !! {warn}"),
                    theme::danger(),
                )));
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("  {}", t(Key::Reveal_GetPen)),
                theme::title(),
            )));
            lines.push(Line::from(""));
            if *checking {
                lines.push(Line::from(Span::styled(
                    format!("   {}  verifying...", app.spinner()),
                    Style::default().fg(theme::ACCENT),
                )));
            } else {
                lines.push(field_line(
                    t(Key::Common_Password),
                    password.display(),
                    true,
                ));
            }
            f.render_widget(Paragraph::new(lines), inner);
        }
        RevealStage::Words {
            words,
            cursor,
            show_all,
            hide_at,
        } => {
            let left = hide_at
                .saturating_duration_since(std::time::Instant::now())
                .as_secs();
            let block = shell(
                app,
                format!(
                    "recovery phrase . {name} . hides in {left}s . word {} of {}",
                    cursor + 1,
                    words.len()
                ),
            );
            let inner = block.inner(area);
            f.render_widget(block, area);

            let mut lines = vec![Line::from("")];
            // Four per row, masked unless revealed. Showing one word at a time
            // means a screenshot leaks 1 of 12, not all of them.
            for row in words.chunks(4).enumerate().map(|(r, c)| (r, c.to_vec())) {
                let (r, chunk) = row;
                let mut spans = vec![Span::raw("   ")];
                for (c, word) in chunk.iter().enumerate() {
                    let idx = r * 4 + c;
                    let visible = *show_all || idx == *cursor;
                    let text = if visible {
                        word.clone()
                    } else {
                        "######".to_string()
                    };
                    let style = if idx == *cursor {
                        Style::default()
                            .fg(theme::ACCENT)
                            .add_modifier(Modifier::BOLD)
                    } else if visible {
                        Style::default()
                    } else {
                        theme::hint()
                    };
                    spans.push(Span::styled(format!("{:>3} ", idx + 1), theme::hint()));
                    spans.push(Span::styled(width::pad(&text, 14, Align::Left), style));
                }
                lines.push(Line::from(spans));
                lines.push(Line::from(""));
            }
            lines.push(Line::from(Span::styled(
                format!(
                    "   Write word {} down, then press -> for the next.",
                    cursor + 1
                ),
                theme::hint(),
            )));
            f.render_widget(Paragraph::new(lines), inner);
        }
    }
}

/// Best-effort detection of things that will capture the screen. Labelled as
/// heuristics because that is what they are.
pub fn environment_warnings() -> Vec<String> {
    let mut out = Vec::new();
    if std::env::var_os("TMUX").is_some() {
        out.push(t(Key::RevealEnv_Tmux).to_string());
    }
    if matches!(std::env::var("TERM").as_deref(), Ok(t) if t.starts_with("screen")) {
        out.push(t(Key::RevealEnv_Screen).to_string());
    }
    if crate::clipboard::Clipboard::is_remote_session() {
        out.push(t(Key::RevealEnv_Ssh).to_string());
    }
    if std::env::var_os("ASCIINEMA_REC").is_some() {
        out.push(t(Key::RevealEnv_Recording).to_string());
    }
    out
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    }
}

fn draw_send(f: &mut Frame, area: Rect, app: &App, st: &SendState) {
    let title = format!("send {} . {}", st.asset_label, st.wallet_name);
    let block = shell(app, title);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![Line::from("")];

    match &st.step {
        SendStep::Recipient | SendStep::EnterAmount => {
            let on_amount = matches!(st.step, SendStep::EnterAmount);
            lines.push(field_line(t(Key::Send_To), st.to.display(), !on_amount));
            if let Some(e) = st.recipient_error() {
                lines.push(Line::from(Span::styled(
                    format!("            ! {e}"),
                    theme::danger(),
                )));
            }
            lines.push(Line::from(""));
            lines.push(field_line(
                &format!("Amount ({})", st.asset_label),
                st.amount.display(),
                on_amount,
            ));
            if let Some(e) = st.amount_error() {
                lines.push(Line::from(Span::styled(
                    format!("            ! {e}"),
                    theme::danger(),
                )));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("   From  {}", st.from),
                theme::hint(),
            )));
        }

        SendStep::Quoting => {
            lines.push(Line::from(Span::styled(
                format!(
                    "   {}  asking the chain what this will cost...",
                    app.spinner()
                ),
                Style::default().fg(theme::ACCENT),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("   {}", t(Key::Send_QuotingNote)),
                theme::hint(),
            )));
        }

        SendStep::Review {
            req, quote, typed, ..
        } => {
            let dest = req.to.to_string();
            let (head, mid, tail) = send::split_address(&dest);

            lines.push(Line::from(Span::styled(
                format!("   {}", t(Key::Send_Verify)),
                theme::danger(),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(format!("   {}     ", t(Key::Send_From)), theme::hint()),
                Span::raw(req.from.to_string()),
            ]));
            lines.push(Line::from(vec![
                Span::styled(format!("   {}       ", t(Key::Send_To)), theme::hint()),
                Span::styled(
                    head.clone(),
                    Style::default()
                        .fg(theme::WARN)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(mid, theme::hint()),
                Span::styled(
                    tail.clone(),
                    Style::default()
                        .fg(theme::WARN)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled("            ", theme::hint()),
                Span::styled(
                    "^".repeat(head.chars().count()),
                    Style::default().fg(theme::WARN),
                ),
                Span::styled(
                    " ".repeat(dest.chars().count() - head.chars().count() - tail.chars().count()),
                    theme::hint(),
                ),
                Span::styled(
                    "^".repeat(tail.chars().count()),
                    Style::default().fg(theme::WARN),
                ),
                Span::styled(format!("  {}", t(Key::Send_CheckThese)), theme::hint()),
            ]));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(format!("   {}   ", t(Key::Send_Amount)), theme::hint()),
                Span::styled(
                    format!("{} {}", req.amount.to_display_string_full(), st.asset_label),
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));

            if let Some(q) = quote {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("   {}", t(Key::Send_Fee)),
                    theme::hint(),
                )));

                match q {
                    crate::send::FeeQuote::Tron(q) => {
                        lines.push(resource_line(
                            t(Key::Send_Energy),
                            q.energy_needed(),
                            q.energy_available().zip(q.energy_limit()),
                            q.energy_shortfall(),
                            q.energy_burn(),
                        ));
                        if q.energy_penalty > 0 {
                            lines.push(Line::from(Span::styled(
                                format!(
                                    "                {} base + {} dynamic-energy surcharge",
                                    group(q.energy_base),
                                    group(q.energy_penalty)
                                ),
                                theme::hint(),
                            )));
                        }
                        lines.push(resource_line(
                            t(Key::Send_Bandwidth),
                            q.bandwidth_needed,
                            q.bandwidth_available().zip(q.bandwidth_limit()),
                            q.bandwidth_shortfall(),
                            q.bandwidth_burn(),
                        ));
                    }
                    crate::send::FeeQuote::Bsc(b) => {
                        // No allowance to draw down here, so there is nothing
                        // to show as "needed against held": gas is simply
                        // bought. What matters instead is whether the BNB
                        // balance covers it, because a wallet holding only
                        // USDT cannot move that USDT.
                        lines.push(Line::from(vec![
                            Span::styled(format!("     {}      ", t(Key::Send_Gas)), theme::hint()),
                            Span::raw(format!(
                                "{} units x {} gwei",
                                group(b.gas_limit as i64),
                                neko_core::Amount::new(b.gas_price as i128, 9)
                                    .to_display_string_trim(crate::chain::BALANCE_FRAC)
                            )),
                        ]));
                        lines.push(Line::from(vec![
                            Span::styled(
                                format!("     {}   ", t(Key::Send_BnbBalance)),
                                theme::hint(),
                            ),
                            match b.bnb_balance {
                                Some(v) => Span::raw(format!(
                                    "{} BNB",
                                    neko_core::Amount::new(v as i128, 18)
                                        .to_display_string_trim(crate::chain::BALANCE_FRAC)
                                )),
                                // A failed lookup is not zero, and must not be
                                // rendered as though it were.
                                None => {
                                    Span::styled(t(Key::Common_Unknown).to_string(), theme::hint())
                                }
                            },
                        ]));
                        if b.affordable() == Some(false) {
                            let short = b.shortfall().unwrap_or(neko_core::Amount::new(0, 18));
                            lines.push(Line::from(Span::styled(
                                format!(
                                    "     {}",
                                    tf(
                                        Key::Send_NeedMoreBnb,
                                        &[
                                            (
                                                "amount",
                                                &short.to_display_string_trim(
                                                    crate::chain::BALANCE_FRAC,
                                                ),
                                            )
                                        ]
                                    )
                                ),
                                Style::default().fg(theme::DANGER),
                            )));
                        }
                    }
                }

                let total = q.total();
                let chain = st.asset.chain();
                let unit = chain.native_symbol();
                // What the fee actually costs. Without this the line reads
                // "~0.00000311 BNB", which nobody can weigh against what they
                // are sending - and weighing it is the entire point.
                let priced = fee_in_usdt(total, chain, &app.prices);
                // TRON literally destroys the TRX that covers a shortfall;
                // BNB Chain pays gas to a validator. Same column, different
                // fact, so the word differs.
                let verb = match q {
                    crate::send::FeeQuote::Tron(_) => " burned",
                    crate::send::FeeQuote::Bsc(_) => "",
                };
                let mut total_line = vec![
                    Span::styled(format!("     {}      ", t(Key::Send_Total)), theme::hint()),
                    if q.is_free() {
                        Span::styled(
                            t(Key::Send_Free).to_string(),
                            Style::default().fg(theme::OK),
                        )
                    } else {
                        let shown = total.to_display_string_trim(crate::chain::BALANCE_FRAC);
                        let lead = if q.is_upper_bound() { "at most " } else { "~" };
                        Span::styled(
                            format!("{lead}{shown} {unit}{verb}"),
                            Style::default()
                                .fg(theme::WARN)
                                .add_modifier(Modifier::BOLD),
                        )
                    },
                ];
                // A free transfer costs nothing to price, and a fee whose price
                // is not known yet says nothing rather than implying zero.
                if !q.is_free() {
                    if let Some(p) = priced {
                        total_line.push(Span::styled(format!("   {p}"), theme::hint()));
                    }
                }
                lines.push(Line::from(total_line));

                // Never let a failed lookup masquerade as a fact about the
                // account. Without an API key these calls hit the public rate
                // limit intermittently, which is exactly when a confident but
                // wrong number would do the most damage.
                if q.is_upper_bound() {
                    lines.push(Line::from(Span::styled(
                        format!("     !  {}", t(Key::Send_ResourcesUnknown)),
                        theme::danger(),
                    )));
                    lines.push(Line::from(Span::styled(
                        format!("        {}", t(Key::Send_ResourcesUnknown2)).to_string(),
                        theme::warn(),
                    )));
                    lines.push(Line::from(Span::styled(
                        format!("        {}", t(Key::Send_ResourcesUnknown3)),
                        theme::warn(),
                    )));
                }

                if let crate::send::FeeQuote::Tron(q) = q {
                    lines.push(Line::from(Span::styled(
                        format!(
                            "                {}",
                            tf(
                                if q.prices_known() {
                                    Key::Send_PricesChain
                                } else {
                                    Key::Send_PricesFallback
                                },
                                &[
                                    ("energy", &q.sun_per_energy().to_string()),
                                    ("bandwidth", &q.sun_per_bandwidth().to_string()),
                                ]
                            )
                        ),
                        theme::hint(),
                    )));

                    if q.recipient_is_new {
                        lines.push(Line::from(Span::styled(
                            format!(
                                "     !  This address has never held {}. Creating its balance",
                                st.asset_label
                            ),
                            theme::warn(),
                        )));
                        lines.push(Line::from(Span::styled(
                            format!("        {}", t(Key::Send_FirstTime2)),
                            theme::warn(),
                        )));
                    }
                }
            }

            if let Some(similar) = st.lookalike_warning() {
                lines.push(Line::from(""));
                lines.push(Line::from(Span::styled(
                    format!("   !! {}", t(Key::Send_Lookalike)),
                    theme::danger(),
                )));
                lines.push(Line::from(Span::styled(
                    format!("      {similar}"),
                    theme::warn(),
                )));
                lines.push(Line::from(Span::styled(
                    format!("      {}", t(Key::Send_LookalikeCopied)),
                    theme::danger(),
                )));
            }

            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!(
                    "   Type the LAST {} characters of the destination to confirm:",
                    send::CONFIRM_CHARS
                ),
                theme::title(),
            )));
            let ok = st.confirmation_satisfied();
            lines.push(Line::from(vec![
                Span::raw("            "),
                Span::styled(
                    format!("[ {:<8} ]", typed.display()),
                    if ok {
                        Style::default().fg(theme::OK)
                    } else {
                        Style::default().fg(theme::ACCENT)
                    },
                ),
                Span::styled(
                    format!(
                        "  {}/{}",
                        typed.value().chars().count(),
                        send::CONFIRM_CHARS
                    ),
                    theme::hint(),
                ),
                Span::styled(
                    if ok { "  matches" } else { "" },
                    Style::default().fg(theme::OK),
                ),
            ]));
        }

        SendStep::Authorize {
            req,
            password,
            checking,
            ..
        } => {
            lines.push(Line::from(Span::styled(
                format!("   {}", t(Key::Send_LastStep)),
                theme::title(),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(format!("   {}  ", t(Key::Send_Sending)), theme::hint()),
                Span::styled(
                    format!("{} {}", req.amount.to_display_string_full(), st.asset_label),
                    Style::default()
                        .fg(theme::ACCENT)
                        .add_modifier(Modifier::BOLD),
                ),
            ]));
            lines.push(Line::from(vec![
                Span::styled(format!("   {}       ", t(Key::Send_To)), theme::hint()),
                Span::raw(req.to.to_string()),
            ]));
            lines.push(Line::from(""));
            if *checking {
                lines.push(Line::from(Span::styled(
                    format!("   {}  verifying...", app.spinner()),
                    Style::default().fg(theme::ACCENT),
                )));
            } else {
                lines.push(field_line(
                    t(Key::Common_Password),
                    password.display(),
                    true,
                ));
            }
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("   {}", t(Key::Send_NothingSigned)),
                theme::hint(),
            )));
        }

        SendStep::Broadcasting => {
            lines.push(Line::from(Span::styled(
                format!("   {}  broadcasting...", app.spinner()),
                Style::default().fg(theme::ACCENT),
            )));
        }

        SendStep::Done { txid, explorer } => {
            lines.push(Line::from(Span::styled(
                format!("   {}", t(Key::Send_Accepted)),
                Style::default().fg(theme::OK).add_modifier(Modifier::BOLD),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(vec![
                Span::styled(format!("   {}   ", t(Key::Send_Txid)), theme::hint()),
                Span::raw(txid.clone()),
            ]));
            lines.push(Line::from(Span::styled(
                format!("   {explorer}"),
                theme::hint(),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("   {}", t(Key::Send_NotFinal)),
                theme::hint(),
            )));
        }

        SendStep::Failed(e) => {
            lines.push(Line::from(Span::styled(
                format!("   {}", t(Key::Send_Failed)),
                theme::danger(),
            )));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(format!("   {e}"), theme::hint())));
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("   {}", t(Key::Send_NothingSent)),
                theme::hint(),
            )));
        }
    }

    if let Some(e) = &st.error {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("   ! {e}"),
            theme::danger(),
        )));
    }
    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// An indented hint, wrapped to fit inside the block.
///
/// Wrapped here rather than by ratatui's own `Wrap`: that measures in a way
/// that lets a wide grapheme on the boundary spill one cell past the area, and
/// inside a bordered block the cell it spills onto is the border. Japanese
/// prose hits that boundary routinely, and the result is a hole in the frame
/// that no width assertion catches, because the row is still exactly as wide as
/// the terminal.
fn hint_lines(text: &str, inner_width: u16) -> Vec<Line<'static>> {
    const INDENT: usize = 3;
    let budget = (inner_width as usize).saturating_sub(INDENT);
    width::wrap(text, budget)
        .into_iter()
        .map(|l| Line::from(Span::styled(format!("{:INDENT$}{l}", ""), theme::hint())))
        .collect()
}

fn draw_settings(f: &mut Frame, area: Rect, app: &App, st: &SettingsState) {
    let block = shell(app, t(Key::App_Settings));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![Line::from(""), Line::from("")];

    for (i, row) in SETTING_ROWS.iter().enumerate() {
        let sel = i == st.selected;
        let style = if sel {
            Style::default()
                .fg(theme::ACCENT)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let value = match (sel, st.editing.as_ref()) {
            (true, Some(field)) => format!("[ {} ]", field.display()),
            _ => app.setting_value(*row),
        };
        lines.push(Line::from(vec![
            Span::raw(if sel { " > " } else { "   " }),
            Span::styled(width::pad(row.label(), 20, Align::Left), style),
            Span::styled(width::pad(&value, 46, Align::Left), style),
        ]));
        // Show every language in its own script, so the picker is readable no
        // matter which one is currently active.
        if sel && matches!(row, SettingRow::Language) && st.editing.is_none() {
            let names: Vec<String> = neko_i18n::LOCALES
                .iter()
                .map(|l| {
                    if *l == app.locale {
                        format!("[{}]", l.endonym())
                    } else {
                        l.endonym().to_string()
                    }
                })
                .collect();
            lines.push(Line::from(Span::styled(
                format!("                       < {} >", names.join(" . ")),
                theme::hint(),
            )));
        }
    }

    lines.push(Line::from(""));
    if app.api_key.is_none() {
        lines.extend(hint_lines(t(Key::Settings_ApiKeyNote), inner.width));
        lines.extend(hint_lines(t(Key::Settings_ApiKeyNote2), inner.width));
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// History table column widths.
///
/// The header and the data rows read the *same* constants. Duplicating the
/// numbers is how `out` ended up glued to the asset symbol: the header padded
/// the direction to 4 while the row padded it to 3, so every column below the
/// header sat one cell to the left and the shortest gap closed entirely.
mod col {
    /// `YYYY-MM-DD HH:MM` plus a separating space.
    pub const WHEN: usize = 17;
    /// `out` is three characters, so 4 guarantees a gap after the longest value.
    pub const DIR: usize = 4;
    pub const ASSET: usize = 6;
    /// Room for the whole USDT supply (`100,000,000,000.000000`) plus a sign,
    /// so a real amount is never truncated on screen.
    pub const AMOUNT: usize = 24;
    pub const STATUS: usize = 10;
}

/// The list shows no addresses at all.
///
/// Any head..tail abbreviation makes a poisoned lookalike indistinguishable
/// from the address it imitates, and a full 34-character address does not fit
/// beside the other columns at 80 terminal columns. So the table carries only
/// what is unambiguous, and the selected row's counterparty and txid are
/// printed in full underneath it.
fn draw_history(f: &mut Frame, area: Rect, app: &App, h: &HistoryState) {
    let block = shell(app, t(Key::App_History));
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines = vec![
        Line::from(""),
        Line::from(vec![
            Span::styled(format!("   {}   ", t(Key::Assets_Address)), theme::hint()),
            Span::styled(h.address.clone(), Style::default().fg(theme::ACCENT)),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::raw("   "),
            Span::styled(
                width::pad(t(Key::History_ColWhen), col::WHEN, Align::Left),
                theme::hint(),
            ),
            Span::styled(width::pad("", col::DIR, Align::Left), theme::hint()),
            Span::styled(
                width::pad(t(Key::History_ColAsset), col::ASSET, Align::Left),
                theme::hint(),
            ),
            // Wide enough for the entire USDT supply plus a sign. A truncated
            // amount is worse than no amount: it silently misinforms.
            Span::styled(
                width::pad(t(Key::History_ColAmount), col::AMOUNT, Align::Right),
                theme::hint(),
            ),
            Span::styled(
                width::pad(
                    &format!("  {}", t(Key::History_ColStatus)),
                    col::STATUS,
                    Align::Left,
                ),
                theme::hint(),
            ),
        ]),
    ];

    match (&h.entries, &h.error) {
        (_, Some(e)) => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("   ! {e}"),
                theme::danger(),
            )));
            lines.push(Line::from(Span::styled(
                format!("     {}", t(Key::Assets_Retry)),
                theme::hint(),
            )));
        }
        (None, None) => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("   {}  loading history...", app.spinner()),
                Style::default().fg(theme::ACCENT),
            )));
        }
        (Some(_), None) if h.is_empty() && h.dust_count() > 0 => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!(
                    "   Nothing here except {} suspected address-poisoning transfer(s).",
                    h.dust_count()
                ),
                theme::hint(),
            )));
            lines.push(Line::from(Span::styled(
                format!("   {}", t(Key::History_DustHint)),
                theme::hint(),
            )));
        }
        (Some(entries), None) if entries.is_empty() => {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                format!("   {}", t(Key::History_Empty)),
                theme::hint(),
            )));
        }
        (Some(entries), None) => {
            let _ = entries;
            let rows = h.visible();
            for (i, e) in rows.iter().enumerate().skip(h.offset).take(h.page) {
                let sel = i == h.selected;
                let incoming = e.direction == neko_tron::Direction::In;
                let sign = if incoming { "+" } else { "-" };
                let dir_style = if incoming {
                    Style::default().fg(theme::OK)
                } else {
                    Style::default().fg(theme::WARN)
                };
                let amount = neko_core::Amount::new(e.amount, e.decimals)
                    .to_display_string_trim(crate::chain::BALANCE_FRAC);
                let (status, status_style) = if e.is_dust() {
                    (t(Key::History_StatusDust), theme::danger())
                } else {
                    match e.status {
                        neko_tron::TxStatus::Success => {
                            (t(Key::History_StatusOk), Style::default().fg(theme::OK))
                        }
                        neko_tron::TxStatus::Failed => {
                            (t(Key::History_StatusFailed), theme::danger())
                        }
                        neko_tron::TxStatus::Pending => (
                            t(Key::History_StatusPending),
                            Style::default().fg(theme::WARN),
                        ),
                    }
                };
                lines.push(Line::from(vec![
                    Span::raw(if sel { " > " } else { "   " }),
                    Span::styled(
                        width::pad(&fmt_time(e.block_ts), col::WHEN, Align::Left),
                        theme::hint(),
                    ),
                    Span::styled(
                        width::pad(
                            if incoming {
                                t(Key::History_DirectionIn)
                            } else {
                                t(Key::History_DirectionOut)
                            },
                            col::DIR,
                            Align::Left,
                        ),
                        dir_style,
                    ),
                    Span::styled(
                        width::pad(&e.symbol, col::ASSET, Align::Left),
                        Style::default(),
                    ),
                    Span::styled(
                        width::pad(&format!("{sign}{amount}"), col::AMOUNT, Align::Right),
                        dir_style,
                    ),
                    Span::styled(
                        width::pad(&format!("  {status}"), 10, Align::Left),
                        status_style,
                    ),
                ]));
            }
            lines.push(Line::from(""));
            let mut summary = format!("   {} of {}", h.selected + 1, rows.len());
            if h.dust_count() > 0 {
                summary.push_str(&format!(
                    "   ({} suspected poisoning transfer(s) {}, D toggles)",
                    h.dust_count(),
                    if h.show_dust { "shown" } else { "hidden" }
                ));
            }
            lines.push(Line::from(Span::styled(summary, theme::hint())));
            if let Some(e) = h.current() {
                lines.push(Line::from(vec![
                    Span::styled(format!("   {}  ", t(Key::Send_Txid)), theme::hint()),
                    Span::raw(e.txid.clone()),
                ]));
                lines.push(Line::from(vec![
                    Span::styled(format!("   {}  ", t(Key::History_FromTo)), theme::hint()),
                    Span::raw(e.counterparty.clone()),
                ]));
            }
        }
    }

    f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

/// `YYYY-MM-DD HH:MM` in UTC. Deliberately not a locale format: in a wallet an
/// unambiguous timestamp beats a familiar one.
fn fmt_time(ms: i64) -> String {
    if ms <= 0 {
        return "-".into();
    }
    let secs = ms / 1000;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    let (h, m) = (tod / 3600, (tod % 3600) / 60);

    // Civil-from-days (Howard Hinnant's algorithm), so no date dependency.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mth = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mth <= 2 { y + 1 } else { y };
    format!("{y:04}-{mth:02}-{d:02} {h:02}:{m:02}")
}

/// Width of the estimated-value column.
const VALUE_COL: usize = 18;

/// The fee, in something a person can weigh.
///
/// `~0.00000311 BNB` answers "how much gas" and not "how much money", and the
/// question on this screen is the second one. The unit is USDT rather than
/// dollars for the reason `neko_core::value` sets out: the quote came from a
/// swap pool, not a currency market, and the two are not the same thing.
///
/// Sub-cent fees come out as `<0.01` rather than `0.00`, because a fee shown as
/// zero is a fee somebody stops checking.
///
/// `None` when the price is not known, so the line stays silent instead of
/// implying the transfer is free.
fn fee_in_usdt(
    total: neko_core::Amount,
    chain: neko_core::ChainId,
    prices: &neko_core::Prices,
) -> Option<String> {
    let price = prices.native(chain)?;
    let v = neko_core::value::value_of(total.raw, total.decimals, price)?;
    let shown = neko_core::Amount::new(v, neko_core::PRICE_SCALE).to_display_string_trim(2);
    Some(format!("\u{2248} {shown} USDT"))
}

/// A wallet's total, or a placeholder saying why there isn't one.
///
/// Three states, and each says something different. A dash means no price has
/// been fetched yet; `?` means a price is missing for something this wallet
/// actually holds, so the total would understate it - and understating is the
/// direction that makes somebody think they can afford a transfer they cannot.
fn value_cell(item: &neko_core::WalletView, prices: &neko_core::Prices) -> String {
    if prices.is_empty() {
        return "-".to_string();
    }
    // Nothing has ever been fetched for this wallet. `total` of no holdings is
    // zero, and zero would claim the wallet is empty - which is a different
    // statement from "not looked yet", and the wrong one to make about
    // somebody's funds.
    if item.assets.iter().all(|(_, a)| a.rows.is_empty()) {
        return "-".to_string();
    }
    let holdings: Vec<_> = item
        .assets
        .iter()
        .flat_map(|(chain, a)| {
            a.rows
                .iter()
                .map(move |r| (*chain, r.symbol.as_str(), r.amount, r.decimals))
        })
        .collect();
    match neko_core::value::total(holdings, prices) {
        Some(v) => v.to_display_string_max(2),
        None => "?".to_string(),
    }
}

/// How old the cached figure is.
///
/// A stale balance presented as current is a lie; a stale balance labelled as
/// stale is useful. So the age is always shown, never hidden.
fn freshness(item: &neko_core::WalletView) -> String {
    // The *oldest* of the chains that actually have a figure, not the newest:
    // the row shows several, so it is only as current as its stalest number.
    //
    // A chain with nothing cached is skipped rather than making the whole row
    // read "never" - its own cell already shows "-", and the numbers that are
    // on screen do have an age worth stating.
    let Some(ts) = item.assets.iter().filter_map(|(_, a)| a.updated_at).min() else {
        return "never".into();
    };
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let age = (now - ts).max(0);
    match age {
        0..=59 => "just now".into(),
        60..=3599 => format!("{}m ago", age / 60),
        3600..=86_399 => format!("{}h ago", age / 3600),
        _ => format!("{}d ago", age / 86_400),
    }
}

/// One resource row: what the transfer needs, what the account has, and what
/// the shortfall costs.
///
/// `available == None` means the lookup failed. That is rendered as "unknown",
/// never as a zero: claiming an account holds nothing when we simply could not
/// ask is both false and expensive.
fn resource_line<'a>(
    label: &str,
    needed: i64,
    available: Option<(i64, i64)>,
    shortfall: i64,
    burn: neko_core::Amount,
) -> Line<'a> {
    let mut spans = vec![
        Span::styled(
            format!("     {}", width::pad(label, 11, Align::Left)),
            theme::hint(),
        ),
        Span::raw(width::pad(&group(needed), 10, Align::Right)),
        Span::styled(format!("  {}  ", t(Key::Send_Needed)), theme::hint()),
    ];
    match available {
        None => {
            spans.push(Span::styled(
                width::pad("unknown", 16, Align::Right),
                theme::danger(),
            ));
            spans.push(Span::styled(
                format!("  {}  ", t(Key::Send_YouHave)),
                theme::hint(),
            ));
            spans.push(Span::styled(
                format!("assuming none -> up to {} TRX", burn.to_exact_string()),
                theme::danger(),
            ));
        }
        Some((have, limit)) => {
            let covered = shortfall == 0;
            // Both figures, like every TRON explorer shows them: the available
            // side regenerates continuously, so on its own it looks wrong
            // against any reading taken a few seconds apart.
            spans.push(Span::styled(
                width::pad(
                    &format!("{}/{}", group(have), group(limit)),
                    16,
                    Align::Right,
                ),
                if covered {
                    Style::default().fg(theme::OK)
                } else {
                    Style::default().fg(theme::WARN)
                },
            ));
            spans.push(Span::styled(
                format!("  {}  ", t(Key::Send_YouHave)),
                theme::hint(),
            ));
            spans.push(if covered {
                Span::styled(
                    t(Key::Send_Covered).to_string(),
                    Style::default().fg(theme::OK),
                )
            } else {
                Span::styled(
                    format!(
                        "short {} -> {} TRX",
                        group(shortfall),
                        burn.to_exact_string()
                    ),
                    Style::default().fg(theme::WARN),
                )
            });
        }
    }
    Line::from(spans)
}

/// Thousands separators for a plain count.
fn group(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}
