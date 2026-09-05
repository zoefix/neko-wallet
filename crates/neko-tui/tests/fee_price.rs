//! What the fee costs, not just how much gas it burns.
//!
//! `~0.00000311 BNB` is a true statement that answers the wrong question. The
//! review screen has to say what leaves the account in money terms, and it has
//! to do it without ever implying a fee is free when it merely could not be
//! priced.

use neko_tui::app::{App, Screen};
use neko_tui::send::{BtcFee, EvmFee, FeeQuote, SendState, SendStep, SolanaFee, TronFee};

const TRON_MINE: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";
const TRON_TO: &str = "TNYxHL2s6Wjpx86NRwhekYzc27p3oDYrk6";
const BSC_MINE: &str = "0x1111111111111111111111111111111111111111";
const BSC_TO: &str = "0x2222222222222222222222222222222222222222";
const SOL_MINE: &str = "5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9";
const SOL_TO: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
const TON_MINE: &str = "EQAzWZa6nM5mJev91wGc7VCSfBoIsYRqKJpV78N8Add9-U9d";
const TON_TO: &str = "EQDVJucJT96vGh_bYm3e5uzenasiTOwA9orUHQiyhNsKmEcK";
const BTC_MINE: &str = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu";
const BTC_TO: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
/// BTCB's quote, which is what BTC is priced from.
const BTC_PRICE: i128 = 65_990_080_000; // 65,990.08 USDT

/// Real quotes, so the arithmetic below is the arithmetic users see.
const TRX_PRICE: i128 = 330_325; // 1 TRX = 0.330325 USDT
const BNB_PRICE: i128 = 722_902_400; // 1 BNB = 722.902400 USDT
const SOL_PRICE: i128 = 103_553_245; // 1 SOL = 103.553245 USDT, read from the pool

fn render(app: &App, w: u16, h: u16) -> String {
    neko_i18n::set_locale(neko_i18n::Locale::English);
    render_raw(app, w, h)
}

/// Renders in whatever locale is currently set, for the tests that are about
/// the other three.
fn render_raw(app: &App, w: u16, h: u16) -> String {
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
        neko_core::ChainId::Solana => (SOL_MINE, SOL_TO),
        neko_core::ChainId::Bitcoin => (BTC_MINE, BTC_TO),
        neko_core::ChainId::Ton => (TON_MINE, TON_TO),
        neko_core::ChainId::Ethereum
        | neko_core::ChainId::Polygon
        | neko_core::ChainId::Base
        | neko_core::ChainId::Arbitrum
        | neko_core::ChainId::Optimism => (BSC_MINE, BSC_TO),
    };
    let mut st = SendState::new(
        1,
        "w".into(),
        neko_core::ChainAddress::parse(chain, mine).unwrap(),
        // The last asset each chain carries: USDT where there is one, and BTC
        // on the chain that has only itself.
        *chain.assets().last().unwrap(),
        chain.assets().last().unwrap().symbol().to_string(),
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
        neko_core::ChainId::Bsc
        | neko_core::ChainId::Ethereum
        | neko_core::ChainId::Polygon
        | neko_core::ChainId::Base
        | neko_core::ChainId::Arbitrum
        | neko_core::ChainId::Optimism => neko_core::ChainTxParams::Evm(neko_evm::tx::TxParams {
            nonce: 0,
            gas_limit: 62_395,
            chain_id: chain.evm().unwrap().chain_id,
            fees: neko_evm::tx::Fees::Legacy {
                gas_price: 50_000_000,
            },
        }),
        neko_core::ChainId::Solana => neko_core::ChainTxParams::Solana(neko_solana::tx::TxParams {
            recent_blockhash: [0x22; 32],
            compute_unit_limit: neko_solana::COMPUTE_UNITS_TOKEN_WITH_ATA,
            compute_unit_price: 10_000,
            create_recipient_account: true,
        }),
        // Unused by these tests, which render a quote rather than sign one -
        // the fee shown comes from the FeeQuote, not from here.
        neko_core::ChainId::Ton => {
            neko_core::ChainTxParams::Ton(Box::new(neko_core::TonTxParams {
                seqno: 0,
                valid_until: 0,
                deploy: false,
                jetton_wallet: None,
            }))
        }
        neko_core::ChainId::Bitcoin => {
            neko_core::ChainTxParams::Bitcoin(Box::new(neko_core::BtcTxParams {
                inputs: Vec::new(),
                change: None,
                change_to: neko_hd::BtcAddress::parse(BTC_MINE).unwrap(),
                fee: 0,
            }))
        }
    };
    st.step = SendStep::Review {
        req: Box::new(req),
        params: Box::new(params),
        quote: Some(Box::new(quote)),
        typed: neko_tui::input::Field::new(false),
    };
    let mut app = App::new(std::path::PathBuf::from("/tmp/neko-fee-price.db"));
    app.screen = Screen::Send(Box::new(st));
    app
}

