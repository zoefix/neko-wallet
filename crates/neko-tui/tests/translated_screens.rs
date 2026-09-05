//! Screens rendered in a language other than English.
//!
//! Its own test binary on purpose. The active locale is process-global, and
//! every other rendering test sets it to English so it can assert on English
//! strings - so a test that sets it to anything else races with them, and both
//! sides lose. Cargo runs each integration file as its own process, which is
//! the isolation this needs.

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
        neko_core::ChainId::Ton.usdt().unwrap(),
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
