//! The fee breakdown.
//!
//! TRON burns TRX only for what an account cannot already cover, so a bare
//! total is not actionable. These tests pin that the arithmetic is right and
//! that the user can see where the number came from.

use neko_tui::app::{App, Screen};
use neko_tui::send::{FeeQuote, SendState, SendStep, TronFee};

const MINE: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";
const TO: &str = "TNYxHL2s6Wjpx86NRwhekYzc27p3oDYrk6";

fn quote(energy_needed: i64, energy_available: i64, bw_needed: i64, bw_available: i64) -> TronFee {
    TronFee {
        // Split in the measured mainnet proportion (64,285 of 113,920 is base).
        energy_base: energy_needed * 64_285 / 113_920,
        energy_penalty: energy_needed - energy_needed * 64_285 / 113_920,
        bandwidth_needed: bw_needed,
        available: Some(((energy_available, 131_016), (bw_available, 600))),
        // The live values on mainnet at the time of writing.
        prices: Some((100, 1000)),
        recipient_is_new: false,
    }
}

/// A quote where the resource lookup failed — the intermittent case that
/// previously rendered as a confident zero.
fn quote_unknown(energy_needed: i64, bw_needed: i64) -> TronFee {
    TronFee {
        energy_base: energy_needed,
        energy_penalty: 0,
        bandwidth_needed: bw_needed,
        available: None,
        prices: Some((100, 1000)),
        recipient_is_new: false,
    }
}

/// Only the shortfall costs anything.
#[test]
fn only_the_shortfall_is_burned() {
    // Nothing staked: the whole requirement is burned.
    let q = quote(113_920, 0, 345, 0);
    assert_eq!(q.energy_shortfall(), 113_920);
    assert_eq!(q.energy_burn().to_exact_string(), "11.392000");
    assert_eq!(q.bandwidth_burn().to_exact_string(), "0.345000");
    assert_eq!(q.total_burn().to_exact_string(), "11.737000");
    assert!(!q.is_free());

    // Fully covered: nothing is burned at all.
    let q = quote(113_920, 200_000, 345, 600);
    assert_eq!(q.energy_shortfall(), 0);
    assert_eq!(q.bandwidth_shortfall(), 0);
    assert_eq!(q.total_burn().to_exact_string(), "0.000000");
    assert!(q.is_free());

    // Partly covered: only the difference.
    let q = quote(100_000, 40_000, 345, 600);
    assert_eq!(q.energy_shortfall(), 60_000);
    assert_eq!(q.total_burn().to_exact_string(), "6.000000");
}

/// Surplus resources must not produce a negative "credit".
#[test]
fn surplus_resources_never_go_negative() {
    let q = quote(10, 999_999, 5, 999_999);
    assert_eq!(q.energy_shortfall(), 0);
    assert_eq!(q.bandwidth_shortfall(), 0);
    assert_eq!(q.total_burn().raw, 0);
}

/// The price comes from the chain, not a constant. Mainnet moved from 210 to
/// 100 sun/energy; a hardcoded figure would have overstated every fee by 2.1x.
#[test]
fn the_price_is_a_parameter_not_a_constant() {
    let mut q = quote(100_000, 0, 0, 0);
    assert_eq!(q.energy_burn().to_exact_string(), "10.000000");
    q.prices = Some((210, 1000));
    assert_eq!(q.energy_burn().to_exact_string(), "21.000000");
}

