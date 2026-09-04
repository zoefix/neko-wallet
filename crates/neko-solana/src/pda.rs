//! Program-derived addresses, and the token account that hangs off one.
//!
//! On TRON and BNB Chain a token balance lives in a contract's storage, keyed
//! by the holder's address. On Solana it lives in a *separate account*, whose
//! address is derived from the owner and the mint. Two consequences that
//! nothing on the other chains prepares anyone for:
//!
//! * An address that has never held a token has no account for it, and the
//!   first sender has to create one and pay its rent.
//! * The address funds are sent to is not the recipient's address - it is this
//!   derived one. Getting the derivation wrong sends tokens somewhere real and
//!   unreachable.

use neko_hd::SolanaAddress;
use sha2::{Digest, Sha256};

use crate::chain_consts;
use crate::error::SolanaError;

/// The marker Solana appends so a derived address cannot collide with a hash
/// computed for any other purpose.
const PDA_MARKER: &[u8] = b"ProgramDerivedAddress";

/// Whether 32 bytes are a point on the Ed25519 curve.
///
/// This is the whole trick behind program-derived addresses: a PDA is chosen so
/// that it is *not* a curve point, which means no private key can exist for it.
fn is_on_curve(bytes: &[u8; 32]) -> bool {
    ed25519_dalek::VerifyingKey::from_bytes(bytes).is_ok()
}

/// The first address, walking the bump seed down from 255, that has no private
/// key.
///
/// Solana's own implementation searches in this direction, and the bump it
/// finds is part of the address - searching upward would produce a valid but
/// different account.
pub fn find_program_address(
    seeds: &[&[u8]],
    program_id: &SolanaAddress,
) -> Result<(SolanaAddress, u8), SolanaError> {
    for bump in (0..=u8::MAX).rev() {
        let mut h = Sha256::new();
        for s in seeds {
            h.update(s);
        }
        h.update([bump]);
        h.update(program_id.as_bytes());
        h.update(PDA_MARKER);
        let out: [u8; 32] = h.finalize().into();
        if !is_on_curve(&out) {
            return Ok((SolanaAddress::from_bytes(&out)?, bump));
        }
    }
    Err(SolanaError::NoProgramAddress)
}

/// Where `owner`'s balance of `mint` lives.
pub fn associated_token_address(
    owner: &SolanaAddress,
    mint: &SolanaAddress,
) -> Result<SolanaAddress, SolanaError> {
    let token = chain_consts::token_program();
    find_program_address(
        &[owner.as_bytes(), token.as_bytes(), mint.as_bytes()],
        &chain_consts::associated_token_program(),
    )
    .map(|(a, _)| a)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real mainnet associated token accounts.
    ///
    /// Derivation that is subtly wrong yields a valid address that no wallet
    /// will ever look at, so this is checked against accounts that exist rather
    /// than against the algorithm restated.
    #[test]
    fn known_associated_token_accounts() {
        for (owner, mint, want) in KNOWN_ATAS {
            let o = SolanaAddress::parse(owner).unwrap();
            let m = SolanaAddress::parse(mint).unwrap();
            assert_eq!(
                associated_token_address(&o, &m).unwrap().to_string(),
                *want,
                "ATA for {owner} / {mint}"
            );
        }
    }

    /// A derived address must have no private key, or somebody else could sign
    /// for it.
    #[test]
    fn a_derived_address_is_off_the_curve() {
        let o = SolanaAddress::parse(KNOWN_ATAS[0].0).unwrap();
        let m = SolanaAddress::parse(KNOWN_ATAS[0].1).unwrap();
        let (ata, _) = find_program_address(
            &[
                o.as_bytes(),
                chain_consts::token_program().as_bytes(),
                m.as_bytes(),
            ],
            &chain_consts::associated_token_program(),
        )
        .unwrap();
        assert!(!is_on_curve(ata.as_bytes()));
        // ...whereas an ordinary wallet address is on it.
        assert!(is_on_curve(o.as_bytes()));
    }

    include!("../vectors/ata.rs");
}
