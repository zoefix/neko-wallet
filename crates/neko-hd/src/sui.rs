//! Keys for Sui.
//!
//! Ed25519 and SLIP-0010 at `m/44'/784'/0'/0'/0'` - the same five-level shape
//! Aptos uses, at a different coin type. That is the Sui Wallet's default and
//! the SDK's.

use zeroize::Zeroizing;

use crate::error::HdError;
use crate::COIN_TYPE_SUI;

pub fn path_for(index: u32) -> [u32; 5] {
    [44, COIN_TYPE_SUI, index, 0, 0]
}

pub fn path_string(index: u32) -> String {
    format!("m/44'/{COIN_TYPE_SUI}'/{index}'/0'/0'")
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

    #[test]
    fn the_path_is_five_levels_at_coin_type_784() {
        assert_eq!(path_for(0), [44, 784, 0, 0, 0]);
        assert_eq!(path_string(0), "m/44'/784'/0'/0'/0'");
        // And not Aptos's, which is the neighbouring mistake.
        assert_ne!(path_for(0), crate::aptos::path_for(0));
    }

    /// The reference phrase, whose public key was checked against a real
    /// signer's address on mainnet - see `neko_sui::address`.
    #[test]
    fn the_reference_phrase_derives_the_expected_key() {
        let seed = crate::derive::seed_from_mnemonic(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            "",
        )
        .unwrap();
        let sk = private_key_at(&seed, 0).unwrap();
        assert_eq!(
            hex::encode(public_key(&sk)),
            "900b4d81eecea3df2f74b14200c4f4cf3f49afaca7a634ffd2cf6ff82bdaecf2"
        );
    }
}
