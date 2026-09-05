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
use neko_tui::send::{EvmFee, FeeQuote, SendState, TronFee};

const BSC_MINE: &str = "0x1111111111111111111111111111111111111111";
const BSC_TO: &str = "0x2222222222222222222222222222222222222222";
const SOLANA_MINE: &str = "5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9";
const TON_MINE: &str = "EQAzWZa6nM5mJev91wGc7VCSfBoIsYRqKJpV78N8Add9-U9d";
const TON_TO: &str = "EQDVJucJT96vGh_bYm3e5uzenasiTOwA9orUHQiyhNsKmEcK";
const SOLANA_TO: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
const BTC_TO: &str = "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4";
const TRON_TO: &str = "TNYxHL2s6Wjpx86NRwhekYzc27p3oDYrk6";
const BTC_MINE: &str = "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu";
const TRON_MINE: &str = "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH";

/// The balance from the screenshot that started this: 0.00008488 BNB, and a
/// 25,200 x 0.05 gwei fee that leaves it 0.00000126 BNB short of sending it all.
const BALANCE: i128 = 84_880_000_000_000;
const GAS_LIMIT: u64 = 25_200;
const GAS_PRICE: u128 = 50_000_000;
const FEE: i128 = GAS_LIMIT as i128 * GAS_PRICE as i128; // 1_260_000_000_000

fn state(asset: neko_core::Asset, balance: Option<i128>) -> SendState {
    // Both ends have to belong to the chain being tested. A destination from
    // the wrong chain does not fail loudly here - the request simply cannot be
    // built, and the test sees a quote that never arrives.
    let (mine, to) = match asset.chain() {
        neko_core::ChainId::Tron => (TRON_MINE, TRON_TO),
        neko_core::ChainId::Bsc => (BSC_MINE, BSC_TO),
        neko_core::ChainId::Solana => (SOLANA_MINE, SOLANA_TO),
        neko_core::ChainId::Bitcoin => (BTC_MINE, BTC_TO),
        neko_core::ChainId::Ton => (TON_MINE, TON_TO),
        // The same twenty bytes as BNB Chain's, which is the point.
        neko_core::ChainId::Ethereum => (BSC_MINE, BSC_TO),
    };
    let mut st = SendState::new(
        1,
        "w".into(),
        neko_core::ChainAddress::parse(asset.chain(), mine).unwrap(),
        asset,
        asset.symbol().to_string(),
    );
    to.chars().for_each(|c| st.to.push(c));
    st.balance = balance;
    st
}

