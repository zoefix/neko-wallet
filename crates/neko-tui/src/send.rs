//! The transfer flow.
//!
//! The confirmation is not a keypress. Clipboard-hijacking malware that swaps a
//! destination address is the dominant real-world loss vector for command-line
//! wallets, and pressing `y` does not defend against it: the user is looking at
//! what they *believe* they pasted. Retyping the last characters of the actual
//! destination is the only step that forces attention onto the bytes about to
//! be signed.

use neko_core::{Amount, Asset, ChainAddress, ChainId, ChainTxParams, TransferRequest};

use crate::input::Field;

/// How many trailing characters of the destination the user must retype.
///
/// This defends against clipboard hijacking, where malware swaps the address at
/// paste time and cannot choose its tail. It is *weaker* against address
/// poisoning, where the attacker picks their own address and can afford to
/// brute-force a matching suffix — which is why the lookalike check below
/// exists as a separate defence.
pub const CONFIRM_CHARS: usize = 6;

/// Base58 TRON addresses are always this long.
pub const TRON_ADDRESS_LEN: usize = 34;
/// `0x` plus 40 hex characters.
pub const EVM_ADDRESS_LEN: usize = 42;
/// Base58 of 32 bytes. The range is real rather than defensive: each leading
/// zero byte encodes to a single `1`, so an address with several of them is
/// genuinely shorter than one without.
pub const SOLANA_ADDRESS_MIN_LEN: usize = 32;
pub const SOLANA_ADDRESS_MAX_LEN: usize = 44;
/// Bitcoin's range spans two encodings and five script types.
pub const BTC_ADDRESS_MIN_LEN: usize = 26;
pub const BTC_ADDRESS_MAX_LEN: usize = 62;
/// A workchain byte, 32 bytes of hash and a CRC, in base64url. 36 bytes, which
/// divides by three, so there is no padding and no variation in the length.
pub const TON_ADDRESS_LEN: usize = 48;
/// `0x` plus at least one hex digit, and at most 64.
///
/// Not a single length, unlike TON's: Aptos prints addresses with leading
/// zeros removed, so `0x1` and a 66-character string are both complete.
const APTOS_ADDRESS_MIN_LEN: usize = 3;
const APTOS_ADDRESS_MAX_LEN: usize = 66;
/// `0x` and exactly 64 hex characters.
const SUI_ADDRESS_LEN: usize = 66;

/// The fee, broken down.
///
/// A bare "~23.9 TRX" is not actionable. TRON burns TRX only for the part of a
/// transfer the account cannot already cover, so the same transfer is free for
/// an account with staked energy and costly for one without. Showing what is
/// needed alongside what is held is what makes the number mean anything.
pub struct TronFee {
    /// What the contract call would cost without the surcharge.
    ///
    /// Derived by subtraction, from a node that reports the total and the
    /// surcharge within it - see [`neko_tron::EnergyEstimate`]. These two
    /// fields *are* addends, and the estimate's are not; keeping the split
    /// here is what lets the screen say where the number comes from.
    pub energy_base: i64,
    /// TRON's dynamic-energy surcharge on heavily used contracts. For USDT it
    /// is over three quarters of the charge - 49,635 of 64,285 - which is why
    /// the figure looks nothing like the numbers quoted in older documentation.
    pub energy_penalty: i64,
    pub bandwidth_needed: i64,
    /// `None` when the account's resources could not be read. Distinct from
    /// zero: one is a fact about the account, the other is our own failure.
    ///
    /// Each entry is `(available, limit)`. Both are shown, because the
    /// available figure regenerates continuously — roughly 0.75 energy per
    /// second for a 131k limit — so a bare number looks wrong the moment it is
    /// compared against another reading taken seconds later.
    pub available: Option<((i64, i64), (i64, i64))>,
    /// `None` when the chain's burn prices could not be read.
    pub prices: Option<(i64, i64)>,
    /// A first-time recipient pays to create a storage slot, which real mainnet
    /// receipts (2026-09) put at 130,285 energy against 64,285 for an address
    /// that already holds the token — roughly double, not the 15x quoted in
    /// older documentation. Both figures include the dynamic-energy surcharge,
    /// which is most of each.
    pub recipient_is_new: bool,
}

/// Fallback burn prices, used only for an upper-bound figure when the chain
/// could not be asked. The UI always says when these are in play.
pub const FALLBACK_SUN_PER_ENERGY: i64 = 210;
pub const FALLBACK_SUN_PER_BANDWIDTH: i64 = 1_000;

impl TronFee {
    pub fn energy_available(&self) -> Option<i64> {
        self.available.map(|(e, _)| e.0)
    }
    pub fn energy_limit(&self) -> Option<i64> {
        self.available.map(|(e, _)| e.1)
    }
    pub fn bandwidth_available(&self) -> Option<i64> {
        self.available.map(|(_, b)| b.0)
    }
    pub fn bandwidth_limit(&self) -> Option<i64> {
        self.available.map(|(_, b)| b.1)
    }
    pub fn resources_known(&self) -> bool {
        self.available.is_some()
    }
    pub fn prices_known(&self) -> bool {
        self.prices.is_some()
    }
    pub fn sun_per_energy(&self) -> i64 {
        self.prices
            .map(|(e, _)| e)
            .unwrap_or(FALLBACK_SUN_PER_ENERGY)
    }
    pub fn sun_per_bandwidth(&self) -> i64 {
        self.prices
            .map(|(_, b)| b)
            .unwrap_or(FALLBACK_SUN_PER_BANDWIDTH)
    }

