//! Renders every screen against a headless backend.
//!
//! What this actually catches: a CJK wallet name or a long address shearing a
//! column, a panic in a layout path, and secrets reaching the screen buffer.

use neko_tui::app::{App, LockReason, Screen, SetupField};
use neko_tui::nav::WalletsState;
use neko_tui::render;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

fn screen_text(app: &App, w: u16, h: u16) -> String {
    // These assertions are written against the English strings; the app
    // otherwise follows the OS language, which is not the same on every
    // machine that runs the suite.
    neko_i18n::set_locale(neko_i18n::Locale::English);
    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
    term.draw(|f| render::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    // A wide char occupies TWO cells in the buffer: the symbol, then a blank
    // continuation cell. Concatenating every cell would count it as 3 columns.
    // Advance by the symbol's own width to reconstruct the logical line.
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

fn app_with(screen: Screen) -> App {
    let mut a = App::new(std::path::PathBuf::from("/tmp/neko-test.db"));
    a.screen = screen;
    a
}

#[test]
fn every_screen_renders_at_common_sizes() {
    let screens = || {
        vec![
            Screen::FirstRun {
                focus: SetupField::Email,
                warned: false,
            },
            Screen::FirstRun {
                focus: SetupField::Password,
                warned: true,
            },
            Screen::Login {
                email_focused: true,
            },
            Screen::Login {
                email_focused: false,
            },
            Screen::Locked {
                reason: LockReason::Idle,
            },
            Screen::Wallets(WalletsState::new(Vec::new())),
        ]
    };
    for (w, h) in [(80, 24), (120, 40), (200, 60)] {
        for s in screens() {
            let mut app = app_with(s);
            app.email.push('z');
            app.password.push('x');
            let out = screen_text(&app, w, h);
            assert_eq!(out.lines().count(), h as usize);
            for line in out.lines() {
                assert_eq!(
                    unicode_width::UnicodeWidthStr::width(line),
                    w as usize,
                    "line does not fill exactly {w} cells at {w}x{h}:\n{line:?}"
                );
            }
        }
    }
}

/// Below the floor we draw one panel instead of a broken layout.
#[test]
fn tiny_terminal_shows_a_resize_prompt() {
    let app = app_with(Screen::Login {
        email_focused: true,
    });
    let out = screen_text(&app, 40, 10);
    assert!(out.contains("too small"), "got:\n{out}");
}

/// The password must never reach the screen buffer in clear text.
#[test]
fn passwords_are_masked_in_the_rendered_buffer() {
    let mut app = app_with(Screen::Login {
        email_focused: false,
    });
    for c in "correct horse battery staple".chars() {
        app.password.push(c);
    }
    let out = screen_text(&app, 80, 24);
    assert!(
        !out.contains("correct"),
        "password leaked into the buffer:\n{out}"
    );
    assert!(out.contains("****"), "password field is not masked:\n{out}");
}

/// A CJK email must not shear the field box.
#[test]
fn cjk_input_does_not_break_the_layout() {
    let mut app = app_with(Screen::Login {
        email_focused: true,
    });
    for c in "钱包日本用ウォレット@例え.jp".chars() {
        app.email.push(c);
    }
    let out = screen_text(&app, 80, 24);
    for line in out.lines() {
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(line),
            80,
            "sheared line: {line:?}"
        );
    }
}

/// The zero-recovery warning is the single most important string in the app.
#[test]
fn first_run_states_the_zero_recovery_consequence() {
    let app = app_with(Screen::FirstRun {
        focus: SetupField::Email,
        warned: false,
    });
    let out = screen_text(&app, 80, 24);
    assert!(
        out.contains("no recovery"),
        "missing the irreversibility warning:\n{out}"
    );
}

/// A frozen UI reads as a crash; a labelled wait reads as security.
#[test]
fn deriving_screen_explains_the_delay() {
    let app = app_with(Screen::Deriving {
        req: neko_tui::event::ReqId(1),
        started: std::time::Instant::now(),
        creating: false,
    });
    let out = screen_text(&app, 80, 24);
    assert!(out.contains("slow on purpose"), "got:\n{out}");
    assert!(out.contains("Argon2id"), "got:\n{out}");
}

/// Regression: the footer used to be drawn into the same rect as the bordered
/// block, painting the key hints straight over the bottom border. The
/// fills-exactly-N-cells assertion could not catch it, because a collided row
/// is still full width.
#[test]
fn footer_does_not_collide_with_the_border() {
    for screen in [
        Screen::Wallets(WalletsState::new(Vec::new())),
        Screen::Login {
            email_focused: true,
        },
        Screen::FirstRun {
            focus: SetupField::Email,
            warned: false,
        },
        Screen::Locked {
            reason: LockReason::Idle,
        },
    ] {
        let out = screen_text(&app_with(screen), 80, 24);
        let lines: Vec<&str> = out.lines().collect();

        let hint_row: Vec<char> = lines[lines.len() - 1].chars().collect();
        let border_row: Vec<char> = lines[lines.len() - 2].chars().collect();

        // Only the box-drawing block is checked. ASCII '-' and '|' cannot be
        // used as markers here: "Ctrl-L" legitimately contains a hyphen.
        assert!(
            !hint_row.iter().any(|c| is_box_drawing(*c)),
            "hints share a row with the border: {:?}",
            lines[lines.len() - 1]
        );
        assert!(
            is_box_drawing(border_row[0]) && is_box_drawing(border_row[border_row.len() - 1]),
            "the block's bottom border was overwritten: {:?}",
            lines[lines.len() - 2]
        );
        assert!(
            border_row.iter().filter(|c| is_box_drawing(**c)).count() >= 78,
            "bottom border looks truncated: {:?}",
            lines[lines.len() - 2]
        );
    }
}

fn is_box_drawing(c: char) -> bool {
    ('\u{2500}'..='\u{257F}').contains(&c)
}

/// Coverage gap: "every line is exactly N cells" is satisfied even when a
/// border column is missing, because the row is still padded to full width.
/// Assert the frame's left and right edges are actually drawn.
#[test]
fn block_borders_are_continuous_on_all_sides() {
    let mut app = app_with(Screen::Wallets(WalletsState::new(Vec::new())));
    "zoe@example.com".chars().for_each(|c| app.email.push(c));

    for (w, h) in [(80u16, 24u16), (120, 30), (200, 60)] {
        let out = screen_text(&app, w, h);
        let rows: Vec<Vec<char>> = out.lines().map(|l| l.chars().collect()).collect();
        // Last row is the footer, which sits outside the block.
        let body = &rows[..rows.len() - 1];

        for (y, row) in body.iter().enumerate() {
            assert!(
                is_box_drawing(row[0]),
                "left border missing at row {y} ({w}x{h}): {:?}",
                row.iter().take(8).collect::<String>()
            );
            let last = *row.last().unwrap();
            assert!(
                is_box_drawing(last),
                "right border missing at row {y} ({w}x{h}): {:?}",
                row.iter().rev().take(8).collect::<String>()
            );
        }
    }
}
