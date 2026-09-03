//! Language selection in the running app.

use crossterm::event::{KeyCode, KeyEvent};
use neko_i18n::{Key, Locale};
use neko_tui::app::{App, Screen};
use neko_tui::keys;
use neko_tui::nav::SettingRow;

/// `t()` reads a process-global locale, and tests in one binary run on
/// parallel threads. Any test that either asserts on translated text or
/// changes the language has to hold this, or it will occasionally read a
/// language another test set half a frame earlier.
static LOCALE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn locale_guard() -> std::sync::MutexGuard<'static, ()> {
    LOCALE.lock().unwrap_or_else(|e| e.into_inner())
}

const EMAIL: &str = "zoe@example.com";
const PW: &str = "correct horse battery staple xyzzy";

fn code(c: KeyCode) -> KeyEvent {
    KeyEvent::from(c)
}
fn channel() -> tokio::sync::mpsc::UnboundedSender<neko_tui::event::AppEvent> {
    tokio::sync::mpsc::unbounded_channel().0
}

fn unlocked(dir: &std::path::Path) -> App {
    let mut app = App::new(dir.join("neko-wallet.db"));
    EMAIL.chars().for_each(|c| app.email.push(c));
    PW.chars().for_each(|c| app.password.push(c));
    PW.chars().for_each(|c| app.confirm.push(c));
    let ev = app.begin_unlock(true).unwrap().run();
    app.on_app_event(ev);
    app
}

/// All four languages are reachable by cycling, and cycling wraps.
#[tokio::test]
async fn cycling_reaches_every_language() {
    let _g = locale_guard();
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    app.set_language(Locale::English);

    let mut seen = vec![app.locale];
    for _ in 0..3 {
        app.cycle_language(true);
        seen.push(app.locale);
    }
    for l in Locale::English as usize..=3 {
        assert!(
            seen.contains(&neko_i18n::LOCALES[l]),
            "language {l} unreachable"
        );
    }
    app.cycle_language(true);
    assert_eq!(app.locale, Locale::English, "cycling does not wrap");

    app.cycle_language(false);
    assert_eq!(
        app.locale,
        Locale::Japanese,
        "cycling backwards does not wrap"
    );
}

/// The choice is per-vault and must survive a lock/unlock cycle.
#[tokio::test]
async fn language_choice_persists_across_unlock() {
    let _g = locale_guard();
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("neko-wallet.db");
    {
        let mut app = unlocked(dir.path());
        app.set_language(Locale::Japanese);
    }

    let mut app = App::new(db);
    EMAIL.chars().for_each(|c| app.email.push(c));
    PW.chars().for_each(|c| app.password.push(c));
    let ev = app.begin_unlock(false).unwrap().run();
    app.on_app_event(ev);
    assert_eq!(
        app.locale,
        Locale::Japanese,
        "language choice was not restored"
    );
}

/// Switching takes effect on the very next frame; nothing is reloaded.
#[tokio::test]
async fn switching_changes_the_rendered_text_immediately() {
    let _g = locale_guard();
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());

    app.set_language(Locale::English);
    let en = render(&app);
    app.set_language(Locale::Simplified);
    let zh = render(&app);
    app.set_language(Locale::Japanese);
    let ja = render(&app);

    assert_ne!(en, zh, "switching to Simplified changed nothing");
    assert_ne!(zh, ja, "switching to Japanese changed nothing");
    assert!(zh.contains("钱包"), "Simplified text missing:\n{zh}");
    assert!(ja.contains("ウォレット"), "Japanese text missing:\n{ja}");

    neko_i18n::set_locale(Locale::English);
}

/// The picker lists every language in its own script, so somebody who cannot
/// read the current UI can still get back out.
#[tokio::test]
async fn the_picker_shows_each_language_in_its_own_script() {
    let _g = locale_guard();
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    app.set_language(Locale::English);
    app.open_settings();

    let out = render(&app);
    for l in neko_i18n::LOCALES {
        assert!(
            out.contains(l.endonym()),
            "{} is not offered in its own script:\n{out}",
            l.tag()
        );
    }
    neko_i18n::set_locale(Locale::English);
}

/// Arrow keys move through the languages from the settings screen.
#[tokio::test]
async fn the_language_row_responds_to_arrows() {
    let _g = locale_guard();
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());
    app.set_language(Locale::English);
    app.open_settings();

    let Screen::Settings(st) = &app.screen else {
        panic!()
    };
    assert_eq!(
        st.row(),
        SettingRow::Language,
        "Language is not the first row"
    );

    keys::on_key_settings(&mut app, code(KeyCode::Right), &channel());
    assert_ne!(
        app.locale,
        Locale::English,
        "arrow did not change the language"
    );
    keys::on_key_settings(&mut app, code(KeyCode::Left), &channel());
    assert_eq!(app.locale, Locale::English, "arrow did not change it back");

    neko_i18n::set_locale(Locale::English);
}

/// CJK is wide: every line must still be exactly the terminal width in all four
/// languages, or the tables shear.
#[tokio::test]
async fn every_language_renders_without_shearing() {
    let _g = locale_guard();
    let dir = tempfile::tempdir().unwrap();
    let mut app = unlocked(dir.path());

    for l in neko_i18n::LOCALES {
        app.set_language(l);
        for (w, h) in [(80u16, 24u16), (120, 36)] {
            let mut term =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(w, h)).unwrap();
            term.draw(|f| neko_tui::render::draw(f, &app)).unwrap();
            let buf = term.backend().buffer().clone();
            for y in 0..buf.area.height {
                let mut line = String::new();
                let mut x = 0u16;
                while x < buf.area.width {
                    let sym = buf[(x, y)].symbol();
                    line.push_str(sym);
                    x += unicode_width::UnicodeWidthStr::width(sym).max(1) as u16;
                }
                assert_eq!(
                    unicode_width::UnicodeWidthStr::width(line.as_str()),
                    w as usize,
                    "{} sheared row {y} at {w}x{h}: {line:?}",
                    l.tag()
                );
            }
        }
    }
    neko_i18n::set_locale(Locale::English);
}

/// A key with a placeholder must substitute in every language.
#[tokio::test]
async fn placeholders_substitute_in_every_language() {
    let _g = locale_guard();
    for l in neko_i18n::LOCALES {
        neko_i18n::set_locale(l);
        let s = neko_i18n::tf(Key::Common_MinutesAgo, &[("n", "7")]);
        assert!(
            s.contains('7'),
            "{}: placeholder not substituted: {s}",
            l.tag()
        );
        assert!(!s.contains("%{"), "{}: placeholder left raw: {s}", l.tag());
    }
    neko_i18n::set_locale(Locale::English);
}

fn render(app: &App) -> String {
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 36)).unwrap();
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