    /// With holdings unknown, assume none: the resulting figure is an upper
    /// bound, which the UI labels as such rather than passing it off as exact.
    ///
    /// The surcharge is added *here*, to a base this side computed, rather than
    /// to the node's own figure - which already contains it.
    pub fn energy_needed(&self) -> i64 {
        self.energy_base + self.energy_penalty
    }
    pub fn energy_shortfall(&self) -> i64 {
        (self.energy_needed() - self.energy_available().unwrap_or(0)).max(0)
    }
    pub fn bandwidth_shortfall(&self) -> i64 {
        (self.bandwidth_needed - self.bandwidth_available().unwrap_or(0)).max(0)
    }
    pub fn energy_burn(&self) -> Amount {
        Amount::new(
            self.energy_shortfall() as i128 * self.sun_per_energy() as i128,
            6,
        )
    }
    pub fn bandwidth_burn(&self) -> Amount {
        Amount::new(
            self.bandwidth_shortfall() as i128 * self.sun_per_bandwidth() as i128,
            6,
        )
    }
    pub fn total_burn(&self) -> Amount {
        Amount::new(self.energy_burn().raw + self.bandwidth_burn().raw, 6)
    }
    /// Only claim a transfer is free when we actually know the holdings.
    pub fn is_free(&self) -> bool {
        self.resources_known() && self.total_burn().raw == 0
    }
    /// True when the total is a ceiling rather than a figure.
    pub fn is_upper_bound(&self) -> bool {
        !self.resources_known()
    }
}

/// What an EVM transfer costs, on either chain.
///
/// Nothing here resembles TRON's model and that is the point. There is no
/// allowance to draw down: gas is always paid, always in the chain's own coin,
/// at a price the node quotes. The consequence worth surfacing is that a wallet
/// holding only USDT cannot send that USDT - it has no coin for the fee - which
/// is a state people reach constantly and which an "insufficient funds" from
/// the node explains badly.
///
/// The two chains differ in one visible way: BNB Chain names a single gas
/// price, Ethereum names a ceiling and pays the base fee plus a tip. Showing
/// Ethereum's ceiling as though it were the price would say a transfer costs
/// about twice what it does, so both figures are carried and the screen shows
/// the one that is true.
pub struct EvmFee {
    /// Boxed: a dozen static strings, and an unboxed copy makes every other
    /// variant of `FeeQuote` as large as the heaviest.
    pub chain: Box<neko_evm::EvmChain>,
    pub gas_limit: u64,
    pub fees: neko_evm::tx::Fees,
    /// `None` when the balance could not be read. Distinct from zero: one is a
    /// fact about the account, the other is our own failure.
    pub native_balance: Option<u128>,
    /// Whether the amount and the fee come out of the same balance.
    pub sending_native: bool,
    pub amount: u128,
    /// What a rollup charges for posting this transaction to Ethereum, on top
    /// of L2 gas. Zero everywhere else.
    ///
    /// `op-geth` counts it in the balance check, so a "send everything" that
    /// leaves it out produces an amount the node refuses - which is exactly
    /// what Base did, by 434,251,659 wei.
    pub l1_fee: u128,
}

impl EvmFee {
    /// What this is expected to cost. For a legacy transaction it is exact;
    /// for a type-2 one it is the base fee plus the tip, which is what gets
    /// charged.
    pub fn fee_wei(&self) -> u128 {
        (self.gas_limit as u128 * self.fees.expected_per_gas()).saturating_add(self.l1_fee)
    }

    /// The most it can cost - the ceiling a balance has to cover even though
    /// the difference comes back. Equal to `fee_wei` on a legacy chain.
    ///
    /// The L1 fee is in both, and is not refundable: a rollup charges it for
    /// posting the transaction to Ethereum whatever the L2 gas turns out to
    /// cost.
    pub fn max_fee_wei(&self) -> u128 {
        (self.gas_limit as u128 * self.fees.max_per_gas()).saturating_add(self.l1_fee)
    }

    /// L2 gas alone, for the line that explains where the total came from.
    pub fn l2_fee_wei(&self) -> u128 {
        self.gas_limit as u128 * self.fees.expected_per_gas()
    }

    pub fn l1_fee_amount(&self) -> Amount {
        Amount::new(self.l1_fee as i128, self.chain.native_decimals)
    }

    pub fn fee(&self) -> Amount {
        Amount::new(self.fee_wei() as i128, self.chain.native_decimals)
    }

    pub fn max_fee(&self) -> Amount {
        Amount::new(self.max_fee_wei() as i128, self.chain.native_decimals)
    }

    /// What this transfer takes out of the coin balance: the ceiling, plus the
    /// amount itself when the amount *is* the coin.
    ///
    /// The ceiling rather than the expected cost, because that is what the
    /// chain checks before it will accept the transaction at all.
    pub fn native_needed(&self) -> u128 {
        if self.sending_native {
            self.max_fee_wei().saturating_add(self.amount)
        } else {
            self.max_fee_wei()
        }
    }

