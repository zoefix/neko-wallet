//! TON key derivation.
//!
//! TON's own wallets do not use BIP39 at all. Tonkeeper and the others take 24
//! words through a TON-specific KDF - PBKDF2 with the literal salt
//! `TON default seed` - and the result *is* the key, with no path. A phrase
//! from this wallet will not open one of those, and one of theirs will not open
//! this.
//!
//! What is used here instead is SLIP-0010 at `m/44'/607'/0'`, which is what
//! Ledger and Trust Wallet do. It keeps this wallet's promise that one phrase
//! covers every chain, and it means the address differs from Tonkeeper's for
//! the same words. There is no path that satisfies both.

use zeroize::Zeroizing;

use crate::error::HdError;
use crate::slip10::derive_path;

/// SLIP-44 coin type for TON.
pub const COIN_TYPE_TON: u32 = 607;

/// `m/44'/607'/{account}'`. Three levels, not five: this is the path Ledger
/// and Trust Wallet use, and adding change and index levels below it would be
/// a different key.
pub fn path_for(account: u32) -> [u32; 3] {
    [44, COIN_TYPE_TON, account]
}

pub fn path_string(account: u32) -> String {
    format!("m/44'/{COIN_TYPE_TON}'/{account}'")
}

pub fn private_key_at(seed: &[u8; 64], account: u32) -> Result<Zeroizing<[u8; 32]>, HdError> {
    if account >= crate::derive::MAX_INDEX {
        return Err(HdError::IndexOutOfRange(account));
    }
    Ok(derive_path(seed, &path_for(account)))
}

/// The Ed25519 public key, which goes into a wallet contract's storage. On this
/// chain that is as close to "an address" as a key gets: the address is what
/// the contract holding this key hashes to.
pub fn public_key(sk: &[u8; 32]) -> [u8; 32] {
    ed25519_dalek::SigningKey::from_bytes(sk)
        .verifying_key()
        .to_bytes()
}

pub fn sign(sk: &[u8; 32], message: &[u8]) -> [u8; 64] {
    use ed25519_dalek::Signer;
    ed25519_dalek::SigningKey::from_bytes(sk)
        .sign(message)
        .to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The path is three levels and all of them hardened. A fourth level, or a
    /// non-hardened one, is a different key and so a different wallet.
    #[test]
    fn the_path_is_the_ledger_one() {
        assert_eq!(path_for(0), [44, 607, 0]);
        assert_eq!(path_string(0), "m/44'/607'/0'");
        assert_eq!(path_for(3), [44, 607, 3]);
        assert_eq!(COIN_TYPE_TON, 607);
    }

    /// Different accounts are different keys, and a signature verifies against
    /// the key it came from.
    #[test]
    fn keys_are_distinct_and_sign() {
        let seed = [21u8; 64];
        let a = public_key(&private_key_at(&seed, 0).unwrap());
        let b = public_key(&private_key_at(&seed, 1).unwrap());
        assert_ne!(a, b);

        let sk = private_key_at(&seed, 0).unwrap();
        let sig = sign(&sk, b"neko-wallet");
        use ed25519_dalek::Verifier;
        ed25519_dalek::VerifyingKey::from_bytes(&a)
            .unwrap()
            .verify(b"neko-wallet", &ed25519_dalek::Signature::from_bytes(&sig))
            .expect("the signature does not match the key");
    }
}
