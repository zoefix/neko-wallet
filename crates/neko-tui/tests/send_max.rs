//! Sending an entire balance.
//!
//! For a token this is just the balance. For the chain's own coin it is not
//! typeable at all: the fee comes out of the same balance, and the fee is not
//! known until the chain has been asked. Somebody trying to empty an account by
//! hand types the balance, is told they are short by the fee, subtracts it,
//! is told the fee changed, and gives up.
//!
//! The arithmetic here decides whether a transfer is accepted or rejected by the
//! chain, so every case below checks the exact figure rather than that something
//! plausible happened.

use neko_core::Amount;
use neko_tui::send::{BscFee, FeeQuote, SendState, TronFee};

const BSC_MINE: &str = "0x1111111111111111111111111111111111111111";
const BSC_TO: &str = "0x2222222222222222222222222222222222222222";
const SOLANA_MINE: &str = "5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9";
const BTC_MINE: &str = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu";
const TRON_MINE: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";

/// The balance from the screenshot that started this: 0.00008488 BNB, and a
/// 25,200 x 0.05 gwei fee that leaves it 0.00000126 BNB short of sending it all.
const BALANCE: i128 = 84_880_000_000_000;
const GAS_LIMIT: u64 = 25_200;
const GAS_PRICE: u128 = 50_000_000;
const FEE: i128 = GAS_LIMIT as i128 * GAS_PRICE as i128; // 1_260_000_000_000

fn state(asset: neko_core::Asset, balance: Option<i128>) -> SendState {
    let mine = match asset.chain() {
        neko_core::ChainId::Tron => TRON_MINE,
        neko_core::ChainId::Bsc => BSC_MINE,
        neko_core::ChainId::Solana => SOLANA_MINE,
        neko_core::ChainId::Bitcoin => BTC_MINE,
    };
    let mut st = SendState::new(
        1,
        "w".into(),
        neko_core::ChainAddress::parse(asset.chain(), mine).unwrap(),
        asset,
        asset.symbol().to_string(),
    );
    BSC_TO.chars().for_each(|c| st.to.push(c));
    st.balance = balance;
    st
}

fn bsc_quote(amount: u128, balance: Option<u128>) -> FeeQuote {
    FeeQuote::Bsc(BscFee {
        gas_limit: GAS_LIMIT,
        gas_price: GAS_PRICE,
        bnb_balance: balance,
        sending_native: true,
        amount,
    })
}

/// The whole point: after the fee is held back, amount + fee is the balance
/// exactly. A wei too many is rejected by the chain; a wei too few is dust left
/// behind in an account somebody meant to empty.
#[test]
fn the_maximum_plus_the_fee_is_the_balance_exactly() {
    let mut st = state(neko_core::Asset::Bnb, Some(BALANCE));
    st.request_max();
    // Before the quote, "everything" is the whole balance - the fee is not
    // known yet, so there is nothing to subtract.
    assert_eq!(
        neko_core::Amount::parse(st.amount.value(), 18).unwrap().raw,
        BALANCE
    );

    let new = st
        .hold_back_fee(BALANCE, Amount::new(FEE, 18))
        .expect("the fee should have been held back");
    assert_eq!(new + FEE, BALANCE, "amount + fee must be the balance");

    // And the field the request is built from says the same thing.
    assert_eq!(
        neko_core::Amount::parse(st.amount.value(), 18).unwrap().raw,
        new
    );
    assert_eq!(st.held_back.map(|f| f.raw), Some(FEE));
}

/// The quote decides affordability from the amount it was handed, so a reduced
/// amount that is not fed back leaves the screen still saying "not enough".
#[test]
fn the_reduced_amount_is_affordable() {
    let mut q = bsc_quote(BALANCE as u128, Some(BALANCE as u128));
    let FeeQuote::Bsc(b) = &q else { unreachable!() };
    assert_eq!(b.affordable(), Some(false), "the setup must start short");

    let mut st = state(neko_core::Asset::Bnb, Some(BALANCE));
    st.request_max();
    let new = st.hold_back_fee(BALANCE, q.total()).unwrap();
    q.set_amount(new);

    let FeeQuote::Bsc(b) = &q else { unreachable!() };
    assert_eq!(b.affordable(), Some(true), "still short after reducing");
    assert_eq!(b.bnb_needed(), BALANCE as u128, "it should use every wei");
}