    /// `None` while the balance is unknown - never guessed.
    pub fn affordable(&self) -> Option<bool> {
        self.native_balance.map(|b| b >= self.native_needed())
    }

    pub fn shortfall(&self) -> Option<Amount> {
        self.native_balance.map(|b| {
            Amount::new(
                self.native_needed().saturating_sub(b) as i128,
                self.chain.native_decimals,
            )
        })
    }
}

/// What a Solana transfer costs.
///
/// Two of these three numbers have no equivalent on the other chains.
///
/// The base fee is trivial and fixed. The *rent* is not: tokens do not go to an
/// address, they go to an account derived from the address and the mint, and if
/// the recipient has never held this token the sender pays about 0.002 SOL to
/// open one - roughly forty times the fee itself. It is charged once per
/// recipient per token, and it is the cost people are most surprised by.
///
/// The priority fee is what decides whether the transfer arrives at all. Solana
/// drops rather than queues, so a transaction that bids too low during
/// congestion is not slow; it never lands, and the blockhash expires.
pub struct SolanaFee {
    pub compute_units: u32,
    /// Micro-lamports per compute unit, taken from what recent blocks accepted.
    pub compute_unit_price: u64,
    /// Rent for the recipient's token account, or zero when they have one.
    pub rent: u64,
    /// `None` when the balance could not be read. Distinct from zero: one is a
    /// fact about the account, the other is our own failure.
    pub sol_balance: Option<u64>,
    /// Whether the amount and the fee come out of the same balance.
    pub sending_native: bool,
    pub amount: u64,
}

impl SolanaFee {
    /// Signature plus compute, plus rent when this transfer opens an account.
    pub fn fee_lamports(&self) -> u64 {
        neko_solana::tx::fee_lamports(1, self.compute_units, self.compute_unit_price)
            .saturating_add(self.rent)
    }

    pub fn fee(&self) -> Amount {
        Amount::new(self.fee_lamports() as i128, neko_solana::SOL_DECIMALS)
    }

    /// Rent shown on its own, because it is the part nobody expects and the
    /// part that is forty times everything else.
    pub fn rent_amount(&self) -> Amount {
        Amount::new(self.rent as i128, neko_solana::SOL_DECIMALS)
    }

    pub fn sol_needed(&self) -> u64 {
        if self.sending_native {
            self.fee_lamports().saturating_add(self.amount)
        } else {
            self.fee_lamports()
        }
    }

    /// `None` while the balance is unknown - never guessed.
    pub fn affordable(&self) -> Option<bool> {
        self.sol_balance.map(|b| b >= self.sol_needed())
    }

    pub fn shortfall(&self) -> Option<Amount> {
        self.sol_balance.map(|b| {
            Amount::new(
                self.sol_needed().saturating_sub(b) as i128,
                neko_solana::SOL_DECIMALS,
            )
        })
    }
}

/// What a TON transfer costs.
///
/// Two figures rather than one, which no other chain here needs. The fee is
/// what the message costs and is gone. The *attached* coin is something else:
/// a token transfer is a message to our own jetton wallet contract, which
/// messages the recipient's, and each hop has to be paid for by coin travelling
/// with the message. Most of it comes back. Adding the two would say sending
/// USDT costs about five times what it does; leaving the second out would say a
/// wallet with 0.02 GRAM can send USDT, and it cannot.
///
/// This is the surprising cost on this chain, the way rent is Solana's: sending
/// a token needs GRAM, in a quantity that dwarfs the fee, and most of it is a
/// deposit rather than a charge.
pub struct TonFee {
    /// Nanotons the message itself costs.
    pub fee: u128,
    /// Nanotons that travel with a token transfer to pay for its hops, and are
    /// mostly refunded. Zero for a plain GRAM transfer.
    pub attached: u128,
    /// `None` when the balance could not be read. Distinct from zero: one is a
    /// fact about the account, the other is our own failure.
    pub gram_balance: Option<u128>,
    /// Whether the amount and the fee come out of the same balance.
    pub sending_native: bool,
    pub amount: u128,
    /// Whether this transfer also deploys the wallet contract. Its code travels
    /// with the first message a wallet ever sends, and that costs more.
    pub deploy: bool,
}

impl TonFee {
    pub fn fee_amount(&self) -> Amount {
        Amount::new(self.fee as i128, neko_ton::GRAM_DECIMALS)
    }

    /// Shown on its own, because it is the part nobody expects and it is not a
    /// charge.
    pub fn attached_amount(&self) -> Amount {
        Amount::new(self.attached as i128, neko_ton::GRAM_DECIMALS)
    }

    /// What the balance has to cover: the fee, the attached coin, and - when
    /// GRAM is what is being sent - the amount itself.
    pub fn gram_needed(&self) -> u128 {
        let base = self.fee.saturating_add(self.attached);
        if self.sending_native {
            base.saturating_add(self.amount)
        } else {
            base
        }
    }

    /// `None` while the balance is unknown - never guessed.
    pub fn affordable(&self) -> Option<bool> {
        self.gram_balance.map(|b| b >= self.gram_needed())
    }

