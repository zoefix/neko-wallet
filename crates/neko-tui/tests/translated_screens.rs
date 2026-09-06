//! Screens rendered in a language other than English.
//!
//! Its own test binary on purpose. The active locale is process-global, and
//! every other rendering test sets it to English so it can assert on English
//! strings - so a test that sets it to anything else races with them, and both
//! sides lose. Cargo runs each integration file as its own process, which is
//! the isolation this needs.
//!
//! Isolation from *other files* is not isolation from each other. Once this
//! file held two tests that each walk every language, they raced here instead,
//! and the second one read a screen the first had just switched. So both take
//! the guard below before they touch the locale - the same one `borders.rs`,
//! `language.rs` and `fee_price.rs` use.

static LOCALE: std::sync::Mutex<()> = std::sync::Mutex::new(());

use neko_tui::app::{App, Screen};
use neko_tui::send::{FeeQuote, SendState, SendStep, TonFee};

fn ton_review() -> App {
    let mut app = App::new(std::path::PathBuf::from("/tmp/neko-translated.db"));
    let mut st = SendState::new(
        1,
        "w".into(),
        neko_core::ChainAddress::parse(
            neko_core::ChainId::Ton,
            "EQAzWZa6nM5mJev91wGc7VCSfBoIsYRqKJpV78N8Add9-U9d",
        )
        .unwrap(),
        neko_core::ChainId::Ton.stable().unwrap(),
        "USDT".into(),
    );
    "EQDVJucJT96vGh_bYm3e5uzenasiTOwA9orUHQiyhNsKmEcK"
        .chars()
        .for_each(|c| st.to.push(c));
    "2.7".chars().for_each(|c| st.amount.push(c));
    let req = st.build_request().unwrap();
    st.step = SendStep::Review {
        req: Box::new(req),
        params: Box::new(neko_core::ChainTxParams::Ton(Box::new(
            neko_core::TonTxParams {
                seqno: 1,
                valid_until: 0,
                deploy: false,
                jetton_wallet: None,
            },
        ))),
        quote: Some(Box::new(FeeQuote::Ton(TonFee {
            fee: neko_ton::FEE_TRANSFER,
            attached: neko_ton::JETTON_TRANSFER_ATTACHED,
            gram_balance: Some(99_006_120),
            sending_native: false,
            amount: 2_700_000,
            deploy: false,
        }))),
        typed: neko_tui::input::Field::new(false),
    };
    app.screen = Screen::Send(Box::new(st));
    app
}

/// The rendered screen, with the padding a double-width character leaves in
/// the cell beside it removed - the buffer reads "请 输 入", not "请输入".
fn flat(app: &App) -> String {
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 40)).unwrap();
    term.draw(|f| neko_tui::render::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
        .replace(' ', "")
}

/// Everything on the TON review screen that a non-English reader has to read.
///
/// One test rather than several, and the file holds nothing else: the active
/// locale is process-global, and Rust runs tests in parallel threads. Two tests
/// that each set it fight over the same value, which showed up as the suite
/// failing eight tests one run and one the next.
#[test]
fn the_ton_review_screen_reads_in_every_language() {
    let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
    let app = ton_review();

    // 1. The confirmation prompt had a translation in all four languages and
    //    the screen ignored it. `send.confirm_prompt` was written, translated,
    //    and never wired up - the renderer built the English sentence inline
    //    instead. So one line of English sat in the middle of an otherwise
    //    translated screen, on the last step before a signature, which is the
    //    worst place there is to be unreadable.
    for (locale, want) in [
        (neko_i18n::Locale::Simplified, "请输入收款地址的最后"),
        (neko_i18n::Locale::Traditional, "請輸入收款地址的最後"),
        (
            neko_i18n::Locale::Japanese,
            "確認のため送金先アドレスの末尾",
        ),
    ] {
        neko_i18n::set_locale(locale);
        let out = flat(&app);
        assert!(out.contains(want), "{locale:?} still shows English:\n{out}");
        assert!(
            !out.contains("TypetheLAST"),
            "{locale:?} shows both languages:\n{out}"
        );
    }

    // 2. TON's own fee lines, in the language they were reported in.
    neko_i18n::set_locale(neko_i18n::Locale::Traditional);
    let out = flat(&app);
    assert!(out.contains("隨訊息附帶"), "no attached-coin row:\n{out}");
    // Why the total says "at most", which is not because anything failed.
    assert!(out.contains("固定的上限額度"), "no allowance note:\n{out}");
    // And none of TRON's vocabulary, which this screen used to borrow.
    for absent in ["能量", "頻寬", "APIKey"] {
        assert!(!out.contains(absent), "TON has no {absent}:\n{out}");
    }

    // 3. English still reads as English rather than as a raw key.
    neko_i18n::set_locale(neko_i18n::Locale::English);
    assert!(flat(&app).contains("TypetheLAST6characters"));
}