/// The exact quote from a real BEP-20 transfer: 62,395 gas at 0.05 gwei.
fn bsc_quote() -> FeeQuote {
    FeeQuote::Evm(EvmFee {
        chain: Box::new(neko_evm::BSC),
        gas_limit: 62_395,
        fees: neko_evm::tx::Fees::Legacy {
            gas_price: 50_000_000,
        },
        native_balance: Some(490_000_000_000_000),
        sending_native: false,
        amount: 1_000_000_000_000_000_000,
        l1_fee: 0,
    })
}

fn tron_quote() -> FeeQuote {
    FeeQuote::Tron(TronFee {
        // A real USDT transfer: 64,285 energy charged, of which 49,635 is the
        // dynamic-energy surcharge. The two are a total and a part of it - see
        // `neko_tron::EnergyEstimate`.
        energy_base: 64_285 - 49_635,
        energy_penalty: 49_635,
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
    // 64,285 x 100 sun = 6.4285 TRX, x 0.330325 = 2.123494 USDT, to cents.
    assert!(out.contains("6.4285"), "burn in TRX missing:\n{out}");
    assert!(out.contains("2.12 USDT"), "burn is not priced:\n{out}");
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

/// Opening a recipient's token account is the cost with no equivalent on the
/// other two chains, and it dwarfs the fee it is bundled with. Showing it as
/// part of an undifferentiated total would hide the reason this one transfer
/// costs forty times the last one.
#[test]
fn the_rent_for_a_new_token_account_is_shown_and_priced() {
    let quote = FeeQuote::Solana(SolanaFee {
        compute_units: neko_solana::COMPUTE_UNITS_TOKEN_WITH_ATA,
        compute_unit_price: 10_000,
        rent: neko_solana::TOKEN_ACCOUNT_RENT,
        sol_balance: Some(50_000_000), // 0.05 SOL
        sending_native: false,
        amount: 1_000_000,
    });
    let mut app = app_at_review(neko_core::ChainId::Solana, quote);
    app.prices
        .set_native(neko_core::ChainId::Solana, SOL_PRICE, 1_756_000_000);
    let out = render(&app, 135, 40);

    // 0.00203928 SOL of rent, spelled out on its own line.
    assert!(out.contains("0.00203928"), "the rent is not shown:\n{out}");
    assert!(
        out.contains("never held"),
        "the rent is shown without saying why:\n{out}"
    );

    // Signature 5,000 + ceil(60,000 x 10,000 / 1e6) = 600, plus rent.
    // 5,000 + 600 + 2,039,280 = 2,044,880 lamports = 0.00204488 SOL.
    assert!(
        out.contains("0.00204488"),
        "the total is not the fee plus the rent:\n{out}"
    );
    // x 103.553245 = 0.2117 USDT.
    assert!(out.contains("0.21 USDT"), "the total is not priced:\n{out}");
}

/// Without the rent, a Solana transfer is a fraction of a cent - and has to say
/// so rather than rounding to zero.
#[test]
fn a_transfer_to_an_existing_account_is_under_a_cent() {
    let quote = FeeQuote::Solana(SolanaFee {
        compute_units: neko_solana::COMPUTE_UNITS_TOKEN,
        compute_unit_price: 0,
        rent: 0,
        sol_balance: Some(50_000_000),
        sending_native: false,
        amount: 1_000_000,
    });
    let mut app = app_at_review(neko_core::ChainId::Solana, quote);
    app.prices
        .set_native(neko_core::ChainId::Solana, SOL_PRICE, 1_756_000_000);
    let out = render(&app, 135, 40);

    // 5,000 lamports = 0.000005 SOL = 0.00052 USDT.
    assert!(out.contains("0.000005"), "the fee is not shown:\n{out}");
    assert!(
        out.contains("<0.01 USDT"),
        "a sub-cent fee is not priced as one:\n{out}"
    );
    assert!(
        !out.contains("never held"),
        "rent was mentioned for an account that already exists:\n{out}"
    );
}

/// SOL pays every fee here, so a wallet holding only USDT cannot move it - the
/// same trap as BNB Chain's, and it has to be named rather than left as an
/// "insufficient funds" from the cluster.
#[test]
fn a_wallet_with_no_sol_is_told_what_it_needs() {
    let quote = FeeQuote::Solana(SolanaFee {
        compute_units: neko_solana::COMPUTE_UNITS_TOKEN_WITH_ATA,
        compute_unit_price: 0,
        rent: neko_solana::TOKEN_ACCOUNT_RENT,
        sol_balance: Some(0),
        sending_native: false,
        amount: 1_000_000,
    });
    let app = app_at_review(neko_core::ChainId::Solana, quote);
    let out = render(&app, 135, 40);

    assert!(
        out.contains("not enough SOL"),
        "the shortfall is not explained:\n{out}"
    );
    // 5,000 + 2,039,280 = 2,044,280 lamports short.
    assert!(
        out.contains("0.00204428"),
        "the shortfall figure is wrong:\n{out}"
    );
}

/// The markers under the destination point at the characters that have to be
/// checked, so they have to sit under exactly those characters.
///
/// This held only in English, because each of the three lines padded its own
/// label by hand and the padding was tuned to `From` being four cells and `To`
/// being two. In Japanese the two addresses and the markers started at three
/// different columns, and the thing the markers point at was whatever happened
/// to be two cells to the left.
#[test]
fn the_markers_sit_under_the_characters_they_point_at() {
    for locale in neko_i18n::LOCALES {
        neko_i18n::set_locale(locale);
        let app = app_at_review(neko_core::ChainId::Bsc, bsc_quote());
        let out = render_raw(&app, 135, 40);

        let from = out
            .lines()
            .find(|l| l.contains(BSC_MINE))
            .unwrap_or_else(|| panic!("{locale:?}: no From line"));
        let to = out
            .lines()
            .find(|l| l.contains(BSC_TO))
            .unwrap_or_else(|| panic!("{locale:?}: no To line"));
        let carets = out
            .lines()
            .find(|l| l.contains("^^^^^^"))
            .unwrap_or_else(|| panic!("{locale:?}: no marker line"));

        // Cells, not characters: a CJK label is two cells per character, which
        // is the whole reason this drifted.
        let col = |line: &str, needle: &str| {
            let at = line.find(needle).unwrap();
            unicode_width::UnicodeWidthStr::width(&line[..at])
        };

        let from_col = col(from, BSC_MINE);
        let to_col = col(to, BSC_TO);
        let caret_col = col(carets, "^");

        assert_eq!(
            from_col, to_col,
            "{locale:?}: the two addresses start at different columns\n{from}\n{to}"
        );
        assert_eq!(
            to_col, caret_col,
            "{locale:?}: the markers are not under the address\n{to}\n{carets}"
        );
    }
    neko_i18n::set_locale(neko_i18n::Locale::English);
}

/// On this chain the fee is a function of how many separate coins are being
/// spent, not of the amount. A wallet that received a hundred small payments
/// pays far more to move the same money than one holding a single large coin,
/// and there is nothing on the screen to explain that unless it is put there.
#[test]
fn a_bitcoin_fee_says_how_many_coins_it_is_spending() {
    let quote = FeeQuote::Bitcoin(BtcFee {
        fee_rate: neko_btc::coins::FeeRate::from_sat_per_vb(12.0),
        vbytes: 1_500,
        fee: 18_000,
        inputs: 21,
        utxo_count: 40,
        balance: 5_000_000,
        change: Some(120_000),
        change_was_dust: false,
    });
    let mut app = app_at_review(neko_core::ChainId::Bitcoin, quote);
    app.prices
        .set_native(neko_core::ChainId::Bitcoin, BTC_PRICE, 1_756_000_000);
    let out = render(&app, 135, 40);

    assert!(out.contains("12 sat/vB"), "the rate is not shown:\n{out}");
    assert!(
        out.contains("21 of 40"),
        "the coin count is not shown:\n{out}"
    );
    assert!(
        out.contains("68 bytes"),
        "spending 21 coins is not explained:\n{out}"
    );
    // 18,000 sat = 0.00018 BTC, and 0.00018 x 65,990.08 = 11.87 USDT.
    assert!(out.contains("0.00018"), "the fee is not shown:\n{out}");
    assert!(out.contains("11.87 USDT"), "the fee is not priced:\n{out}");
    // Change is money coming back, and has to be visible as such.
    assert!(out.contains("0.0012"), "the change is not shown:\n{out}");
}

/// A remainder too small to return goes to the fee. Left unsaid, the fee is
/// higher than the rate explains and there is no way to find out why.
#[test]
fn change_folded_into_the_fee_is_explained() {
    let quote = FeeQuote::Bitcoin(BtcFee {
        fee_rate: neko_btc::coins::FeeRate::from_sat_per_vb(10.0),
        vbytes: 110,
        fee: 1_290, // 1,100 at the rate, plus 190 of unreturnable remainder
        inputs: 1,
        utxo_count: 1,
        balance: 200_000,
        change: None,
        change_was_dust: true,
    });
    let app = app_at_review(neko_core::ChainId::Bitcoin, quote);
    let out = render(&app, 135, 40);

    assert!(
        out.contains("too small to return"),
        "the extra fee is unexplained:\n{out}"
    );
    // One coin is not the many-coins case, and must not claim to be.
    assert!(
        !out.contains("68 bytes"),
        "a single-coin spend was blamed on coin count:\n{out}"
    );
}