    pub fn shortfall(&self) -> Option<Amount> {
        self.gram_balance.map(|b| {
            Amount::new(
                self.gram_needed().saturating_sub(b) as i128,
                neko_ton::GRAM_DECIMALS,
            )
        })
    }
}

/// What a Bitcoin transfer costs.
///
/// The odd one out, because on this chain the fee is not a property of the
/// transfer - it is a property of the *coins chosen to pay for it*. Each one
/// adds about 68 virtual bytes, so a wallet holding a hundred small outputs
/// pays far more to move the same amount than one holding a single large one.
/// That is worth showing, because it is otherwise inexplicable.
///
/// Affordability is decided before this exists: if the coins cannot cover the
/// amount and the fee, selection fails and there is no quote to show. So unlike
/// the other three there is no shortfall state here - only a failed quote that
/// says how much was needed.
pub struct BtcFee {
    /// Thousandths of a satoshi per virtual byte.
    pub fee_rate: neko_btc::coins::FeeRate,
    pub vbytes: usize,
    pub fee: u64,
    /// Coins being spent, out of coins held.
    pub inputs: usize,
    pub utxo_count: usize,
    pub balance: u64,
    /// What comes back to us. `None` means the remainder was below dust.
    pub change: Option<u64>,
    /// The remainder was too small to create an output for, so it went to the
    /// fee. Said out loud, because the fee is then higher than the rate
    /// explains.
    pub change_was_dust: bool,
}

impl BtcFee {
    pub fn fee_amount(&self) -> Amount {
        Amount::new(self.fee as i128, neko_btc::BTC_DECIMALS)
    }

    pub fn balance_amount(&self) -> Amount {
        Amount::new(self.balance as i128, neko_btc::BTC_DECIMALS)
    }

    pub fn change_amount(&self) -> Option<Amount> {
        self.change
            .map(|c| Amount::new(c as i128, neko_btc::BTC_DECIMALS))
    }

    /// What the rate alone would have cost, before dust was folded in. The
    /// difference is what the extra line on the screen is explaining.
    pub fn rate_only_fee(&self) -> u64 {
        self.fee_rate.fee_for(self.vbytes)
    }
}

/// The fee, per chain.
pub enum FeeQuote {
    Tron(TronFee),
    Evm(EvmFee),
    Solana(SolanaFee),
    Bitcoin(BtcFee),
    Ton(TonFee),
    Aptos(AptosFee),
    Sui(SuiFee),
}

/// What a Sui transfer costs.
///
/// The chain charges computation plus storage and then *refunds* part of the
/// storage. Consolidating coin objects frees storage, so a transfer that folds
/// thirty objects into one can earn back more than it costs - the net figure
/// here is genuinely zero sometimes, and that is a fact about the chain rather
/// than a reading that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuiFee {
    /// Net cost in MIST, after the rebate.
    pub fee: u64,
    /// The ceiling, which has to be available in the gas coins whatever the
    /// transfer ends up costing.
    pub budget: u64,
    pub sui_balance: Option<u128>,
    pub sending_native: bool,
    pub amount: u128,
    pub coins_spent: usize,
}

impl SuiFee {
    pub fn fee_amount(&self) -> Amount {
        Amount::new(self.fee as i128, neko_sui::SUI_DECIMALS)
    }

    pub fn budget_amount(&self) -> Amount {
        Amount::new(self.budget as i128, neko_sui::SUI_DECIMALS)
    }

    /// What the balance has to cover: the whole budget, plus the amount when
    /// SUI is what is being sent. The budget rather than the fee, because the
    /// chain will not run a transaction whose gas coins cannot cover it.
    pub fn sui_needed(&self) -> u128 {
        if self.sending_native {
            (self.budget as u128).saturating_add(self.amount)
        } else {
            self.budget as u128
        }
    }

    pub fn affordable(&self) -> Option<bool> {
        self.sui_balance.map(|b| b >= self.sui_needed())
    }

    pub fn shortfall(&self) -> Option<Amount> {
        self.sui_balance.map(|b| {
            Amount::new(
                self.sui_needed().saturating_sub(b) as i128,
                neko_sui::SUI_DECIMALS,
            )
        })
    }
}

/// What an Aptos transfer costs.
///
/// Gas units times a price in octas, which is the same shape as an EVM fee.
///
/// A **ceiling**, not an estimate. Aptos will price a transaction exactly, but
/// only when shown the sender's real public key, which this wallet does not
/// have while quoting - see `chain::aptos_quote`. So the figure here is the
/// allowance, which is what the balance is checked against and what is held
/// back from a "send everything"; unused units are not charged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AptosFee {
    pub max_gas_amount: u64,
    pub gas_unit_price: u64,
    /// `None` when the balance could not be read. Distinct from zero.
    pub apt_balance: Option<u128>,
    pub sending_native: bool,
    pub amount: u128,
}

impl AptosFee {
    /// The ceiling, which is both what is shown and what is reserved.
    pub fn max_fee_octas(&self) -> u128 {
        self.max_gas_amount as u128 * self.gas_unit_price as u128
    }

    pub fn fee_amount(&self) -> Amount {
        Amount::new(self.max_fee_octas() as i128, neko_aptos::APT_DECIMALS)
    }

