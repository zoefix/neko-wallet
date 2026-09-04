//! What the fee costs, not just how much gas it burns.
//!
//! `~0.00000311 BNB` is a true statement that answers the wrong question. The
//! review screen has to say what leaves the account in money terms, and it has
//! to do it without ever implying a fee is free when it merely could not be
//! priced.

use neko_tui::app::{App, Screen};
use neko_tui::send::{BscFee, FeeQuote, SendState, SendStep, TronFee};

const TRON_MINE: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";
const TRON_TO: &str = "TNYxHL2s6Wjpx86NRwhekYzc27p3oDYrk6";
const BSC_MINE: &str = "0x1111111111111111111111111111111111111111";
const BSC_TO: &str = "0x2222222222222222222222222222222222222222";

/// Real quotes, so the arithmetic below is the arithmetic users see.
const TRX_PRICE: i128 = 330_325; // 1 TRX = 0.330325 USDT
const BNB_PRICE: i128 = 722_902_400; // 1 BNB = 722.902400 USDT

fn render(app: &App, w: u16, h: u16) -> String {
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

fn app_at_review(chain: neko_core::ChainId, quote: FeeQuote) -> App {
    let (mine, to) = match chain {
        neko_core::ChainId::Tron => (TRON_MINE, TRON_TO),
        neko_core::ChainId::Bsc => (BSC_MINE, BSC_TO),
    };
    let mut st = SendState::new(
        1,
        "w".into(),
        neko_core::ChainAddress::parse(chain, mine).unwrap(),
        chain.usdt(),
        "USDT".into(),
    );
    to.chars().for_each(|c| st.to.push(c));
    "1".chars().for_each(|c| st.amount.push(c));
    let req = st.build_request().unwrap();
    let params = match chain {
        neko_core::ChainId::Tron => {
            neko_core::ChainTxParams::Tron(Box::new(neko_tron::tx::TxParams {
                ref_block_num: 1,
                ref_block_hash: [0xab; 32],
                timestamp: 1_756_000_000_000,
                expiration: 1_756_000_060_000,
                fee_limit: 100_000_000,
            }))
        }
        neko_core::ChainId::Bsc => neko_core::ChainTxParams::Evm(neko_evm::tx::TxParams {
            nonce: 0,
            gas_price: 50_000_000,
            gas_limit: 62_395,
            chain_id: neko_evm::CHAIN_ID,
        }),
    };
    st.step = SendStep::Review {
        req: Box::new(req),
        params: Box::new(params),
        quote: Some(quote),
        typed: neko_tui::input::Field::new(false),
    };
    let mut app = App::new(std::path::PathBuf::from("/tmp/neko-fee-price.db"));
    app.screen = Screen::Send(Box::new(st));
    app
}

/// The exact quote from a real BEP-20 transfer: 62,395 gas at 0.05 gwei.
fn bsc_quote() -> FeeQuote {
    FeeQuote::Bsc(BscFee {
        gas_limit: 62_395,
        gas_price: 50_000_000,
        bnb_balance: Some(490_000_000_000_000),
        sending_native: false,
        amount: 1_000_000_000_000_000_000,
    })
}

fn tron_quote() -> FeeQuote {
    FeeQuote::Tron(TronFee {
        energy_base: 64_285,
        energy_penalty: 113_920 - 64_285,
        bandwidth_needed: 345,
        available: Some(((0, 131_016), (600, 600))),
        prices: Some((100, 1000)),
        recipient_is_new: false,
    })
}

/// A BNB Chain fee is a handful of millionths of a coin. Left unpriced the
/// number is unreadable; priced and rounded to cents it would read `0.00`,
/// which is the one thing it must not say.
#[test]
fn a_sub_cent_fee_says_it_is_under_a_cent_not_zero() {
    let mut app = app_at_review(neko_core::ChainId::Bsc, bsc_quote());
    app.prices
        .set_native(neko_core::ChainId::Bsc, BNB_PRICE, 1_756_000_000);
    let out = render(&app, 135, 40);

    // The gas figure is still there...
    assert!(out.contains("0.00000311"), "fee in BNB missing:\n{out}");
    // ...and now so is what it costs.
    assert!(
        out.contains("< 0.01 USDT") || out.contains("<0.01 USDT"),
        "sub-cent fee is not priced:\n{out}"
    );
    assert!(
        !out.contains("0.00 USDT"),
        "a real fee was rendered as costing nothing:\n{out}"
    );
}

/// A TRON burn is large enough to price properly, and that figure is what
/// tells somebody whether the transfer is worth making.
#[test]
fn a_tron_burn_is_priced_in_usdt() {
    let mut app = app_at_review(neko_core::ChainId::Tron, tron_quote());
    app.prices
        .set_native(neko_core::ChainId::Tron, TRX_PRICE, 1_756_000_000);
    let out = render(&app, 135, 40);

    // Bandwidth is covered here, so the burn is energy alone:
    // 11.392 TRX x 0.330325 = 3.763062 USDT, shown to cents.
    assert!(out.contains("11.392"), "burn in TRX missing:\n{out}");
    assert!(out.contains("3.76 USDT"), "burn is not priced:\n{out}");
}

/// An unknown price is not a free transfer. Saying nothing is the only honest
/// option: a zero here would be read as "this costs nothing".
#[test]
fn an_unknown_price_prices_nothing_rather_than_zero() {
    let app = app_at_review(neko_core::ChainId::Bsc, bsc_quote());
    assert!(app.prices.is_empty());
    let out = render(&app, 135, 40);

    assert!(out.contains("0.00000311"), "fee in BNB missing:\n{out}");
    assert!(
        !out.contains("USDT 0") && !out.contains("0.00 USDT") && !out.contains("\u{2248}"),
        "an unpriced fee still claimed a price:\n{out}"
    );
}

/// The amount being signed keeps every digit, and loses only the zeros that
/// make it harder to check. Eighteen of them turn telling `1` from `10` into a
/// counting exercise.
#[test]
fn the_signed_amount_sheds_its_zeros_but_not_its_digits() {
    let app = app_at_review(neko_core::ChainId::Bsc, bsc_quote());
    let out = render(&app, 135, 40);

    assert!(
        !out.contains("1.000000000000000000"),
        "eighteen zeros are still on the confirmation screen:\n{out}"
    );
    assert!(out.contains("1 USDT"), "the amount is missing:\n{out}");
}
