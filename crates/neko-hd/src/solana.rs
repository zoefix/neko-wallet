//! Solana addresses and key derivation.
//!
//! Three things here are unlike TRON and the EVM chains, and each one is a way
//! to lose money if it is assumed rather than checked.
//!
//! **The curve is different.** Solana signs with Ed25519, not secp256k1. The
//! BIP32 machinery the other chains use cannot produce these keys at all, so
//! derivation is SLIP-0010 - a separate algorithm that happens to look similar.
//!
//! **Derivation is hardened at every level.** Ed25519 has no public-key
//! addition, so SLIP-0010 defines no non-hardened child. The `m/44'/x'/y'/0/i`
//! shape the other chains use is not merely discouraged here, it is undefined.
//!
//! **An address carries no checksum.** A TRON address is base58*check* and an
//! EVM address has EIP-55 capitalisation, so a mistyped character is usually
//! caught. A Solana address is the raw 32-byte public key in plain base58:
//! change one character and you very often get another perfectly valid address,
//! belonging to nobody. Length and decodability are all this module can check,
//! which is why the send screen makes the destination be retyped.

use zeroize::Zeroizing;

use crate::error::HdError;
use crate::slip10::derive_path;
#[cfg(test)]
use crate::slip10::HARDENED;

/// SLIP-44 coin type for Solana.
pub const COIN_TYPE_SOLANA: u32 = 501;

/// A base58 Solana address is 32 bytes, which encodes to 32-44 characters.
/// Leading zero bytes shorten the encoding, so the range is real rather than
/// defensive.
pub const ADDRESS_BYTES: usize = 32;

/// An Ed25519 public key, which on Solana *is* the address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SolanaAddress([u8; ADDRESS_BYTES]);

impl SolanaAddress {
    pub fn from_bytes(b: &[u8]) -> Result<Self, HdError> {
        let arr: [u8; ADDRESS_BYTES] = b.try_into().map_err(|_| HdError::BadSolanaAddress)?;
        Ok(Self(arr))
    }

    /// The public key is the address; there is no hashing step to get it wrong.
    pub fn from_public_key(pk: &[u8]) -> Result<Self, HdError> {
        Self::from_bytes(pk)
    }

    pub fn parse(s: &str) -> Result<Self, HdError> {
        let s = s.trim();
        // Rejected before decoding so a pasted TRON or EVM address fails on
        // shape rather than on an obscure alphabet error.
        if s.is_empty() || s.len() > 44 {
            return Err(HdError::BadSolanaAddress);
        }
        let raw = bs58::decode(s)
            .into_vec()
            .map_err(|_| HdError::BadSolanaAddress)?;
        Self::from_bytes(&raw)
    }

    pub fn as_bytes(&self) -> &[u8; ADDRESS_BYTES] {
        &self.0
    }
}

impl std::fmt::Display for SolanaAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&bs58::encode(self.0).into_string())
    }
}

/// Phantom's default path for account `index`: `m/44'/501'/{index}'/0'`.
///
/// Chosen because Phantom (and Backpack, which follows it) is what most people
/// have. Solflare, Ledger Live and Trust Wallet default to `m/44'/501'/{i}'`
/// instead, so the same phrase shows a *different* address there. That is not a
/// bug in either wallet; there is no agreed path for Solana, and this one has
/// to be stated rather than assumed.
pub fn path_for(index: u32) -> [u32; 4] {
    [44, COIN_TYPE_SOLANA, index, 0]
}

/// Human-readable form of the same path, for the UI and for documentation.
pub fn path_string(index: u32) -> String {
    format!("m/44'/{COIN_TYPE_SOLANA}'/{index}'/0'")
}

/// The signing key at `m/44'/501'/{index}'/0'`.
///
/// For Ed25519 the "private key" is a 32-byte seed from which the keypair is
/// expanded, so this has the same shape as the secp256k1 keys elsewhere in this
/// crate but is not interchangeable with them.
pub fn private_key_at(seed: &[u8; 64], index: u32) -> Result<Zeroizing<[u8; 32]>, HdError> {
    if index >= crate::derive::MAX_INDEX {
        return Err(HdError::IndexOutOfRange(index));
    }
    Ok(derive_path(seed, &path_for(index)))
}

/// The public key for a signing key.
pub fn public_key(sk: &[u8; 32]) -> [u8; 32] {
    ed25519_dalek::SigningKey::from_bytes(sk)
        .verifying_key()
        .to_bytes()
}

pub fn address_from_private_key(sk: &[u8; 32]) -> Result<SolanaAddress, HdError> {
    SolanaAddress::from_public_key(&public_key(sk))
}

pub fn address_at(seed: &[u8; 64], index: u32) -> Result<SolanaAddress, HdError> {
    let sk = private_key_at(seed, index)?;
    address_from_private_key(&sk)
}