    pub fn max_fee_amount(&self) -> Amount {
        self.fee_amount()
    }

    /// APT pays the fee whatever is being sent, so a wallet holding only USDT
    /// cannot move it.
    pub fn apt_needed(&self) -> u128 {
        if self.sending_native {
            self.max_fee_octas().saturating_add(self.amount)
        } else {
            self.max_fee_octas()
        }
    }

    /// `None` while the balance is unknown - never guessed.
    pub fn affordable(&self) -> Option<bool> {
        self.apt_balance.map(|b| b >= self.apt_needed())
    }

    pub fn shortfall(&self) -> Option<Amount> {
        self.apt_balance.map(|b| {
            Amount::new(
                self.apt_needed().saturating_sub(b) as i128,
                neko_aptos::APT_DECIMALS,
            )
        })
    }
}

impl FeeQuote {
    /// The figure shown as the total. On TRON this can be an upper bound; on
    /// BNB Chain it is exact.
    /// What has to be *held back* from a balance, which is not always what the
    /// transfer costs.
    ///
    /// On EIP-1559 the chain checks the balance against the ceiling - the whole
    /// `gas_limit x max_fee_per_gas` - even though only the base fee plus the
    /// tip is charged and the difference is refunded. Reserving the expected
    /// cost instead makes "send everything" produce an amount the node rejects
    /// outright, which is exactly what it did.
    ///
    /// Everywhere else the two are the same number, and TRON's total is already
    /// an upper bound when its resources could not be read.
    pub fn reserve(&self) -> Amount {
        match self {
            FeeQuote::Evm(e) => e.max_fee(),
            // A token transfer has to have the attached coin available even
            // though most of it returns, so "send everything" must hold it
            // back. This is the same trap EIP-1559's ceiling was.
            FeeQuote::Ton(t) => Amount::new(
                t.fee.saturating_add(t.attached) as i128,
                neko_ton::GRAM_DECIMALS,
            ),
            // The gas ceiling, not the simulated cost - the same trap as
            // EIP-1559's, and for the same reason: the chain checks the
            // balance against `max_gas_amount x price`.
            FeeQuote::Aptos(a) => a.max_fee_amount(),
            // The whole budget, for the same reason.
            FeeQuote::Sui(s) => s.budget_amount(),
            other => other.total(),
        }
    }

    /// What the transfer is expected to cost. What the screen shows.
    pub fn total(&self) -> Amount {
        match self {
            FeeQuote::Tron(t) => t.total_burn(),
            FeeQuote::Evm(e) => e.fee(),
            FeeQuote::Solana(s) => s.fee(),
            FeeQuote::Bitcoin(b) => b.fee_amount(),
            // The fee alone. The attached coin is shown beside it rather than
            // added into it, because it comes back.
            FeeQuote::Ton(t) => t.fee_amount(),
            FeeQuote::Aptos(a) => a.fee_amount(),
            FeeQuote::Sui(s) => s.fee_amount(),
        }
    }

    /// The native balance the chain reported while quoting, when it reported
    /// one. Fresher than whatever the screen was opened with, so it wins.
    ///
    /// TRON's quote is about energy and bandwidth, not the TRX balance, so
    /// there is nothing to offer here rather than something approximate.
    pub fn native_balance(&self) -> Option<i128> {
        match self {
            FeeQuote::Tron(_) => None,
            FeeQuote::Evm(e) => e.native_balance.map(|v| v as i128),
            FeeQuote::Solana(s) => s.sol_balance.map(|v| v as i128),
            FeeQuote::Bitcoin(b) => Some(b.balance as i128),
            FeeQuote::Ton(t) => t.gram_balance.map(|v| v as i128),
            FeeQuote::Aptos(a) => a.apt_balance.map(|v| v as i128),
            FeeQuote::Sui(s) => s.sui_balance.map(|v| v as i128),
        }
    }

    /// Tell the quote the amount changed, so affordability is recomputed
    /// against what is actually being sent.
    pub fn set_amount(&mut self, amount: i128) {
        match self {
            // TRON's burn is a function of the transaction's size and the
            // account's resources, not of the amount, and its quote carries no
            // amount to update.
            FeeQuote::Tron(_) => {}
            FeeQuote::Evm(e) => e.amount = amount.max(0) as u128,
            FeeQuote::Solana(s) => s.amount = amount.max(0) as u64,
            // The coins were chosen for a particular amount; changing it after
            // the fact would invalidate the selection that produced this fee.
            // "Send everything" is a different selection, not an adjustment.
            FeeQuote::Bitcoin(_) => {}
            FeeQuote::Ton(t) => t.amount = amount.max(0) as u128,
            FeeQuote::Aptos(a) => a.amount = amount.max(0) as u128,
            // The coin objects were chosen for a particular amount, as on
            // Bitcoin. Changing it is a new selection, not an adjustment.
            FeeQuote::Sui(_) => {}
        }
    }

