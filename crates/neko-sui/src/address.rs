//! A Sui address: 32 bytes, written as hex with a `0x` prefix.
//!
//! `blake2b256(scheme || public_key)` - note the order, which is the opposite
//! of Aptos's, where the scheme byte goes last. Two chains, two Ed25519 keys,
//! two hashes, and swapping the operands produces a valid-looking address for
//! an account nobody controls.
//!
//! Verified against mainnet rather than against a document: two real
//! transactions were taken from the chain, the signer's public key read out of
//! the signature, and the address recomputed. Both matched.

use crate::error::SuiError;

pub const ADDRESS_BYTES: usize = 32;

/// The signature scheme byte. `0x00` is Ed25519; `0x01` and `0x02` are the two
/// secp curves, which this wallet does not use here.
pub const SCHEME_ED25519: u8 = 0x00;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SuiAddress([u8; ADDRESS_BYTES]);

impl SuiAddress {
    pub fn from_bytes(b: &[u8]) -> Result<Self, SuiError> {
        if b.len() != ADDRESS_BYTES {
            return Err(SuiError::BadAddress);
        }
        let mut a = [0u8; ADDRESS_BYTES];
        a.copy_from_slice(b);
        Ok(SuiAddress(a))
    }

    /// The account an Ed25519 public key controls.
    pub fn from_public_key(pk: &[u8; 32]) -> Self {
        let mut a = [0u8; ADDRESS_BYTES];
        a.copy_from_slice(&crate::blake2b256(&[&[SCHEME_ED25519][..], &pk[..]].concat()));
        SuiAddress(a)
    }

    /// Sui always prints the full width, and this insists on it.
    ///
    /// Unlike Aptos, a shortened address is not something the chain itself
    /// produces, so accepting one would only widen what a mistyped
    /// destination can be.
    pub fn parse(s: &str) -> Result<Self, SuiError> {
        let s = s.trim();
        let h = s
            .strip_prefix("0x")
            .or_else(|| s.strip_prefix("0X"))
            .ok_or(SuiError::BadAddress)?;
        if h.len() != ADDRESS_BYTES * 2 || !h.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(SuiError::BadAddress);
        }
        let raw = hex::decode(h).map_err(|_| SuiError::BadAddress)?;
        Self::from_bytes(&raw)
    }

    pub fn as_bytes(&self) -> &[u8; ADDRESS_BYTES] {
        &self.0
    }
}

impl std::fmt::Display for SuiAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against two real signers on mainnet.
    ///
    /// Each public key here was read out of the signature on a transaction
    /// that chain accepted, and the address recomputed from it matches the
    /// sender the chain recorded. That is what proves the hash and the operand
    /// order, which no amount of reading can.
    #[test]
    fn the_address_is_blake2b_of_the_scheme_and_the_key() {
        for (pk_hex, want) in [
            // The reference phrase this wallet derives from.
            (
                "900b4d81eecea3df2f74b14200c4f4cf3f49afaca7a634ffd2cf6ff82bdaecf2",
                "0x5e93a736d04fbb25737aa40bee40171ef79f65fae833749e3c089fe7cc2161f1",
            ),
            // And two strangers, whose keys were read out of the signatures
            // on transactions mainnet accepted. Their addresses are what the
            // chain itself recorded as the sender.
            (
                "0d862424c2c6a87d85f770b95b123f9c871bf4c2fb7b86b0af1501e02306d9cb",
                "0x7698719143381c549cdcfa502ab2233d867917f3eacd96cea17aa30d7f4d3a07",
            ),
            (
                "872e6878e123487375d294ffddc200fd49a9b6400a76b34b9bf6fbe03ad8a43b",
                "0x4440be690d52c31e71dc38d7f5a4b7f8fe98187658098101fb238d39745482fe",
            ),
        ] {
            let pk: [u8; 32] = hex::decode(pk_hex).unwrap().try_into().unwrap();
            assert_eq!(SuiAddress::from_public_key(&pk).to_string(), want);
        }
    }

    /// The scheme byte goes *first* here and last on Aptos. Swapping them
    /// produces a different, valid-looking address that nobody holds the key
    /// to, so the difference is worth its own assertion.
    #[test]
    fn the_scheme_byte_leads_rather_than_trails() {
        let pk = [7u8; 32];
        let ours = SuiAddress::from_public_key(&pk);
        let swapped = {
            let mut v = pk.to_vec();
            v.push(SCHEME_ED25519);
            crate::blake2b256(&v)
        };
        assert_ne!(ours.as_bytes()[..], swapped[..]);
    }

    #[test]
    fn malformed_addresses_are_refused() {
        for s in [
            "",
            "0x",
            "0x1",
            "5e93a736d04fbb25737aa40bee40171ef79f65fae833749e3c089fe7cc2161f1",
            "0x5e93a736d04fbb25737aa40bee40171ef79f65fae833749e3c089fe7cc2161f",
            "0xzz93a736d04fbb25737aa40bee40171ef79f65fae833749e3c089fe7cc2161f1",
        ] {
            assert!(SuiAddress::parse(s).is_err(), "{s:?} should be refused");
        }
    }
}