/// Sign a message. Ed25519 signatures are deterministic, so this needs no
/// randomness and cannot leak a key through a bad nonce - the failure mode that
/// makes ECDSA signing delicate on the other chains.
pub fn sign(sk: &[u8; 32], message: &[u8]) -> [u8; 64] {
    use ed25519_dalek::Signer;
    ed25519_dalek::SigningKey::from_bytes(sk)
        .sign(message)
        .to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex32(s: &str) -> [u8; 32] {
        let mut out = [0u8; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }

    /// The official SLIP-0010 Ed25519 vectors.
    ///
    /// This is the load-bearing test in this file. Derivation that is subtly
    /// wrong does not fail - it produces a different valid wallet, and the
    /// money goes to an address nobody can reach. A hash chain that matches
    /// three published values cannot match by accident, so this pins both the
    /// HMAC construction and the zero byte in front of the key material.
    #[test]
    fn matches_the_slip_0010_vectors() {
        // Test vector 1, seed 000102030405060708090a0b0c0d0e0f.
        let seed: Vec<u8> = (0u8..16).collect();

        for (path, want_key, want_pub) in [
            (
                &[][..],
                "2b4be7f19ee27bbf30c667b642d5f4aa69fd169872f8fc3059c08ebae2eb19e7",
                "a4b2856bfec510abab89753fac1ac0e1112364e7d250545963f135f2a33188ed",
            ),
            (
                &[0][..],
                "68e0fe46dfb67e368c75379acec591dad19df3cde26e63b93a8e704f1dade7a3",
                "8c8a13df77a28f3445213a0f432fde644acaa215fc72dcdf300d5efaa85d350c",
            ),
            (
                &[0, 1][..],
                "b1d0bad404bf35da785a64ca1ac54b2617211d2777696fbffaf208f746ae84f2",
                "1932a5270f335bed617d5b935c80aedb1a35bd9fc1e31acafd5372c30f5c1187",
            ),
        ] {
            let k = derive_path(&seed, path);
            assert_eq!(*k, hex32(want_key), "private key at {path:?}");
            // SLIP-0010 prints Ed25519 public keys with a leading 00; the key
            // itself is the remaining 32 bytes.
            assert_eq!(public_key(&k), hex32(want_pub), "public key at {path:?}");
        }
    }

    /// Ed25519 has no non-hardened child, so asking for index 0 and index
    /// 0x80000000 has to mean the same thing rather than two different wallets.
    #[test]
    fn every_index_is_hardened() {
        let seed: Vec<u8> = (0u8..16).collect();
        assert_eq!(*derive_path(&seed, &[0]), *derive_path(&seed, &[HARDENED]));
        assert_eq!(
            *derive_path(&seed, &[5]),
            *derive_path(&seed, &[5 | HARDENED])
        );
    }

    /// Phantom's path, spelled out. A silent change here moves every Solana
    /// address this wallet has ever shown.
    #[test]
    fn the_path_is_phantoms() {
        assert_eq!(path_for(0), [44, 501, 0, 0]);
        assert_eq!(path_string(0), "m/44'/501'/0'/0'");
        assert_eq!(path_for(3), [44, 501, 3, 0]);
        assert_eq!(path_string(3), "m/44'/501'/3'/0'");
    }

    /// Real mainnet program ids, which are ordinary addresses. The system
    /// program is 32 zero bytes - the case where base58's leading-zero handling
    /// decides whether the string is 32 characters or 44.
    #[test]
    fn well_known_addresses_round_trip() {
        for s in [
            "11111111111111111111111111111111",
            "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
            "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
        ] {
            let a = SolanaAddress::parse(s).unwrap_or_else(|e| panic!("{s} did not parse: {e}"));
            assert_eq!(a.to_string(), s, "round trip changed the address");
            assert_eq!(a.as_bytes().len(), 32);
        }
        assert_eq!(
            SolanaAddress::parse("11111111111111111111111111111111")
                .unwrap()
                .as_bytes(),
            &[0u8; 32]
        );
    }

    /// An address from another chain must not be accepted here. There is no
    /// checksum to save anyone, so shape is the only guard this layer has.
    #[test]
    fn addresses_from_other_chains_are_rejected() {
        for s in [
            "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH",         // TRON, 21 bytes
            "0xA41811CF4D41e306310CB82B47258C22b80475cC", // EVM
            "",
            "0",                                              // not in the alphabet
            "1111111111111111111111111111111111111111111111", // too long
        ] {
            assert!(
                SolanaAddress::parse(s).is_err(),
                "{s:?} was accepted as a Solana address"
            );
        }
    }

    /// A signature has to verify against the address it claims to come from,
    /// or the transaction is rejected by the cluster and the fee is still paid.
    #[test]
    fn a_signature_verifies_against_the_derived_address() {
        let seed = [7u8; 64];
        let sk = private_key_at(&seed, 0).unwrap();
        let addr = address_from_private_key(&sk).unwrap();

        let msg = b"neko-wallet";
        let sig = sign(&sk, msg);

        use ed25519_dalek::Verifier;
        let vk = ed25519_dalek::VerifyingKey::from_bytes(addr.as_bytes()).unwrap();
        vk.verify(msg, &ed25519_dalek::Signature::from_bytes(&sig))
            .expect("the signature does not match the address it was derived from");
    }

    /// Different accounts are different wallets.
    #[test]
    fn each_index_is_a_distinct_address() {
        let seed = [9u8; 64];
        let a: Vec<String> = (0..4)
            .map(|i| address_at(&seed, i).unwrap().to_string())
            .collect();
        for i in 0..a.len() {
            for j in i + 1..a.len() {
                assert_ne!(a[i], a[j], "index {i} and {j} derived the same address");
            }
        }
    }
}
