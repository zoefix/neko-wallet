//! Choosing which coins to spend, and what that costs.
//!
//! This is the part of Bitcoin with no counterpart on an account chain, and the
//! part where a mistake is silent. Three facts drive all of it:
//!
//! * **The fee is inputs minus outputs.** Nothing declares it. Forgetting the
//!   change output does not fail - it hands the entire remainder to a miner,
//!   which is how people have lost tens of BTC on a single transfer.
//! * **The fee depends on the choice.** Each input adds about 68 virtual bytes,
//!   so selecting coins to cover a fee changes the fee. It has to be solved by
//!   recomputing as coins are added, not by estimating once.
//! * **Small outputs are refused.** Below the dust threshold an output costs
//!   more to spend than it holds, and the network will not relay it. Change
//!   that falls under it cannot be created, and has to go to the fee instead -
//!   which shrinks the transaction, which lowers the fee again.

use neko_hd::BtcAddress;

use crate::error::BtcError;
use crate::tx::Utxo;

/// A P2WPKH input: 32-byte txid, 4-byte index, an empty scriptSig, 4-byte
/// sequence, and a witness of a ~72-byte signature and a 33-byte key which
/// counts a quarter.
pub const INPUT_VBYTES: usize = 68;

/// Version, both counts, locktime, and the segwit marker and flag at a
/// quarter each. Rounded up, because a fee a satoshi short is a transaction
/// that does not relay.
pub const OVERHEAD_VBYTES: usize = 11;

/// The relay fee dust is measured against, in satoshis per thousand virtual
/// bytes. A consensus-adjacent constant: nodes will not forward an output
/// worth less than this makes it cost to spend.
const DUST_RELAY_FEE: u64 = 3_000;

/// A fee rate, in **thousandths** of a satoshi per virtual byte.
///
/// Not whole satoshis, and that is the whole reason this type exists. The
/// network quotes rates like 1.12; rounding that up to 2 before multiplying
/// makes a small transfer cost 79% more than it needs to. Rounding belongs on
/// the total, once, after the size is known - which is what `fee_for` does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FeeRate(u64);

impl FeeRate {
    /// The relay floor: one satoshi per virtual byte.
    ///
    /// Esplora will quote below this for distant targets - 0.36 at the time of
    /// writing - and a transaction at that rate is not slow, it is not
    /// forwarded at all.
    pub const MIN: FeeRate = FeeRate(1_000);

    pub fn from_milli(milli: u64) -> Self {
        FeeRate(milli.max(Self::MIN.0))
    }

    /// From the fractional figure a fee estimator returns.
    pub fn from_sat_per_vb(rate: f64) -> Self {
        if !rate.is_finite() || rate <= 0.0 {
            return Self::MIN;
        }
        Self::from_milli((rate * 1_000.0).round() as u64)
    }

    pub fn milli(self) -> u64 {
        self.0
    }

    /// What a transaction of this size costs. Rounded up, so the fee is never
    /// a satoshi under what the network asks.
    pub fn fee_for(self, vbytes: usize) -> u64 {
        (vbytes as u64).saturating_mul(self.0).div_ceil(1_000)
    }

    /// For the screen: `1.12`, not `1.120` and not `2`.
    pub fn to_display_string(self) -> String {
        let whole = self.0 / 1_000;
        let frac = self.0 % 1_000;
        if frac == 0 {
            return whole.to_string();
        }
        format!("{whole}.{:03}", frac)
            .trim_end_matches('0')
            .to_string()
    }
}

/// What it costs to spend an output of this kind, in virtual bytes. Used only
/// for the dust calculation, which asks whether an output is worth its own
/// future input.
fn spend_vbytes(script_pubkey: &[u8]) -> u64 {
    let witness = matches!(script_pubkey.first(), Some(0x00) | Some(0x51))
        && (script_pubkey.len() == 22 || script_pubkey.len() == 34);
    if witness {
        // 32 + 4 + 1 + 107/4 + 4
        67
    } else {
        // 32 + 4 + 1 + 107 + 4
        148
    }
}