    pub fn is_upper_bound(&self) -> bool {
        match self {
            FeeQuote::Tron(t) => t.is_upper_bound(),
            // Gas limit times a price the node quoted. Nothing is assumed -
            // and on Ethereum the *expected* figure can come in under the
            // ceiling, which is a refund rather than an upper bound.
            FeeQuote::Evm(_) => false,
            // Signature fee, compute budget and rent are all exact figures the
            // cluster gave us.
            FeeQuote::Solana(_) => false,
            // Inputs minus outputs, both already decided.
            FeeQuote::Bitcoin(_) => false,
            // A fixed allowance rather than a quote: TON's fees are small and
            // fixed in shape, and this is deliberately generous. It is the one
            // figure here that really is a ceiling.
            FeeQuote::Ton(_) => true,
            // An allowance rather than a quote - the chain will not price
            // this without a key the wallet does not have yet. The one figure
            // shown really is a ceiling, as TON's is.
            FeeQuote::Aptos(_) => true,
            // A dry run of this exact transaction.
            FeeQuote::Sui(_) => false,
        }
    }

    /// Whether the chain's own figures say this transfer can be paid for.
    ///
    /// `None` means the balance could not be read, and must not be turned into
    /// a refusal - an unknown balance is not an empty one. `Some(false)` is the
    /// chain's arithmetic, not a guess, and nothing should be signed against
    /// it: see [`crate::keys`], where it stops the flow.
    pub fn affordable(&self) -> Option<bool> {
        match self {
            // TRON's quote is about energy and bandwidth. The burn comes out of
            // the TRX balance, which this does not carry, so there is nothing
            // to answer rather than something approximate.
            FeeQuote::Tron(_) => None,
            FeeQuote::Evm(e) => e.affordable(),
            FeeQuote::Solana(s) => s.affordable(),
            // Decided before the quote existed: if the coins could not cover
            // the amount and the fee, selection failed and there is no quote.
            FeeQuote::Bitcoin(_) => Some(true),
            FeeQuote::Ton(t) => t.affordable(),
            FeeQuote::Aptos(a) => a.affordable(),
            FeeQuote::Sui(s) => s.affordable(),
        }
    }

    /// How far short, when the chain's figures say it is short at all.
    pub fn shortfall(&self) -> Option<Amount> {
        match self {
            FeeQuote::Tron(_) | FeeQuote::Bitcoin(_) => None,
            FeeQuote::Evm(e) => e.shortfall(),
            FeeQuote::Solana(s) => s.shortfall(),
            FeeQuote::Ton(t) => t.shortfall(),
            FeeQuote::Aptos(a) => a.shortfall(),
            FeeQuote::Sui(s) => s.shortfall(),
        }
    }

    /// Whether the figure is a ceiling *because a lookup failed*.
    ///
    /// Distinct from [`Self::is_upper_bound`], which two chains answer `true`
    /// to for opposite reasons. TRON's total is a ceiling when the account's
    /// energy and bandwidth could not be read and it had to assume none - that
    /// is a failure, worth an alarm and a pointer at the fix. TON's is a
    /// ceiling by design: the fee is a fixed allowance rather than a quote,
    /// nothing failed, and no API key would change it.
    ///
    /// The screen showed TRON's alarm - energy, bandwidth, API key and all - on
    /// TON, a chain that has none of those things.
    pub fn resources_unreadable(&self) -> bool {
        match self {
            FeeQuote::Tron(t) => t.is_upper_bound(),
            FeeQuote::Evm(_)
            | FeeQuote::Solana(_)
            | FeeQuote::Bitcoin(_)
            | FeeQuote::Ton(_)
            // Aptos simulates the real transaction, so nothing here is a
            // ceiling standing in for a reading that failed.
            | FeeQuote::Aptos(_)
            | FeeQuote::Sui(_) => false,
        }
    }

    pub fn is_free(&self) -> bool {
        match self {
            FeeQuote::Tron(t) => t.is_free(),
            // An EVM transaction always costs gas.
            FeeQuote::Evm(_) => false,
            // And a Solana one always costs at least a signature.
            FeeQuote::Solana(_) => false,
            // Bitcoin has no free transactions at all.
            FeeQuote::Bitcoin(_) => false,
            // And a TON message is charged gas and storage by the contract it
            // runs.
            FeeQuote::Ton(_) => false,
            // Aptos charges gas units times a price, always at least one unit.
            FeeQuote::Aptos(_) => false,
            // And a Sui one can genuinely net to zero, when the storage
            // rebate from consolidating coin objects exceeds the cost.
            FeeQuote::Sui(s) => s.fee == 0,
        }
    }
}

pub enum SendStep {
    Recipient,
    EnterAmount,
    /// Waiting on the block reference and the energy estimate.
    Quoting,
    Review {
        req: Box<TransferRequest>,
        params: Box<ChainTxParams>,
        /// Boxed: the quote now carries a whole chain's parameters, and an
        /// unboxed one makes every other step of the flow as large as the
        /// heaviest.
        quote: Option<Box<FeeQuote>>,
        typed: Field,
    },
    /// The last gate: the password, re-derived in full.
    ///
    /// An unlocked terminal that someone walked away from must not be enough to
    /// move funds. Reusing the in-memory key would make this decorative, so the
    /// whole Argon2id derivation runs again.
    Authorize {
        req: Box<TransferRequest>,
        params: Box<ChainTxParams>,
        password: Field,
        checking: bool,
    },
    Broadcasting,
    Done {
        txid: String,
        explorer: String,
    },
    Failed(String),
}

