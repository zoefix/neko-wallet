//! Pick a KDF profile that actually costs what we intend on *this* machine.
//!
//! Measured on an Apple M-series laptop: BALANCED (256 MiB / t=3) completes in
//! ~78 ms, not the ~0.5-1 s the design assumed. A 2015 Intel laptop is roughly
//! 5-10x slower for the same parameters. A fixed default is therefore wrong in
//! both directions: far too weak on fast hardware, uncomfortably slow on old
//! hardware.
//!
//! So we measure and choose the strongest profile that fits a wall-clock budget
//! and a memory cap. The chosen id is recorded in the plaintext file header, so
//! unlock knows the cost before spending a cycle, and the vault stays portable:
//! a database calibrated on a fast machine still opens on a slow one — it just
//! takes longer there.

use std::time::{Duration, Instant};

use crate::error::VaultError;
use crate::profile::{self, Profile};
use neko_crypto::{kdf, Argon2idParams};

/// How long unlocking should take. Long enough to hurt an offline attacker,
/// short enough that nobody disables it.
pub const DEFAULT_BUDGET: Duration = Duration::from_millis(900);

/// A calibration probe: cheap, but the same code path as the real thing.
const PROBE: Argon2idParams = Argon2idParams {
    mem_kib: 65_536,
    iters: 1,
    par: 4,
    key_len: 32,
};

/// Seconds of work per (MiB x pass) on this machine.
fn measure_unit_cost() -> Result<f64, VaultError> {
    let salt = [0u8; 32];
    // Warm up so the first allocation's page faults do not skew the sample.
    let _ = kdf::derive_key(b"calibration", &salt, PROBE)?;

    let mut best = f64::MAX;
    for _ in 0..3 {
        let t0 = Instant::now();
        kdf::derive_key(b"calibration", &salt, PROBE)?;
        best = best.min(t0.elapsed().as_secs_f64());
    }
    let units = (PROBE.mem_kib as f64 / 1024.0) * PROBE.iters as f64;
    Ok(best / units)
}

fn estimate(unit_cost: f64, p: &Profile) -> Duration {
    let units = (p.params.mem_kib as f64 / 1024.0) * p.params.iters as f64;
    Duration::from_secs_f64(unit_cost * units)
}

#[derive(Debug, Clone, Copy)]
pub struct Calibration {
    pub profile: Profile,
    pub estimated: Duration,
}

/// Choose the strongest profile whose estimated cost fits `budget` and whose
/// memory stays under `mem_cap_kib`.
///
/// Falls back to the weakest profile rather than failing: a slow machine should
/// still be able to open a wallet.
pub fn choose(budget: Duration, mem_cap_kib: Option<u32>) -> Result<Calibration, VaultError> {
    Ok(select(measure_unit_cost()?, budget, mem_cap_kib))
}

/// The selection itself, given a measured cost. Pure, and therefore the part
/// that can be tested without asking how fast the machine running the tests
/// happens to be.
///
/// Keeping this separate is not tidiness. The obvious test - "a generous budget
/// picks the strongest profile" - is only true on hardware fast enough for that
/// to be so, and on a slow machine the *correct* answer is a weaker profile.
/// Asserting it against a live measurement tests the CPU, not the code.
pub fn select(unit_cost: f64, budget: Duration, mem_cap_kib: Option<u32>) -> Calibration {
    let unit = unit_cost;
    let cap = mem_cap_kib.unwrap_or(u32::MAX);

    let mut best: Option<Calibration> = None;
    for p in profile::ALL {
        if p.params.mem_kib > cap {
            continue;
        }
        let est = estimate(unit, p);
        if est <= budget {
            let better = best
                .map(|b| p.params.mem_kib > b.profile.params.mem_kib)
                .unwrap_or(true);
            if better {
                best = Some(Calibration {
                    profile: *p,
                    estimated: est,
                });
            }
        }
    }
    best.unwrap_or(Calibration {
        profile: profile::LIGHT,
        estimated: estimate(unit, &profile::LIGHT),
    })
}

/// Calibrate with the default budget and a cap of 25% of the machine's RAM.
pub fn recommend(total_ram_kib: Option<u64>) -> Result<Calibration, VaultError> {
    let cap = total_ram_kib.map(|r| (r / 4).min(u32::MAX as u64) as u32);
    choose(DEFAULT_BUDGET, cap)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cost per (MiB x pass) on a machine fast enough for any profile.
    const FAST: f64 = 0.00002;
    /// ...and on one where even LIGHT blows a normal budget.
    const GLACIAL: f64 = 1.0;

    #[test]
    fn a_generous_budget_selects_the_strongest_profile() {
        let c = select(FAST, Duration::from_secs(60), None);
        assert_eq!(c.profile.id, profile::PARANOID.id);
    }

    #[test]
    fn an_impossible_budget_still_yields_a_usable_profile() {
        let c = select(FAST, Duration::from_nanos(1), None);
        assert_eq!(c.profile.id, profile::LIGHT.id, "must degrade, never fail");
    }

    /// The case the CI runners actually hit: hardware slow enough that the
    /// strongest profile does not fit. Picking a weaker one is correct, and
    /// must not be mistaken for a bug.
    #[test]
    fn slow_hardware_gets_a_weaker_profile_not_a_failure() {
        let c = select(GLACIAL, DEFAULT_BUDGET, None);
        assert_eq!(c.profile.id, profile::LIGHT.id);
        assert!(profile::by_id(c.profile.id).is_some());
    }

    /// Between the extremes the choice must actually track the budget, or the
    /// whole exercise is decoration.
    #[test]
    fn a_bigger_budget_never_picks_a_weaker_profile() {
        let mut last = 0u32;
        for ms in [1u64, 50, 100, 200, 400, 900, 2_000, 10_000, 60_000] {
            let mem = select(0.0002, Duration::from_millis(ms), None)
                .profile
                .params
                .mem_kib;
            assert!(
                mem >= last,
                "raising the budget to {ms}ms weakened the choice"
            );
            last = mem;
        }
    }

    /// A low-memory machine must never be handed a profile it cannot allocate.
    #[test]
    fn memory_cap_is_respected() {
        let c = select(FAST, Duration::from_secs(60), Some(200_000));
        assert!(
            c.profile.params.mem_kib <= 200_000,
            "picked {}",
            c.profile.name
        );
    }

    /// Whatever we pick must be a real, frozen profile id -- the header stores
    /// only the id, so an ad-hoc parameter set would be unopenable later.
    #[test]
    fn chosen_profile_is_always_from_the_frozen_table() {
        for unit in [FAST, 0.0002, 0.02, GLACIAL] {
            for ms in [1u64, 900, 60_000] {
                let c = select(unit, Duration::from_millis(ms), None);
                assert!(profile::by_id(c.profile.id).is_some());
                assert!(c.profile.params.validate().is_ok());
            }
        }
    }

    /// The measurement path still has to work end to end. It asserts only what
    /// is true on every machine: something valid comes back.
    #[test]
    fn measuring_this_machine_yields_a_valid_profile() {
        let c = choose(DEFAULT_BUDGET, None).unwrap();
        assert!(profile::by_id(c.profile.id).is_some());
        assert!(c.profile.params.validate().is_ok());
    }
}