/// An output smaller than this will not be relayed.
///
/// Bitcoin Core's rule, and the two figures everyone quotes fall out of it:
/// 294 satoshis for a P2WPKH output, 546 for a P2PKH one.
pub fn dust_threshold(script_pubkey: &[u8]) -> u64 {
    let size = 8
        + crate::varint::len(script_pubkey.len() as u64) as u64
        + script_pubkey.len() as u64
        + spend_vbytes(script_pubkey);
    size * DUST_RELAY_FEE / 1_000
}

/// The virtual size an output of this script adds.
pub fn output_vbytes(script_pubkey: &[u8]) -> usize {
    8 + crate::varint::len(script_pubkey.len() as u64) + script_pubkey.len()
}

/// What a transaction of this shape will weigh.
pub fn estimate_vbytes(inputs: usize, outputs: &[&[u8]]) -> usize {
    OVERHEAD_VBYTES
        + inputs * INPUT_VBYTES
        + outputs.iter().map(|s| output_vbytes(s)).sum::<usize>()
}

/// The coins to spend, and what happens to the remainder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub inputs: Vec<Utxo>,
    /// What goes back to us. `None` when the remainder was below dust and went
    /// to the fee instead - stated rather than silently folded in, because the
    /// fee is then larger than the rate would suggest and the screen has to say
    /// so.
    pub change: Option<u64>,
    pub fee: u64,
    pub vbytes: usize,
    /// True when change was dropped for being dust. The difference lands in the
    /// fee, and somebody comparing the fee to the rate deserves to know why.
    pub change_was_dust: bool,
}

impl Selection {
    pub fn input_total(&self) -> u64 {
        self.inputs.iter().map(|u| u.value).sum()
    }
}

/// Choose coins to pay `amount` to `to`, at `fee_rate` satoshis per virtual
/// byte, with change returning to `change_to`.
///
/// Largest first. It is not the cleverest strategy - it leaves more small coins
/// behind than branch-and-bound would - but it uses the fewest inputs, which is
/// the cheapest transaction today, and it is simple enough to be obviously
/// correct. Being obviously correct is worth more here than being optimal.
pub fn select(
    utxos: &[Utxo],
    to: &BtcAddress,
    amount: u64,
    change_to: &BtcAddress,
    fee_rate: FeeRate,
) -> Result<Selection, BtcError> {
    let to_script = to.script_pubkey();
    let change_script = change_to.script_pubkey();

    let dust = dust_threshold(&to_script);
    if amount < dust {
        return Err(BtcError::Dust(amount, dust));
    }

    // Largest first, and confirmed coins ahead of unconfirmed ones of the same
    // size: spending an unconfirmed output makes this transaction depend on
    // one that could still be replaced.
    let mut pool: Vec<Utxo> = utxos.to_vec();
    pool.sort_by(|a, b| {
        b.block_height
            .is_some()
            .cmp(&a.block_height.is_some())
            .then(b.value.cmp(&a.value))
    });

    let available: u64 = pool.iter().map(|u| u.value).sum();
    let mut chosen: Vec<Utxo> = Vec::new();
    let mut total: u64 = 0;

    for utxo in pool {
        total = total.saturating_add(utxo.value);
        chosen.push(utxo);

        // Recomputed every round: each input just added about 68 vB to the
        // fee it is helping to cover.
        let vb_change = estimate_vbytes(chosen.len(), &[&to_script, &change_script]);
        let with_change = fee_rate.fee_for(vb_change);
        if total < amount + with_change {
            continue;
        }

        let change = total - amount - with_change;
        if change >= dust_threshold(&change_script) {
            return Ok(Selection {
                vbytes: vb_change,
                inputs: chosen,
                change: Some(change),
                fee: with_change,
                change_was_dust: false,
            });
        }

        // Change too small to create. Dropping the output makes the
        // transaction smaller, so the fee is recomputed - and everything left
        // over becomes fee, because there is nowhere else for it to go.
        let vb_plain = estimate_vbytes(chosen.len(), &[&to_script]);
        let without_change = fee_rate.fee_for(vb_plain);
        if total >= amount + without_change {
            return Ok(Selection {
                vbytes: vb_plain,
                inputs: chosen,
                change: None,
                fee: total - amount,
                change_was_dust: true,
            });
        }
    }

    let needed = amount
        + fee_rate.fee_for(estimate_vbytes(
            chosen.len().max(1),
            &[&to_script, &change_script],
        ));
    Err(BtcError::NotEnough { needed, available })
}

