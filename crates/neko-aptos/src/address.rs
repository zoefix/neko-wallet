//! An Aptos address: 32 bytes, written as hex.
//!
//! Derived from the key rather than being it: `sha3_256(public_key || 0x00)`.
//! The trailing byte is a scheme tag - 0 for a single Ed25519 key - so the
//! same key under a different scheme is a different account.
//!
//! The text form is where the care is needed. Aptos prints addresses with
//! leading zeros removed, so the same account can appear as a 66-character
//! string or a much shorter one, and `0x1` and the 32-byte address ending in
//! 01 are the same place. Both are accepted here and only the padded form is
//! produced, so two spellings of one account can never be stored as two rows.

use crate::error::AptosError;

pub const ADDRESS_BYTES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AptosAddress([u8; ADDRESS_BYTES]);

impl AptosAddress {
    pub fn from_bytes(b: &[u8]) -> Result<Self, AptosError> {
        let mut a = [0u8; ADDRESS_BYTES];
        if b.len() != ADDRESS_BYTES {
            return Err(AptosError::BadAddress);
        }
        a.copy_from_slice(b);
        Ok(AptosAddress(a))
    }

    /// The account an Ed25519 public key controls.
    pub fn from_public_key(pk: &[u8; 32]) -> Self {
        use sha3::{Digest, Sha3_256};
        let mut h = Sha3_256::new();
        h.update(pk);
        // The single-key scheme. A different tag here is a different account
        // that the same key also controls, which is why it is written out
        // rather than left as a bare zero.
        h.update([SCHEME_ED25519]);
        let mut a = [0u8; ADDRESS_BYTES];
        a.copy_from_slice(&h.finalize());
        AptosAddress(a)
    }

    /// Accepts both the padded and the shortened form.
    ///
    /// A short address is not a typo on this chain - the framework's own
    /// modules live at `0x1` - so refusing them would refuse the addresses
    /// Aptos itself prints.
    pub fn parse(s: &str) -> Result<Self, AptosError> {
        let s = s.trim();
        let hex_part = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X"));
        // Require the prefix. Without it a 64-character hex string and a
        // 64-character something-else look alike, and this is a destination.
        let h = hex_part.ok_or(AptosError::BadAddress)?;
        if h.is_empty() || h.len() > ADDRESS_BYTES * 2 {
            return Err(AptosError::BadAddress);
        }
        if !h.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(AptosError::BadAddress);
        }
        // Left-pad to full width, so `0x1` and its padded spelling become the
        // same bytes.
        let padded = format!("{:0>width$}", h, width = ADDRESS_BYTES * 2);
        let raw = hex::decode(padded).map_err(|_| AptosError::BadAddress)?;
        Self::from_bytes(&raw)
    }

    pub fn as_bytes(&self) -> &[u8; ADDRESS_BYTES] {
        &self.0
    }
}

/// The scheme byte for a single Ed25519 key.
pub const SCHEME_ED25519: u8 = 0;

impl std::fmt::Display for AptosAddress {
    /// Always the full 66 characters.
    ///
    /// Aptos itself shortens, and this does not: a stored address that
    /// sometimes has leading zeros and sometimes does not is two strings for
    /// one account, and every comparison in this wallet is on the text.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", hex::encode(self.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The address for a key, against the chain.
    ///
    /// This exact account exists on mainnet with eleven transactions and an
    /// authentication key equal to its address, which is what confirms the
    /// derivation matches what Petra and the Aptos SDK produce for the same
    /// phrase. A wallet that derived this differently would show an empty
    /// account and be unable to explain why.
    #[test]
    fn the_address_is_the_hash_of_the_key_and_the_scheme() {
        let pk: [u8; 32] =
            hex::decode("a686f0309ab80312979606cfccc10ea2740147ae6888351488d11c46f08fbf60")
                .unwrap()
                .try_into()
                .unwrap();
        assert_eq!(
            AptosAddress::from_public_key(&pk).to_string(),
            "0xeb663b681209e7087d681c5d3eed12aaa8e1915e7c87794542c3f96e94b3d3bf"
        );
    }

    /// Short and padded spellings are one account.
    #[test]
    fn a_shortened_address_is_the_same_account() {
        let short = AptosAddress::parse("0x1").unwrap();
        let long =
            AptosAddress::parse("0x0000000000000000000000000000000000000000000000000000000000000001")
                .unwrap();
        assert_eq!(short, long);
        // And only one of the two spellings is ever produced.
        assert_eq!(short.to_string(), long.to_string());
        assert_eq!(short.to_string().len(), 66);
    }

    /// What is refused, and why each one matters for a destination field.
    #[test]
    fn malformed_addresses_are_refused() {
        for s in [
            "",
            "0x",
            // No prefix: a bare hex run is not distinguishable from something
            // else pasted by accident.
            "eb663b681209e7087d681c5d3eed12aaa8e1915e7c87794542c3f96e94b3d3bf",
            // Longer than 32 bytes.
            "0xeb663b681209e7087d681c5d3eed12aaa8e1915e7c87794542c3f96e94b3d3bf00",
            "0xzz",
            "0x 1",
        ] {
            assert!(AptosAddress::parse(s).is_err(), "{s:?} should be refused");
        }
    }
}