pub struct SendState {
    /// Counterparties seen in this address's history, used to spot a
    /// destination that imitates one of them.
    pub known: Vec<String>,
    pub wallet_id: i64,
    pub wallet_name: String,
    pub from: ChainAddress,
    pub asset: Asset,
    pub asset_label: String,
    pub to: Field,
    pub amount: Field,
    pub step: SendStep,
    pub error: Option<String>,
    /// The asset's balance in minimal units, read when this screen opened.
    /// `None` when it could not be read - and then there is no maximum to
    /// offer, rather than a guessed one.
    pub balance: Option<i128>,
    /// The user asked to send everything.
    ///
    /// Kept as a request rather than acted on immediately, because on a native
    /// transfer the fee comes out of the balance being sent and the fee is not
    /// known until the quote comes back.
    pub max_requested: bool,
    /// What was held back to pay the fee, once that has happened. Shown on the
    /// review screen so a reduced amount never looks like a typo.
    pub held_back: Option<Amount>,
}

impl SendState {
    pub fn new(
        wallet_id: i64,
        wallet_name: String,
        from: ChainAddress,
        asset: Asset,
        asset_label: String,
    ) -> Self {
        Self {
            known: Vec::new(),
            wallet_id,
            wallet_name,
            from,
            asset,
            asset_label,
            to: Field::new(false),
            amount: Field::new(false),
            step: SendStep::Recipient,
            error: None,
            balance: None,
            max_requested: false,
            held_back: None,
        }
    }

    /// Replace the amount with an exact figure.
    ///
    /// Written through the same field the user types into, so everything
    /// downstream - the request, the review screen, what gets signed - reads one
    /// value. `to_display_string_full` keeps every digit and drops only trailing
    /// zeros, so what lands in the field parses back to exactly `raw`.
    pub fn set_amount(&mut self, raw: i128) {
        self.amount.clear();
        for c in Amount::new(raw, self.asset.decimals())
            .to_display_string_full()
            .chars()
        {
            self.amount.push(c);
        }
    }

    /// Everything.
    ///
    /// For a token this is the final answer: the fee is paid in the chain's own
    /// coin, out of a different balance. For the native coin it is only the
    /// starting point - the fee still has to come out of it, and that
    /// subtraction waits for the quote.
    pub fn request_max(&mut self) {
        let Some(bal) = self.balance else {
            return;
        };
        self.max_requested = true;
        self.held_back = None;
        self.set_amount(bal);
    }

    /// Hold back the fee from a "send everything" on the native coin.
    ///
    /// Returns the new amount when it changed. A fee at least as large as the
    /// balance leaves the amount alone: the review screen already explains that
    /// the balance cannot cover the fee, and silently rewriting the amount to
    /// zero would replace that explanation with a puzzle.
    /// `fee` is what the chain will *check the balance against*, not what the
    /// transfer is expected to cost. On EIP-1559 those differ by the whole
    /// refundable headroom, and holding back the smaller one produces an amount
    /// the node refuses.
    pub fn hold_back_fee(&mut self, balance: i128, fee: Amount) -> Option<i128> {
        if !self.max_requested || !self.asset.is_native() {
            return None;
        }
        let max = balance.checked_sub(fee.raw)?;
        if max <= 0 {
            return None;
        }
        self.set_amount(max);
        self.held_back = Some(fee);
        Some(max)
    }

    /// Validate as the user types, so a bad address is obvious before they
    /// commit to an amount.
    pub fn chain(&self) -> ChainId {
        self.asset.chain()
    }