/// A token's fee is paid in the chain's own coin, out of a different balance.
/// Holding any of it back would strand tokens for no reason.
#[test]
fn a_token_sends_its_whole_balance() {
    let usdt = neko_core::ChainId::Bsc.usdt().unwrap();
    let held = 250_000_000_000_000_000_000i128; // 250 USDT, 18 decimals
    let mut st = state(usdt, Some(held));
    st.request_max();
    assert_eq!(
        neko_core::Amount::parse(st.amount.value(), 18).unwrap().raw,
        held
    );

    // Even asked to, it holds nothing back: the fee is not in this balance.
    assert_eq!(st.hold_back_fee(held, Amount::new(FEE, 18)), None);
    assert!(st.held_back.is_none());
    assert_eq!(
        neko_core::Amount::parse(st.amount.value(), 18).unwrap().raw,
        held
    );
}

/// An unknown balance has no maximum. Offering one would mean guessing at how
/// much money somebody has.
#[test]
fn an_unknown_balance_offers_no_maximum() {
    let mut st = state(neko_core::Asset::Bnb, None);
    st.request_max();
    assert!(st.amount.is_empty(), "an amount was invented from nothing");
    assert!(!st.max_requested);
}

/// When the fee is the whole balance there is nothing to send. Rewriting the
/// amount to zero would replace the screen's explanation with a puzzle, so the
/// typed amount stays and the shortfall line does the talking.
#[test]
fn a_fee_larger_than_the_balance_leaves_the_amount_alone() {
    for fee in [BALANCE, BALANCE + 1] {
        let mut st = state(neko_core::Asset::Bnb, Some(BALANCE));
        st.request_max();
        assert_eq!(st.hold_back_fee(BALANCE, Amount::new(fee, 18)), None);
        assert!(st.held_back.is_none());
        assert_eq!(
            neko_core::Amount::parse(st.amount.value(), 18).unwrap().raw,
            BALANCE,
            "the typed amount should have been left as it was"
        );
    }
}

/// TRON burns TRX for what the account's resources cannot cover, and that burn
/// comes out of the same balance a TRX transfer spends.
#[test]
fn a_tron_maximum_holds_back_the_burn() {
    let burn = FeeQuote::Tron(TronFee {
        energy_base: 0,
        energy_penalty: 0,
        bandwidth_needed: 345,
        available: Some(((0, 0), (0, 600))),
        prices: Some((100, 1000)),
        recipient_is_new: false,
    })
    .total();
    assert_eq!(burn.raw, 345_000, "345 bytes x 1000 sun");

    let balance = 8_655_008i128; // 8.655008 TRX
    let mut st = state(neko_core::Asset::Trx, Some(balance));
    st.request_max();
    let new = st.hold_back_fee(balance, burn).unwrap();
    assert_eq!(new, balance - 345_000);
    assert_eq!(
        neko_core::Amount::parse(st.amount.value(), 6).unwrap().raw,
        new
    );
}

/// Once the amount is edited by hand it is no longer "everything", and a later
/// quote must not silently rewrite what was typed.
#[test]
fn editing_by_hand_withdraws_the_request() {
    let mut st = state(neko_core::Asset::Bnb, Some(BALANCE));
    st.request_max();
    assert!(st.max_requested);

    // What the key handler does on any edit.
    st.max_requested = false;
    st.held_back = None;
    st.amount.clear();
    "0.00001".chars().for_each(|c| st.amount.push(c));

    assert_eq!(st.hold_back_fee(BALANCE, Amount::new(FEE, 18)), None);
    assert_eq!(
        st.amount.value(),
        "0.00001",
        "a typed amount was overwritten"
    );
}