/// Spend everything, with no change.
///
/// The one case where the amount is an output of the calculation rather than an
/// input to it: every coin goes in, and the amount is whatever is left after
/// the fee for spending all of them.
pub fn select_all(
    utxos: &[Utxo],
    to: &BtcAddress,
    fee_rate: FeeRate,
) -> Result<(Selection, u64), BtcError> {
    if utxos.is_empty() {
        return Err(BtcError::NotEnough {
            needed: 0,
            available: 0,
        });
    }
    let to_script = to.script_pubkey();
    let total: u64 = utxos.iter().map(|u| u.value).sum();
    let vbytes = estimate_vbytes(utxos.len(), &[&to_script]);
    let fee = fee_rate.fee_for(vbytes);

    let amount = total.checked_sub(fee).ok_or(BtcError::NotEnough {
        needed: fee,
        available: total,
    })?;
    let dust = dust_threshold(&to_script);
    if amount < dust {
        return Err(BtcError::Dust(amount, dust));
    }

    Ok((
        Selection {
            inputs: utxos.to_vec(),
            change: None,
            fee,
            vbytes,
            change_was_dust: false,
        },
        amount,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tx::OutPoint;

    fn mine() -> BtcAddress {
        BtcAddress::parse("bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu").unwrap()
    }
    fn them() -> BtcAddress {
        BtcAddress::parse("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").unwrap()
    }
    fn legacy() -> BtcAddress {
        BtcAddress::parse("1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa").unwrap()
    }

    /// A whole-satoshi rate, which is what most of these tests want.
    fn rate(sat_per_vb: u64) -> FeeRate {
        FeeRate::from_milli(sat_per_vb * 1_000)
    }

    fn utxo(n: u8, value: u64) -> Utxo {
        Utxo {
            outpoint: OutPoint {
                txid: [n; 32],
                vout: 0,
            },
            value,
            script_pubkey: mine().script_pubkey(),
            block_height: Some(800_000),
        }
    }

    /// The rate is fractional, and rounding it before multiplying is most of a
    /// small transfer's fee.
    ///
    /// The live estimate when this was written was 1.12 sat/vB. Rounded up to
    /// 2 first, a 141-byte transfer costs 282 satoshis; rounded once at the
    /// end it costs 158. That is 79% more, for nothing, on every transfer while
    /// the mempool is quiet.
    #[test]
    fn a_fractional_rate_is_not_rounded_before_it_is_multiplied() {
        let r = FeeRate::from_sat_per_vb(1.12);
        assert_eq!(r.milli(), 1_120);
        assert_eq!(r.fee_for(141), 158, "the rate was rounded too early");
        assert_eq!(r.to_display_string(), "1.12");

        // Whole rates still read as whole numbers.
        assert_eq!(FeeRate::from_sat_per_vb(2.0).to_display_string(), "2");
        assert_eq!(FeeRate::from_sat_per_vb(2.0).fee_for(141), 282);
        assert_eq!(FeeRate::from_sat_per_vb(12.5).to_display_string(), "12.5");
    }

    /// The total rounds up, so the fee is never a satoshi under what the
    /// network asks - which would be a transaction that does not relay.
    #[test]
    fn the_total_rounds_up_not_down() {
        let r = FeeRate::from_sat_per_vb(1.001);
        // 141 x 1.001 = 141.141, so 142.
        assert_eq!(r.fee_for(141), 142);
        assert_eq!(FeeRate::from_sat_per_vb(1.0).fee_for(141), 141);
    }

    /// Esplora quotes below one satoshi per byte for distant targets - 0.36 at
    /// the time of writing. A transaction at that rate is not slow; no node
    /// forwards it.
    #[test]
    fn the_relay_floor_is_never_gone_under() {
        for quoted in [0.36, 0.5, 0.999, 0.0, -1.0, f64::NAN] {
            assert_eq!(
                FeeRate::from_sat_per_vb(quoted),
                FeeRate::MIN,
                "{quoted} was accepted as a rate"
            );
        }
        assert_eq!(FeeRate::MIN.milli(), 1_000);
        assert_eq!(FeeRate::MIN.fee_for(141), 141);
    }

    /// The two figures every Bitcoin wallet quotes, and this derives rather
    /// than hardcodes. Matching both from one formula is what says the formula
    /// is the right one.
    #[test]
    fn the_dust_thresholds_are_the_known_ones() {
        assert_eq!(dust_threshold(&them().script_pubkey()), 294, "P2WPKH");
        assert_eq!(dust_threshold(&legacy().script_pubkey()), 546, "P2PKH");
        let taproot =
            BtcAddress::parse("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0")
                .unwrap();
        assert_eq!(dust_threshold(&taproot.script_pubkey()), 330, "P2TR");
    }

    /// The fee is a function of the choice, so covering it has to be
    /// recomputed as coins are added. Selecting against the first estimate
    /// leaves the transaction short by 68 vB per extra input.
    #[test]
    fn the_fee_covers_the_inputs_it_took_to_pay_it() {
        // Four coins that individually cannot pay, and together only just can.
        let pool: Vec<Utxo> = (0..4).map(|i| utxo(i, 30_000)).collect();
        let s = select(&pool, &them(), 100_000, &mine(), rate(10)).unwrap();

        assert!(s.inputs.len() >= 4, "should have needed every coin");
        let out = 100_000 + s.change.unwrap_or(0);
        assert_eq!(
            s.input_total() - out,
            s.fee,
            "inputs minus outputs is not the stated fee"
        );
        // And the fee actually pays for the size it chose.
        let to_s = them().script_pubkey();
        let change_s = mine().script_pubkey();
        let scripts: Vec<&[u8]> = if s.change.is_some() {
            vec![&to_s, &change_s]
        } else {
            vec![&to_s]
        };
        let vb = estimate_vbytes(s.inputs.len(), &scripts);
        assert!(
            s.fee >= rate(10).fee_for(vb),
            "fee {} under-pays {vb} vB",
            s.fee
        );
    }

    /// Change below dust cannot be created. It has to go to the fee, and the
    /// screen has to be told it did - otherwise the fee looks inexplicably
    /// higher than the rate.
    #[test]
    fn change_below_dust_becomes_fee_and_says_so() {
        // Funded so that after the amount and the *two-output* fee - which is
        // what selection tries first - a hundred satoshis are left over, well
        // under the 294 needed to create a change output.
        let to_s = them().script_pubkey();
        let change_s = mine().script_pubkey();
        let with_change = rate(5).fee_for(estimate_vbytes(1, &[&to_s, &change_s]));
        let pool = vec![utxo(1, 100_000 + with_change + 100)];
        let s = select(&pool, &them(), 100_000, &mine(), rate(5)).unwrap();
        let vb = rate(5).fee_for(estimate_vbytes(1, &[&to_s]));

        assert!(s.change.is_none(), "dust change was created");
        assert!(s.change_was_dust, "the reason was not reported");
        assert_eq!(s.fee, s.input_total() - 100_000);
        assert!(s.fee > vb, "the remainder did not go to the fee");
    }

    /// Every satoshi has to be accounted for. On this chain the alternative to
    /// change is not an error - it is a very large fee.
    #[test]
    fn nothing_is_ever_unaccounted_for() {
        for (coins, amount, rate_sat) in [
            (vec![50_000u64], 10_000u64, 1u64),
            (vec![50_000], 10_000, 20),
            (vec![100_000, 100_000, 100_000], 250_000, 8),
            (vec![1_000_000], 999_000, 3),
            (vec![600_000, 5_000, 4_000], 500_000, 15),
        ] {
            let pool: Vec<Utxo> = coins
                .iter()
                .enumerate()
                .map(|(i, v)| utxo(i as u8, *v))
                .collect();
            let s = select(&pool, &them(), amount, &mine(), rate(rate_sat)).unwrap();
            assert_eq!(
                s.input_total(),
                amount + s.change.unwrap_or(0) + s.fee,
                "coins {coins:?} amount {amount} rate {rate_sat}: satoshis went missing"
            );
        }
    }

    /// Fewer inputs is a cheaper transaction, so the biggest coins go first.
    #[test]
    fn the_largest_coins_are_taken_first() {
        let pool = vec![utxo(1, 10_000), utxo(2, 500_000), utxo(3, 20_000)];
        let s = select(&pool, &them(), 100_000, &mine(), rate(5)).unwrap();
        assert_eq!(s.inputs.len(), 1);
        assert_eq!(s.inputs[0].value, 500_000);
    }

    /// An unconfirmed coin can still be replaced, so spending it makes this
    /// transaction depend on one that might not survive. Equal-value coins are
    /// therefore not equal.
    #[test]
    fn confirmed_coins_are_preferred() {
        let mut unconfirmed = utxo(1, 500_000);
        unconfirmed.block_height = None;
        let confirmed = utxo(2, 500_000);
        let s = select(
            &[unconfirmed, confirmed.clone()],
            &them(),
            100_000,
            &mine(),
            rate(5),
        )
        .unwrap();
        assert_eq!(s.inputs[0], confirmed, "an unconfirmed coin was preferred");
    }

    /// Not enough is not enough, and the message has to name the fee - which is
    /// the part somebody cannot work out for themselves on this chain.
    #[test]
    fn a_shortfall_names_what_was_actually_needed() {
        let pool = vec![utxo(1, 50_000)];
        match select(&pool, &them(), 100_000, &mine(), rate(10)) {
            Err(BtcError::NotEnough { needed, available }) => {
                assert_eq!(available, 50_000);
                assert!(needed > 100_000, "the fee was left out of the shortfall");
            }
            other => panic!("expected a shortfall, got {other:?}"),
        }

        // Covering the amount but not the fee is the case people hit, and it
        // has to fail rather than silently underpay.
        let pool = vec![utxo(1, 100_050)];
        assert!(matches!(
            select(&pool, &them(), 100_000, &mine(), rate(50)),
            Err(BtcError::NotEnough { .. })
        ));
    }

    /// An amount the network will not relay is refused here, where it can be
    /// explained, rather than by a node after broadcast.
    #[test]
    fn a_dust_payment_is_refused() {
        let pool = vec![utxo(1, 1_000_000)];
        assert!(matches!(
            select(&pool, &them(), 293, &mine(), rate(5)),
            Err(BtcError::Dust(293, 294))
        ));
        // A legacy recipient has a higher floor, and it has to be their floor.
        assert!(matches!(
            select(&pool, &legacy(), 400, &mine(), rate(5)),
            Err(BtcError::Dust(400, 546))
        ));
        assert!(select(&pool, &them(), 294, &mine(), rate(5)).is_ok());
    }

    /// Sending everything has no change: the amount is what is left after
    /// paying for a transaction that spends every coin.
    #[test]
    fn sending_everything_leaves_nothing_behind() {
        let pool: Vec<Utxo> = (0..3).map(|i| utxo(i, 100_000)).collect();
        let (s, amount) = select_all(&pool, &them(), rate(12)).unwrap();

        assert_eq!(s.inputs.len(), 3, "a coin was left behind");
        assert!(s.change.is_none());
        assert_eq!(amount + s.fee, 300_000, "satoshis went missing");
        assert_eq!(s.fee, rate(12).fee_for(s.vbytes));
    }

    /// A wallet whose whole balance is smaller than the fee to move it cannot
    /// send, and has to be told that rather than shown a negative amount.
    #[test]
    fn everything_can_be_less_than_the_fee() {
        let pool = vec![utxo(1, 500)];
        assert!(matches!(
            select_all(&pool, &them(), rate(100)),
            Err(BtcError::NotEnough { .. })
        ));
        // Or leave an amount too small to relay: 110 vB at 10 sat/vB is a
        // 1,100 satoshi fee, so 1,350 leaves 250 - under the 294 floor.
        let pool = vec![utxo(1, 1_350)];
        assert!(matches!(
            select_all(&pool, &them(), rate(10)),
            Err(BtcError::Dust(250, 294))
        ));
    }
}