fn bsc_quote(amount: u128, balance: Option<u128>) -> FeeQuote {
    FeeQuote::Evm(EvmFee {
        chain: neko_evm::BSC,
        gas_limit: GAS_LIMIT,
        fees: neko_evm::tx::Fees::Legacy {
            gas_price: GAS_PRICE,
        },
        native_balance: balance,
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
    let FeeQuote::Evm(b) = &q else { unreachable!() };
    assert_eq!(b.affordable(), Some(false), "the setup must start short");

    let mut st = state(neko_core::Asset::Bnb, Some(BALANCE));
    st.request_max();
    let new = st.hold_back_fee(BALANCE, q.total()).unwrap();
    q.set_amount(new);

    let FeeQuote::Evm(b) = &q else { unreachable!() };
    assert_eq!(b.affordable(), Some(true), "still short after reducing");
    assert_eq!(
        b.native_needed(),
        BALANCE as u128,
        "it should use every wei"
    );
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
        res: Ok(Box::new(neko_tui::event::Quote::Evm {
            chain: neko_evm::BSC,
            params: neko_evm::tx::TxParams {
                nonce: 0,
                gas_limit: GAS_LIMIT,
                chain_id: neko_evm::BSC.chain_id,
                fees: neko_evm::tx::Fees::Legacy {
                    gas_price: GAS_PRICE,
                },
            },
            native_balance: Some(BALANCE as u128),
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
    let Some(q) = quote else { panic!("no quote") };
    let FeeQuote::Evm(b) = &**q else {
        panic!("no BSC quote")
    };
    assert_eq!(
        b.affordable(),
        Some(true),
        "the screen would still say there is not enough"
    );
    assert_eq!(st.held_back.map(|f| f.raw), Some(FEE));
}

// ── EIP-1559 reserves a ceiling, not a price ───────────────────────────────

/// The exact transfer that failed: 0.001927234996653140 ETH, sent in full,
/// rejected by the node with `have 1927234996653140 want 1930394086141940`.
///
/// EIP-1559 makes the chain check the balance against `gas_limit x
/// max_fee_per_gas` - the whole ceiling - even though only the base fee plus
/// the tip is charged and the difference is refunded within the same block.
/// Holding back the *expected* cost therefore leaves an amount the node refuses
/// outright, and it is off by the entire refundable headroom.
const ETH_BALANCE: i128 = 1_927_234_996_653_140;
const ETH_GAS_LIMIT: u64 = 25_200;
/// base + tip, what the transfer is expected to cost per unit of gas. Derived
/// from what the wallet actually held back that day - `have - amount` over the
/// gas limit - rather than from the screen, which truncates.
const ETH_EXPECTED_PER_GAS: u128 = 124_722_225;
/// Roughly twice that: the quote asks for double the base fee plus the tip, so
/// the ceiling covers six consecutive full blocks. The precise figure from that
/// transfer cannot be reconstructed from what was on screen, and pinning a
/// reverse-engineered one would be false precision - the property below is what
/// matters and it holds for any ceiling above the price.
const ETH_MAX_PER_GAS: u128 = 249_444_450;

fn eth_quote(amount: u128) -> FeeQuote {
    FeeQuote::Evm(neko_tui::send::EvmFee {
        chain: neko_evm::ETHEREUM,
        gas_limit: ETH_GAS_LIMIT,
        fees: neko_evm::tx::Fees::Eip1559 {
            max_fee_per_gas: ETH_MAX_PER_GAS,
            max_priority_fee_per_gas: 0,
            base_fee: ETH_EXPECTED_PER_GAS,
        },
        native_balance: Some(ETH_BALANCE as u128),
        sending_native: true,
        amount,
    })
}

/// The two figures are different, and which one is used decides whether the
/// transfer is accepted at all.
#[test]
fn the_reserve_is_the_ceiling_and_the_total_is_the_price() {
    let q = eth_quote(0);
    let expected = ETH_GAS_LIMIT as i128 * ETH_EXPECTED_PER_GAS as i128;
    let ceiling = ETH_GAS_LIMIT as i128 * ETH_MAX_PER_GAS as i128;

    assert_eq!(q.total().raw, expected, "the screen should show the price");
    assert_eq!(q.reserve().raw, ceiling, "the balance check is the ceiling");
    assert!(q.reserve().raw > q.total().raw);

    // On a chain with one gas price the two are the same number, and nothing
    // extra is held back.
    let legacy = FeeQuote::Evm(neko_tui::send::EvmFee {
        chain: neko_evm::BSC,
        gas_limit: GAS_LIMIT,
        fees: neko_evm::tx::Fees::Legacy {
            gas_price: GAS_PRICE,
        },
        native_balance: Some(BALANCE as u128),
        sending_native: true,
        amount: 0,
    });
    assert_eq!(legacy.reserve().raw, legacy.total().raw);
}

/// Sending everything has to leave an amount the node will take.
#[test]
fn sending_all_of_an_eip1559_balance_is_accepted() {
    let mut q = eth_quote(ETH_BALANCE as u128);
    let FeeQuote::Evm(e) = &q else { unreachable!() };
    assert_eq!(
        e.affordable(),
        Some(false),
        "the setup must start unaffordable"
    );

    let mut st = state(neko_core::Asset::Eth, Some(ETH_BALANCE));
    st.request_max();
    let max = st.hold_back_fee(ETH_BALANCE, q.reserve()).unwrap();
    q.set_amount(max);

    let FeeQuote::Evm(e) = &q else { unreachable!() };
    assert_eq!(
        e.affordable(),
        Some(true),
        "still short after holding back the fee"
    );
    // What the node checks: value + gas_limit x max_fee_per_gas.
    assert_eq!(
        e.native_needed(),
        ETH_BALANCE as u128,
        "the reserve should use the balance exactly"
    );

    // The figure the node refused, reproduced: reserving the expected cost
    // instead leaves an amount that needs more than the balance holds.
    let mut wrong = state(neko_core::Asset::Eth, Some(ETH_BALANCE));
    wrong.request_max();
    let bad = wrong.hold_back_fee(ETH_BALANCE, q.total()).unwrap();
    let mut q2 = eth_quote(bad as u128);
    q2.set_amount(bad);
    let FeeQuote::Evm(e2) = &q2 else {
        unreachable!()
    };
    assert_eq!(
        e2.affordable(),
        Some(false),
        "reserving the price rather than the ceiling should still be short"
    );
    // Short by exactly the headroom, which is what the node's "want" exceeded
    // its "have" by.
    assert_eq!(
        e2.native_needed() - ETH_BALANCE as u128,
        (ETH_MAX_PER_GAS - ETH_EXPECTED_PER_GAS) * ETH_GAS_LIMIT as u128
    );
}

/// End to end, through the event that decides it.
///
/// The two tests above check the arithmetic and would both have passed while
/// the wallet was broken: the fault was in which of the two figures `app.rs`
/// handed to `hold_back_fee`, and calling it directly cannot see that. This
/// drives the quote through `on_app_event`, which is the path that failed.
#[test]
fn sending_everything_on_ethereum_produces_an_amount_the_node_accepts() {
    let mut app = app_entering_amount(neko_core::Asset::Eth, ETH_BALANCE);
    neko_tui::keys::on_key_send(&mut app, KeyEvent::from(KeyCode::Char('m')), &channel());

    let id = app.next_req();
    app.inflight = Some(id);
    app.on_app_event(neko_tui::event::AppEvent::Quoted {
        req: id,
        res: Ok(Box::new(neko_tui::event::Quote::Evm {
            chain: neko_evm::ETHEREUM,
            params: neko_evm::tx::TxParams {
                nonce: 0,
                gas_limit: ETH_GAS_LIMIT,
                chain_id: neko_evm::ETHEREUM.chain_id,
                fees: neko_evm::tx::Fees::Eip1559 {
                    max_fee_per_gas: ETH_MAX_PER_GAS,
                    max_priority_fee_per_gas: 0,
                    base_fee: ETH_EXPECTED_PER_GAS,
                },
            },
            native_balance: Some(ETH_BALANCE as u128),
            sending_native: true,
            amount: ETH_BALANCE as u128,
        })),
    });

    let Screen::Send(st) = &app.screen else {
        unreachable!()
    };
    let SendStep::Review { req, quote, .. } = &st.step else {
        panic!("not at review")
    };
    let Some(q) = quote else { panic!("no quote") };
    let FeeQuote::Evm(e) = &**q else {
        panic!("wrong chain")
    };

    assert_eq!(
        e.affordable(),
        Some(true),
        "the amount the wallet chose would be rejected by the node"
    );
    // Every wei accounted for: the amount plus the ceiling is the balance.
    assert_eq!(
        req.amount.raw as u128 + e.max_fee_wei(),
        ETH_BALANCE as u128
    );
    // And what was held back is the ceiling, not the price - which is the
    // difference the screen has to explain, since the excess comes back.
    assert_eq!(st.held_back.map(|f| f.raw), Some(e.max_fee_wei() as i128));
    assert!(e.max_fee_wei() > e.fee_wei());
}

// ── TON attaches coin that comes back, and still has to be held back ────────

/// Sending every GRAM has to reserve the attached coin as well as the fee.
///
/// This is the same shape as the EIP-1559 failure above, and it is worth
/// testing for the same reason: the attached coin is *mostly refunded*, so the
/// number the screen shows as the cost is smaller than the number the balance
/// has to cover. Reserving the shown figure leaves a transfer the wallet
/// contract cannot pay for - and, unlike a node rejecting a transaction, a
/// TON message that cannot pay is silently not executed.
///
/// Driven through `on_app_event` rather than the fee type directly, because
/// last time the arithmetic was right in both obvious places and the fault was
/// in which figure got passed between them.
#[test]
fn a_ton_maximum_holds_back_the_attached_coin_too() {
    const GRAM_BALANCE: i128 = 2_000_000_000; // 2 GRAM
    const TON_FEE: u128 = neko_ton::FEE_TRANSFER;
    const ATTACHED: u128 = neko_ton::JETTON_TRANSFER_ATTACHED;

    let mut app = app_entering_amount(neko_core::Asset::Gram, GRAM_BALANCE);
    neko_tui::keys::on_key_send(&mut app, KeyEvent::from(KeyCode::Char('m')), &channel());

    let id = app.next_req();
    app.inflight = Some(id);
    app.on_app_event(neko_tui::event::AppEvent::Quoted {
        req: id,
        res: Ok(Box::new(neko_tui::event::Quote::Ton {
            params: Box::new(neko_core::TonTxParams {
                seqno: 7,
                valid_until: 0,
                deploy: false,
                jetton_wallet: None,
            }),
            gram_balance: Some(GRAM_BALANCE as u128),
            sending_native: true,
            amount: GRAM_BALANCE as u128,
            fee: TON_FEE,
            // A plain GRAM transfer attaches nothing; the figure is set here
            // anyway, because what is being tested is that whatever is
            // reserved is what gets held back.
            attached: ATTACHED,
        })),
    });

    let Screen::Send(st) = &app.screen else {
        unreachable!()
    };
    let SendStep::Review { req, quote, .. } = &st.step else {
        panic!("not at review: the quote did not land");
    };
    let held = (TON_FEE + ATTACHED) as i128;
    assert_eq!(
        req.amount.raw,
        GRAM_BALANCE - held,
        "the fee alone was held back, not the coin that travels with the message"
    );
    assert_eq!(st.held_back.map(|f| f.raw), Some(held));

    let Some(q) = quote else { panic!("no quote") };
    let FeeQuote::Ton(t) = &**q else {
        panic!("not a TON quote")
    };
    assert_eq!(
        t.affordable(),
        Some(true),
        "the screen would still say there is not enough GRAM"
    );
    // And the reserve is deliberately larger than what the screen calls the
    // cost. Those being equal is what the EIP-1559 bug looked like.
    assert_eq!(q.total().raw, TON_FEE as i128);
    assert_eq!(q.reserve().raw, held);
}
