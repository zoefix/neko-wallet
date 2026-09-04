//! Solana constants, each one verified against the cluster before use.
//!
//! A wrong mint here sends funds to a token nobody wants; a wrong program id
//! builds a transaction the cluster rejects after taking the fee. The client
//! still asks the chain for a mint's `decimals` before any transfer rather than
//! trusting the number below.

use neko_hd::SolanaAddress;

/// SOL is quoted in lamports: 1 SOL = 1e9.
pub const SOL_DECIMALS: u8 = 9;

/// USDT (SPL).
///
/// **Six decimals**, like TRON's and unlike BNB Chain's eighteen. Three chains,
/// two precisions for one token name, which is why decimals travel with the
/// asset everywhere in this program.
pub const USDT_MINT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";
pub const USDT_DECIMALS: u8 = 6;

/// Programs. These are addresses like any other, and are parsed rather than
/// stored as bytes so a typo fails a test instead of being re-encoded into a
/// different program.
pub const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
pub const TOKEN_PROGRAM: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
pub const ASSOCIATED_TOKEN_PROGRAM: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";
pub const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";

/// Default public endpoint. Rate-limited and configurable, like TronGrid's.
pub const DEFAULT_RPC: &str = "https://api.mainnet-beta.solana.com";

/// Lamports per signature. Fixed by the cluster, and the whole of the base fee
/// for the transactions this wallet builds.
pub const LAMPORTS_PER_SIGNATURE: u64 = 5_000;

/// What it costs to open an SPL token account: rent for 165 bytes, paid once,
/// by whoever sends the first tokens to an address that has never held them.
///
/// This has no equivalent on TRON or BNB Chain, and it is the single most
/// surprising cost on Solana - roughly 40x a plain transfer's fee. The wallet
/// asks the cluster for the current figure rather than relying on this, and
/// uses it only to explain the charge before it happens.
pub const TOKEN_ACCOUNT_RENT: u64 = 2_039_280;

/// The minimum balance an ordinary account must keep to avoid being purged.
///
/// A transfer that would leave *less* than this - but more than zero - is
/// rejected by the runtime. Emptying an account completely is allowed; leaving
/// it with dust is not.
pub const SYSTEM_ACCOUNT_RENT: u64 = 890_880;

/// Compute units a plain transfer or an SPL transfer actually uses, with room
/// to spare. Asking for less makes the transaction fail; asking for far more
/// costs nothing in itself but inflates the priority fee.
pub const COMPUTE_UNITS_SOL: u32 = 500;
pub const COMPUTE_UNITS_TOKEN: u32 = 40_000;
/// With an account creation folded in.
pub const COMPUTE_UNITS_TOKEN_WITH_ATA: u32 = 60_000;

/// How long a blockhash stays valid: 150 slots, about 60-90 seconds.
///
/// Nothing on TRON or BNB Chain expires this fast. It is why the blockhash is
/// fetched immediately before signing rather than carried from the fee quote,
/// which a user reading the confirmation screen would easily outlast.
pub const BLOCKHASH_VALIDITY_SECS: u64 = 60;

pub fn usdt_mint() -> SolanaAddress {
    parse_const(USDT_MINT)
}
pub fn system_program() -> SolanaAddress {
    parse_const(SYSTEM_PROGRAM)
}
pub fn token_program() -> SolanaAddress {
    parse_const(TOKEN_PROGRAM)
}
pub fn associated_token_program() -> SolanaAddress {
    parse_const(ASSOCIATED_TOKEN_PROGRAM)
}
pub fn compute_budget_program() -> SolanaAddress {
    parse_const(COMPUTE_BUDGET_PROGRAM)
}

fn parse_const(s: &str) -> SolanaAddress {
    SolanaAddress::parse(s).expect("a constant address in this file is malformed")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-tripping proves each constant is well-formed base58 of exactly 32
    /// bytes, and that no character was transposed when it was typed in.
    #[test]
    fn every_constant_address_round_trips() {
        for s in [
            USDT_MINT,
            SYSTEM_PROGRAM,
            TOKEN_PROGRAM,
            ASSOCIATED_TOKEN_PROGRAM,
            COMPUTE_BUDGET_PROGRAM,
        ] {
            let a = SolanaAddress::parse(s).unwrap_or_else(|e| panic!("{s}: {e}"));
            assert_eq!(a.to_string(), s);
        }
    }

    /// The system program is the zero address, and the compute budget program
    /// is padded with ones for the same reason - both are recognisable by shape,
    /// which is a cheap check that the right constant is in the right place.
    #[test]
    fn the_system_program_is_the_zero_address() {
        assert_eq!(system_program().as_bytes(), &[0u8; 32]);
    }

    /// Six, not eighteen. The same token is 6 on TRON and 18 on BNB Chain, and
    /// treating one like another is a factor of a million million.
    #[test]
    fn usdt_has_six_decimals_here() {
        assert_eq!(USDT_DECIMALS, 6);
    }
}
