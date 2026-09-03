//! Argon2id cost profiles.
//!
//! FROZEN: never mutate an entry, only append. The profile id is stored in the
//! plaintext file header, so changing what an id means would make existing
//! vaults permanently unopenable.

use neko_crypto::Argon2idParams;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Profile {
    pub id: u8,
    pub name: &'static str,
    pub params: Argon2idParams,
}

const fn p(id: u8, name: &'static str, mem_kib: u32, iters: u32) -> Profile {
    Profile {
        id,
        name,
        params: Argon2idParams {
            mem_kib,
            iters,
            par: 4,
            key_len: 32,
        },
    }
}

/// 128 MiB — for low-spec machines.
pub const LIGHT: Profile = p(1, "light", 131_072, 4);
/// 256 MiB — the default. ~0.5-1s with rayon.
pub const BALANCED: Profile = p(2, "balanced", 262_144, 3);
/// 1 GiB — matches the Go reference's vault profile exactly.
pub const PARANOID: Profile = p(3, "paranoid", 1_048_576, 4);
/// Test-only: sits at *exactly* the production floor (64 MiB / t=2 / p=1), so
/// the suite runs fast while still exercising the same validation paths as a
/// real vault. Going below the floor is not an option -- the schema CHECK
/// constraints and `Argon2idParams::validate` both reject it, which is the
/// point.
#[cfg(any(test, feature = "dangerous-fast-kdf"))]
pub const TESTONLY: Profile = Profile {
    id: 250,
    name: "test-only",
    params: Argon2idParams {
        mem_kib: 65_536,
        iters: 2,
        par: 1,
        key_len: 32,
    },
};

pub const DEFAULT: Profile = BALANCED;

/// `p` is deliberately fixed at 4 in every profile: parallelism changes the
/// Argon2 output, so it can never depend on the machine. (The rayon thread
/// count may vary freely; the result is identical.)
pub fn by_id(id: u8) -> Option<Profile> {
    match id {
        1 => Some(LIGHT),
        2 => Some(BALANCED),
        3 => Some(PARANOID),
        #[cfg(any(test, feature = "dangerous-fast-kdf"))]
        250 => Some(TESTONLY),
        _ => None,
    }
}

pub const ALL: &[Profile] = &[LIGHT, BALANCED, PARANOID];