    /// Validate as the user types, against *this chain's* address format. A
    /// TRON address typed into a BNB Chain transfer is invalid here, not
    /// merely unusual.
    pub fn recipient_error(&self) -> Option<&'static str> {
        if self.to.is_empty() {
            return None;
        }
        match ChainAddress::parse(self.chain(), self.to.value().trim()) {
            Ok(a) if a == self.from => Some(neko_i18n::t(neko_i18n::Key::Send_ErrOwnAddress)),
            Ok(_) => None,
            Err(_) => Some(neko_i18n::t(neko_i18n::Key::Send_ErrInvalidAddress)),
        }
    }

    pub fn amount_error(&self) -> Option<String> {
        if self.amount.is_empty() {
            return None;
        }
        Amount::parse(self.amount.value(), self.asset.decimals())
            .err()
            .map(|e| e.to_string())
    }

    pub fn build_request(&self) -> Result<TransferRequest, neko_core::CoreError> {
        TransferRequest::parse(
            self.wallet_id,
            self.from,
            self.to.value(),
            self.amount.value(),
            self.asset,
        )
    }

    /// Does the destination imitate an address from this wallet's history?
    ///
    /// Address poisoning works by getting a lookalike into your history and
    /// waiting for you to copy the wrong one. If the destination is *similar
    /// to but not the same as* something you have transacted with, say so
    /// loudly — that is the moment the attack pays off or fails.
    pub fn lookalike_warning(&self) -> Option<String> {
        // Compare the typed text rather than a parsed address, so the warning
        // appears as soon as a full-length destination is entered — including
        // one whose checksum is wrong, where a mistyped character is exactly
        // what we want to surface.
        let dest = self.to.value().trim();
        // Only once a full-length address has been typed; comparing prefixes
        // while somebody is still typing would warn on every keystroke.
        let n = dest.chars().count();
        let complete = match self.chain() {
            ChainId::Tron => n == TRON_ADDRESS_LEN,
            ChainId::Bsc
            | ChainId::Ethereum
            | ChainId::Polygon
            | ChainId::Base
            | ChainId::Arbitrum
            | ChainId::Optimism
            | ChainId::Avalanche
            | ChainId::HyperEvm
            | ChainId::Mantle
            | ChainId::Linea
            | ChainId::ZkSyncEra
            | ChainId::Scroll => n == EVM_ADDRESS_LEN,
            // A Solana address is 32 bytes in base58, and base58 shortens a
            // value with leading zero bytes - so there is no single length to
            // compare against, only a range.
            ChainId::Solana => (SOLANA_ADDRESS_MIN_LEN..=SOLANA_ADDRESS_MAX_LEN).contains(&n),
            // Five script types across two text encodings: a base58 P2PKH is
            // 26 to 35 characters, a bech32 P2WPKH is 42, and a P2WSH or
            // Taproot address is 62.
            ChainId::Bitcoin => (BTC_ADDRESS_MIN_LEN..=BTC_ADDRESS_MAX_LEN).contains(&n),
            // A TON address is a workchain byte and 32 bytes of hash with a
            // checksum, base64url'd - always the same length, unlike base58.
            ChainId::Ton => n == TON_ADDRESS_LEN,
            // 0x and 64 hex characters. Aptos itself prints shortened
            // addresses with the leading zeros dropped, so anything from a
            // few characters up to the full width is a complete address here.
            ChainId::Aptos => (APTOS_ADDRESS_MIN_LEN..=APTOS_ADDRESS_MAX_LEN).contains(&n),
            // Always the full width. Sui does not print shortened addresses,
            // so accepting one would only widen what a typo can be.
            ChainId::Sui => n == SUI_ADDRESS_LEN,
        };
        if !complete {
            return None;
        }
        self.known
            .iter()
            .find(|k| neko_tron::history::looks_alike(dest, k))
            .cloned()
    }

    /// Enabled only once the retyped tail matches the real destination.
    pub fn confirmation_satisfied(&self) -> bool {
        match &self.step {
            SendStep::Review { req, typed, .. } => {
                typed.value() == confirm_tail(&req.to.to_string())
            }
            _ => false,
        }
    }
}

/// Split an address for display: leading chunk, dimmed middle, trailing chunk.
/// Turns checking a destination from reading a wall of base58 into a visual
/// comparison.
///
/// The tail is exactly `CONFIRM_CHARS` long, so the highlighted characters are
/// precisely the ones the user is asked to retype.
pub fn split_address(a: &str) -> (String, String, String) {
    let chars: Vec<char> = a.chars().collect();
    let head = 6;
    if chars.len() <= head + CONFIRM_CHARS {
        return (a.to_string(), String::new(), String::new());
    }
    (
        chars[..head].iter().collect(),
        chars[head..chars.len() - CONFIRM_CHARS].iter().collect(),
        chars[chars.len() - CONFIRM_CHARS..].iter().collect(),
    )
}

/// The characters the user must retype to confirm.
pub fn confirm_tail(address: &str) -> String {
    let chars: Vec<char> = address.chars().collect();
    chars[chars.len().saturating_sub(CONFIRM_CHARS)..]
        .iter()
        .collect()
}

/// A short name for a step, for diagnostics that must not print any of its
/// contents.
pub fn step_name(step: &SendStep) -> &'static str {
    match step {
        SendStep::Recipient => "recipient",
        SendStep::EnterAmount => "amount",
        SendStep::Quoting => "quoting",
        SendStep::Review { .. } => "review",
        SendStep::Authorize { .. } => "authorize",
        SendStep::Broadcasting => "broadcasting",
        SendStep::Done { .. } => "done",
        SendStep::Failed(_) => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDR: &str = "TNYxHL2s6Wjpx86NRwhekYzc27p3oDYrk6";

    #[test]
    fn address_splits_for_visual_comparison() {
        let (a, b, c) = split_address(ADDR);
        assert_eq!(a, "TNYxHL");
        assert_eq!(c, "oDYrk6");
        assert_eq!(
            a.len() + b.len() + c.len(),
            ADDR.len(),
            "split lost characters"
        );
    }

    /// The highlighted tail must be exactly what the user is asked to type,
    /// or the confirmation is pointing at the wrong characters.
    #[test]
    fn highlighted_tail_is_what_must_be_retyped() {
        let (_, _, tail) = split_address(ADDR);
        assert_eq!(tail, confirm_tail(ADDR));
        assert_eq!(tail.chars().count(), CONFIRM_CHARS);
    }

    #[test]
    fn short_strings_do_not_panic() {
        let (a, b, c) = split_address("T");
        assert_eq!((a.as_str(), b.as_str(), c.as_str()), ("T", "", ""));
        assert_eq!(split_address("").0, "");
        assert_eq!(confirm_tail("ab"), "ab");
        assert_eq!(confirm_tail(""), "");
    }
}
