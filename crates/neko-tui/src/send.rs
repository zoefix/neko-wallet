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

/// The fee, broken down.
///
/// A bare "~23.9 TRX" is not actionable. TRON burns TRX only for the part of a
/// transfer the account cannot already cover, so the same transfer is free for
/// an account with staked energy and costly for one without. Showing what is
/// needed alongside what is held is what makes the number mean anything.
pub struct TronFee {
    /// Base cost of the contract call.
    pub energy_base: i64,
    /// TRON's dynamic-energy surcharge on heavily used contracts. For USDT this
    /// is a large fraction of the total, and it is why the figure looks nothing
    /// like the numbers quoted in older documentation.
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
    /// A first-time recipient pays to create a storage slot, which measured
    /// (mainnet, 2026-09) as 230,920 energy against 113,920 for an address that
    /// already holds the token — roughly double, not the 15x quoted in older
    /// documentation.
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

/// What a BNB Chain transfer costs.
///
/// Nothing here resembles TRON's model and that is the point. There is no
/// allowance to draw down: gas is always paid, always in BNB, at a price the
/// node quotes. The consequence worth surfacing is that a wallet holding only
/// USDT cannot send that USDT - it has no BNB for the fee - which is a state
/// people reach constantly and which an "insufficient funds" from the node
/// explains badly.
pub struct BscFee {
    pub gas_limit: u64,
    pub gas_price: u128,
    /// `None` when the balance could not be read. Distinct from zero: one is a
    /// fact about the account, the other is our own failure.
    pub bnb_balance: Option<u128>,
    /// Whether the amount and the fee come out of the same balance.
    pub sending_native: bool,
    pub amount: u128,
}

impl BscFee {
    /// Gas is paid in full, so this is the fee - not a ceiling.
    pub fn fee_wei(&self) -> u128 {
        self.gas_limit as u128 * self.gas_price
    }

    pub fn fee(&self) -> Amount {
        Amount::new(self.fee_wei() as i128, neko_evm::BNB_DECIMALS)
    }

    /// What this transfer takes out of the BNB balance: the fee, plus the
    /// amount itself when the amount *is* BNB.
    pub fn bnb_needed(&self) -> u128 {
        if self.sending_native {
            self.fee_wei().saturating_add(self.amount)
        } else {
            self.fee_wei()
        }
    }

    /// `None` while the balance is unknown - never guessed.
    pub fn affordable(&self) -> Option<bool> {
        self.bnb_balance.map(|b| b >= self.bnb_needed())
    }

    pub fn shortfall(&self) -> Option<Amount> {
        self.bnb_balance.map(|b| {
            Amount::new(
                self.bnb_needed().saturating_sub(b) as i128,
                neko_evm::BNB_DECIMALS,
            )
        })
    }
}

/// The fee, per chain.
pub enum FeeQuote {
    Tron(TronFee),
    Bsc(BscFee),
}

impl FeeQuote {
    /// The figure shown as the total. On TRON this can be an upper bound; on
    /// BNB Chain it is exact.
    pub fn total(&self) -> Amount {
        match self {
            FeeQuote::Tron(t) => t.total_burn(),
            FeeQuote::Bsc(b) => b.fee(),
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
            FeeQuote::Bsc(b) => b.bnb_balance.map(|v| v as i128),
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
            FeeQuote::Bsc(b) => b.amount = amount.max(0) as u128,
        }
    }

    pub fn is_upper_bound(&self) -> bool {
        match self {
            FeeQuote::Tron(t) => t.is_upper_bound(),
            // Gas limit times gas price, both known. Nothing is being assumed.
            FeeQuote::Bsc(_) => false,
        }
    }

    pub fn is_free(&self) -> bool {
        match self {
            FeeQuote::Tron(t) => t.is_free(),
            // A BNB Chain transaction always costs gas.
            FeeQuote::Bsc(_) => false,
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
        quote: Option<FeeQuote>,
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
        let want = match self.chain() {
            ChainId::Tron => TRON_ADDRESS_LEN,
            ChainId::Bsc => EVM_ADDRESS_LEN,
        };
        if dest.chars().count() != want {
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
