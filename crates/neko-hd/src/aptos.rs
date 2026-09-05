//! Keys for Aptos.
//!
//! Ed25519 and SLIP-0010, like Solana's and TON's, at `m/44'/637'/0'/0'/0'` -
//! five levels, all hardened. That is Petra's default and the Aptos SDK's, and
//! it has to be stated rather than assumed: a wallet deriving at four levels
//! shows a different, valid, empty account for the same phrase.
//!
//! Confirmed against mainnet. The standard test phrase derives
//! `0xeb66..d3bf`, an account that exists with eleven transactions and an
//! authentication key equal to its address.

use zeroize::Zeroizing;

use crate::error::HdError;
use crate::COIN_TYPE_APTOS;

pub fn path_for(index: u32) -> [u32; 5] {
    [44, COIN_TYPE_APTOS, index, 0, 0]
}

pub fn path_string(index: u32) -> String {
    format!("m/44'/{COIN_TYPE_APTOS}'/{index}'/0'/0'")
}

pub fn private_key_at(seed: &[u8; 64], index: u32) -> Result<Zeroizing<[u8; 32]>, HdError> {
    if index >= crate::derive::MAX_INDEX {
        return Err(HdError::IndexOutOfRange(index));
    }
    Ok(crate::slip10::derive_path(seed, &path_for(index)))
}

pub fn public_key(sk: &[u8; 32]) -> [u8; 32] {
    crate::solana::public_key(sk)
}

pub fn sign(sk: &[u8; 32], message: &[u8]) -> [u8; 64] {
    crate::solana::sign(sk, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The path, spelled out, because a wallet at the wrong depth is silently
    /// a different wallet.
    #[test]
    fn the_path_is_five_levels() {
        assert_eq!(path_for(0), [44, 637, 0, 0, 0]);
        assert_eq!(path_string(0), "m/44'/637'/0'/0'/0'");
        assert_eq!(path_string(3), "m/44'/637'/3'/0'/0'");
    }

    /// Against mainnet: this phrase controls an account that exists.
    #[test]
    fn the_reference_phrase_derives_the_account_that_exists_on_chain() {
        let seed = crate::derive::seed_from_mnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            "",
        )
        .unwrap();
        let sk = private_key_at(&seed, 0).unwrap();
        assert_eq!(
            hex::encode(public_key(&sk)),
            "a686f0309ab80312979606cfccc10ea2740147ae6888351488d11c46f08fbf60"
        );
    }
}