/// The two screens shown before a wallet is unlocked, in every language.
///
/// These are the first thing anybody sees and the last place an English word
/// should survive. Two did: the first-run screen labelled its password and
/// confirm fields with hardcoded strings while the email field beside them was
/// translated, so a Chinese setup screen read 電子郵件 / Password / Confirm.
/// Both translations had existed in all four languages the whole time - the
/// same silent gap as `send.confirm_prompt` and the settings toasts.
///
/// **Asserted on the field's own line, not on the screen.** The first version
/// of this test searched the whole render for the translated word and passed
/// with the bug still in place: the warning above the fields already contains
/// 密碼, so "the screen mentions the word somewhere" was true either way. A
/// label is only translated if the line carrying its input box says so.
#[test]
fn the_screens_before_unlocking_are_translated() {
    let _g = LOCALE.lock().unwrap_or_else(|e| e.into_inner());
    use neko_i18n::Key;

    // The lines carrying an input box, in the order they are drawn.
    fn fields(app: &App) -> Vec<String> {
        let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        term.draw(|f| neko_tui::render::draw(f, app)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .filter(|l| l.contains('['))
            // Spaces removed for the same reason as `flat`: the buffer leaves
            // a padding cell beside every double-width character, so 邮箱
            // reads back as "邮 箱".
            .map(|l| l.replace(' ', ""))
            .collect()
    }

    for locale in neko_i18n::LOCALES {
        neko_i18n::set_locale(locale);

        // First run: email, password, confirm.
        let mut app = App::new(std::path::PathBuf::from("/tmp/neko-no-such-vault.db"));
        app.set_viewport(100, 30);
        let rows = fields(&app);
        assert_eq!(rows.len(), 3, "{locale:?}: expected three fields");
        for (row, key) in
            rows.iter()
                .zip([Key::Common_Email, Key::Common_Password, Key::Common_Confirm])
        {
            let want = neko_i18n::t(key).replace(' ', "");
            assert!(
                row.contains(&want),
                "{locale:?}: a first-run field is not labelled {want:?}:\n{row}"
            );
        }

        // Login: email and password, on the screen with the cat.
        let mut app = App::new(std::path::PathBuf::from("/tmp/neko-no-such-vault.db"));
        app.screen = Screen::Login {
            email_focused: true,
        };
        app.set_viewport(100, 30);
        let rows = fields(&app);
        assert_eq!(rows.len(), 2, "{locale:?}: expected two fields");
        for (row, key) in rows.iter().zip([Key::Common_Email, Key::Common_Password]) {
            let want = neko_i18n::t(key).replace(' ', "");
            assert!(
                row.contains(&want),
                "{locale:?}: a login field is not labelled {want:?}:\n{row}"
            );
        }

        // And the tagline no longer claims this is a TRON wallet.
        let out = flat(&app);
        assert!(out.contains(&neko_i18n::t(Key::Login_Tagline).replace(' ', "")));
        assert!(
            !out.contains("TRON"),
            "{locale:?}: the login screen still names one chain"
        );
    }
    neko_i18n::set_locale(neko_i18n::Locale::English);
}