// ── The whole path, not just the arithmetic ────────────────────────────────
//
// The unit tests above prove the subtraction. These prove the wiring: that the
// key reaches it, and that the request built for signing carries the reduced
// amount rather than the one that was typed.

use crossterm::event::{KeyCode, KeyEvent};
use neko_tui::app::{App, Screen};
use neko_tui::send::SendStep;

fn channel() -> tokio::sync::mpsc::UnboundedSender<neko_tui::event::AppEvent> {
    tokio::sync::mpsc::unbounded_channel().0
}

fn app_entering_amount(asset: neko_core::Asset, balance: i128) -> App {
    let mut st = state(asset, Some(balance));
    st.step = SendStep::EnterAmount;
    let mut app = App::new(std::path::PathBuf::from("/tmp/neko-send-max.db"));
    app.screen = Screen::Send(Box::new(st));
    app
}

#[test]
fn pressing_m_asks_for_everything() {
    let mut app = app_entering_amount(neko_core::Asset::Bnb, BALANCE);
    neko_tui::keys::on_key_send(&mut app, KeyEvent::from(KeyCode::Char('m')), &channel());

    let Screen::Send(st) = &app.screen else {
        unreachable!()
    };
    assert!(st.max_requested);
    assert_eq!(
        neko_core::Amount::parse(st.amount.value(), 18).unwrap().raw,
        BALANCE
    );
}

/// `m` must not be swallowed on the recipient step - base58 addresses contain
/// it, so there it is an ordinary character.
#[test]
fn m_still_types_into_a_tron_address() {
    let mut st = state(neko_core::Asset::Trx, Some(1));
    st.to.clear();
    st.step = SendStep::Recipient;
    let mut app = App::new(std::path::PathBuf::from("/tmp/neko-send-max.db"));
    app.screen = Screen::Send(Box::new(st));

    neko_tui::keys::on_key_send(&mut app, KeyEvent::from(KeyCode::Char('m')), &channel());
    let Screen::Send(st) = &app.screen else {
        unreachable!()
    };
    assert_eq!(st.to.value(), "m", "the address field lost a character");
}

/// End to end: press the key, let the quote arrive, and check the request that
/// would be signed.
#[test]
fn the_request_built_for_signing_carries_the_reduced_amount() {
    let mut app = app_entering_amount(neko_core::Asset::Bnb, BALANCE);
    neko_tui::keys::on_key_send(&mut app, KeyEvent::from(KeyCode::Char('m')), &channel());

    let id = app.next_req();
    app.inflight = Some(id);
    app.on_app_event(neko_tui::event::AppEvent::Quoted {
        req: id,
        res: Ok(Box::new(neko_tui::event::Quote::Bsc {
            params: neko_evm::tx::TxParams {
                nonce: 0,
                gas_price: GAS_PRICE,
                gas_limit: GAS_LIMIT,
                chain_id: neko_evm::CHAIN_ID,
            },
            bnb_balance: Some(BALANCE as u128),
            sending_native: true,
            amount: BALANCE as u128,
        })),
    });

    let Screen::Send(st) = &app.screen else {
        unreachable!()
    };
    let SendStep::Review { req, quote, .. } = &st.step else {
        panic!("not at review: the quote did not land");
    };
    assert_eq!(
        req.amount.raw,
        BALANCE - FEE,
        "the signed amount is not the maximum"
    );
    assert_eq!(
        req.amount.raw + FEE,
        BALANCE,
        "it does not empty the account"
    );
    let Some(FeeQuote::Bsc(b)) = quote else {
        panic!("no BSC quote")
    };
    assert_eq!(
        b.affordable(),
        Some(true),
        "the screen would still say there is not enough"
    );
    assert_eq!(st.held_back.map(|f| f.raw), Some(FEE));
}