fn review_screen(q: TronFee) -> App {
    let mut app = App::new(std::path::PathBuf::from("/tmp/neko-fee.db"));
    let mut st = SendState::new(
        1,
        "w".into(),
        neko_core::ChainAddress::parse(neko_core::ChainId::Tron, MINE).unwrap(),
        neko_core::Asset::Trc20 {
            contract: neko_tron::usdt_address(),
            decimals: 6,
        },
        "USDT".into(),
    );
    TO.chars().for_each(|c| st.to.push(c));
    "1".chars().for_each(|c| st.amount.push(c));
    let req = st.build_request().unwrap();
    st.step = SendStep::Review {
        req: Box::new(req),
        params: Box::new(neko_core::ChainTxParams::Tron(Box::new(
            neko_tron::tx::TxParams {
                ref_block_num: 1,
                ref_block_hash: [0xab; 32],
                timestamp: 1_756_000_000_000,
                expiration: 1_756_000_060_000,
                fee_limit: 100_000_000,
            },
        ))),
        quote: Some(FeeQuote::Tron(q)),
        typed: neko_tui::input::Field::new(false),
    };
    app.screen = Screen::Send(Box::new(st));
    app
}

fn render(app: &App, w: u16, h: u16) -> String {
    // These assertions are written against the English strings; the app
    // otherwise follows the OS language, which is not the same on every
    // machine that runs the suite.
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

/// The screen must show what is held, not just what is needed — that is the
/// difference between a number and an explanation.
#[test]
fn the_breakdown_shows_holdings_not_just_requirements() {
    let out = render(&review_screen(quote(113_920, 0, 345, 600)), 120, 40);
    assert!(out.contains("Energy"), "no energy row:\n{out}");
    assert!(out.contains("Bandwidth"), "no bandwidth row:\n{out}");
    assert!(
        out.contains("113,920"),
        "energy requirement missing:\n{out}"
    );
    assert!(out.contains("you have"), "holdings not shown:\n{out}");
    assert!(out.contains("11.392000"), "energy burn missing:\n{out}");
    // Bandwidth is covered by the free daily allowance here.
    assert!(
        out.contains("covered"),
        "a covered resource is not labelled:\n{out}"
    );
    assert!(out.contains("sun/energy"), "the price is not shown:\n{out}");
}

/// An account with enough staked energy pays nothing, and must be told so
/// plainly rather than shown a misleading "~0.000000 TRX".
#[test]
fn a_fully_covered_transfer_is_labelled_free() {
    let out = render(&review_screen(quote(113_920, 500_000, 345, 600)), 120, 40);
    assert!(
        out.contains("free"),
        "a free transfer is not labelled as such:\n{out}"
    );
    assert!(
        !out.contains("burned"),
        "claimed a burn when nothing is burned:\n{out}"
    );
}

#[test]
fn a_first_time_recipient_is_explained() {
    let mut q = quote(113_920, 0, 345, 600);
    q.recipient_is_new = true;
    let out = render(&review_screen(q), 120, 40);
    assert!(
        out.contains("never held"),
        "the reason for the high energy figure is not explained:\n{out}"
    );
}

/// The breakdown must still fit at the minimum supported terminal size.
#[test]
fn the_breakdown_fits_the_smallest_terminal() {
    let out = render(&review_screen(quote(113_920, 0, 345, 600)), 80, 24);
    assert!(
        !out.contains("too small"),
        "the review screen no longer fits 80x24"
    );
    for line in out.lines() {
        assert_eq!(
            unicode_width::UnicodeWidthStr::width(line),
            80,
            "sheared: {line:?}"
        );
    }
}

/// A failed resource lookup must never be presented as "you have zero".
///
/// Without an API key these calls hit the public rate limit intermittently, so
/// the same transfer would show 336 bandwidth one moment and 0 the next — the
/// second being a claim about the user's own account that is simply false.
#[test]
fn a_failed_lookup_is_not_reported_as_zero() {
    let q = quote_unknown(113_920, 282);
    assert!(!q.resources_known());
    assert!(q.is_upper_bound());
    // It must not claim the transfer is free either.
    assert!(!q.is_free());

    let out = render(&review_screen(q), 120, 40);
    assert!(
        out.contains("unknown"),
        "a failed lookup was not labelled:\n{out}"
    );
    assert!(
        out.contains("Could not read"),
        "the failure is not explained to the user:\n{out}"
    );
    assert!(
        out.contains("at most"),
        "an upper bound is presented as an exact figure:\n{out}"
    );
    assert!(
        out.contains("API key"),
        "no pointer at the fix for the intermittency:\n{out}"
    );
}

/// A known-zero account is different from an unknown one, and must read that way.
#[test]
fn a_genuine_zero_reads_differently_from_unknown() {
    let known = render(&review_screen(quote(113_920, 0, 282, 0)), 120, 40);
    assert!(
        known.contains("short 113,920"),
        "a real shortfall is not shown:\n{known}"
    );
    assert!(
        !known.contains("unknown"),
        "a known zero was labelled unknown:\n{known}"
    );
    assert!(
        !known.contains("Could not read"),
        "a successful lookup warned anyway:\n{known}"
    );
    assert!(
        known.contains("burned"),
        "a definite burn is hedged:\n{known}"
    );
}

/// Falling back on stale burn prices must be disclosed too.
#[test]
fn fallback_prices_are_disclosed() {
    let mut q = quote(113_920, 0, 282, 0);
    q.prices = None;
    assert!(!q.prices_known());
    assert_eq!(q.sun_per_energy(), neko_tui::send::FALLBACK_SUN_PER_ENERGY);

    let out = render(&review_screen(q), 120, 40);
    assert!(
        out.contains("fallback"),
        "fallback prices were presented as chain data:\n{out}"
    );
}

/// With holdings unknown we assume none, so the figure is a ceiling — never an
/// underestimate that could leave a transfer short.
#[test]
fn the_unknown_case_errs_high_not_low() {
    let unknown = quote_unknown(113_920, 282);
    let worst_case = quote(113_920, 0, 282, 0);
    assert_eq!(unknown.total_burn().raw, worst_case.total_burn().raw);

    let best_case = quote(113_920, 999_999, 282, 999_999);
    assert!(unknown.total_burn().raw >= best_case.total_burn().raw);
}

/// The surcharge must be shown, not folded silently into one number.
///
/// Measured on mainnet 2026-09: a USDT transfer to an address that already
/// holds the token costs 64,285 base + 49,635 dynamic-energy surcharge. Showing
/// only 113,920 leaves the user unable to tell why it is four times the figures
/// quoted in older TRON documentation.
#[test]
fn the_dynamic_energy_surcharge_is_broken_out() {
    let q = TronFee {
        energy_base: 64_285,
        energy_penalty: 49_635,
        bandwidth_needed: 282,
        available: Some(((0, 131_016), (336, 600))),
        prices: Some((100, 1000)),
        recipient_is_new: false,
    };
    assert_eq!(q.energy_needed(), 113_920);
    assert_eq!(q.energy_burn().to_exact_string(), "11.392000");

    let out = render(&review_screen(q), 120, 40);
    assert!(out.contains("113,920"), "total energy missing:\n{out}");
    assert!(out.contains("64,285"), "base energy not broken out:\n{out}");
    assert!(out.contains("49,635"), "surcharge not broken out:\n{out}");
    assert!(
        out.contains("surcharge"),
        "the surcharge is not named:\n{out}"
    );
}

/// A transfer with no surcharge must not print an empty breakdown line.
#[test]
fn no_surcharge_means_no_extra_line() {
    let q = TronFee {
        energy_base: 30_000,
        energy_penalty: 0,
        bandwidth_needed: 282,
        available: Some(((0, 131_016), (336, 600))),
        prices: Some((100, 1000)),
        recipient_is_new: false,
    };
    let out = render(&review_screen(q), 120, 40);
    assert!(
        !out.contains("surcharge"),
        "printed a surcharge line for zero:\n{out}"
    );
}

/// The old "15x" claim came from the archived reference project and no longer
/// matches the chain. Measured today it is roughly 2x, and the wording must not
/// overstate it.
#[test]
fn the_first_time_recipient_note_is_not_overstated() {
    let mut q = quote(113_920, 0, 282, 336);
    q.recipient_is_new = true;
    let out = render(&review_screen(q), 120, 40);
    assert!(out.contains("never held"), "the note is missing:\n{out}");
    assert!(
        out.contains("doubles"),
        "the effect is not quantified:\n{out}"
    );
    assert!(
        !out.contains("15x"),
        "the stale 15x figure is still shown:\n{out}"
    );
}

/// Bandwidth is charged for the signed transaction *plus* a flat allowance the
/// chain reserves for the result.
///
/// Checked against a real mainnet USDT transfer: the chain charged 345
/// bandwidth and burned exactly 0.345 TRX at 1000 sun/byte. An estimate that
/// omits the 64-byte allowance comes out at 281 and understates the fee.
#[test]
fn bandwidth_includes_the_result_allowance() {
    // A real USDT transfer: 345 bytes charged, burned at 1000 sun/byte.
    let q = TronFee {
        energy_base: 64_285,
        energy_penalty: 0,
        bandwidth_needed: 345,
        // Free daily allowance already spent, so the bandwidth is burned.
        available: Some(((200_000, 200_000), (0, 600))),
        prices: Some((100, 1000)),
        recipient_is_new: false,
    };
    assert_eq!(q.bandwidth_shortfall(), 345);
    assert_eq!(
        q.bandwidth_burn().to_exact_string(),
        "0.345000",
        "does not match the 0.345 TRX the chain actually burned"
    );
    // Energy came from stake, so only bandwidth costs anything.
    assert_eq!(q.energy_shortfall(), 0);
    assert_eq!(q.total_burn().to_exact_string(), "0.345000");
}

/// The estimator's own arithmetic: a signed transfer must land on the figure
/// the chain charges, not the raw body size.
#[test]
fn the_bandwidth_estimate_matches_a_real_transfer() {
    use neko_hd::Address;
    use neko_tron::tx;

    let params = tx::TxParams {
        ref_block_num: 68_000_123,
        ref_block_hash: [0xab; 32],
        timestamp: 1_756_000_000_000,
        expiration: 1_756_000_060_000,
        fee_limit: 100_000_000,
    };
    let raw = tx::build_trc20_transfer(
        Address::parse(MINE).unwrap(),
        neko_tron::usdt_address(),
        Address::parse(TO).unwrap(),
        1_000_000,
        &params,
    )
    .unwrap();

    // Same formula the quote uses: body + field overhead + signature + the
    // chain's flat result allowance.
    let estimate = raw.len() + 4 + 67 + 64;
    assert!(
        (330..=360).contains(&estimate),
        "estimate {estimate} is nowhere near the 345 the chain charged"
    );
}

/// Energy regenerates continuously — measured at roughly 0.75/second against a
/// 131k limit. Showing only the available figure makes it look wrong the moment
/// it is compared with an explorer reading taken seconds later, so both the
/// available amount and the cap are shown.
#[test]
fn resources_are_shown_against_their_limit() {
    let q = TronFee {
        energy_base: 64_285,
        energy_penalty: 49_635,
        bandwidth_needed: 346,
        available: Some(((66_977, 131_016), (339, 600))),
        prices: Some((100, 1000)),
        recipient_is_new: false,
    };
    assert_eq!(q.energy_limit(), Some(131_016));
    assert_eq!(q.bandwidth_limit(), Some(600));
    assert_eq!(q.energy_shortfall(), 46_943);
    assert_eq!(q.energy_burn().to_exact_string(), "4.694300");

    let out = render(&review_screen(q), 120, 40);
    assert!(
        out.contains("66,977/131,016"),
        "energy is not shown against its limit:\n{out}"
    );
    assert!(
        out.contains("339/600"),
        "bandwidth is not shown against its limit:\n{out}"
    );
}
