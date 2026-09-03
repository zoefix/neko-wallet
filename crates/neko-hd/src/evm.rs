//! EVM addresses (BNB Chain, and any other EVM chain later).
//!
//! The 20 bytes are computed exactly as TRON's are - `keccak256` of the
//! uncompressed public key without its `0x04` tag, last 20 bytes. TRON then
//! prefixes `0x41` and encodes base58check; EVM chains print hex instead. The
//! shared construction is why both live beside the derivation code rather than
//! in their chain crates.
//!
//! The text form carries a checksum, but an unusual one: EIP-55 hides it in the
//! *capitalisation* of the hex digits, so an all-lowercase address is still
//! valid and unchecked. That matters here - refusing lowercase would reject
//! addresses copied from perfectly ordinary places, while accepting mixed case
//! without verifying it would waive the only typo protection the format has.

use crate::error::HdError;

pub const ADDRESS_LEN: usize = 20;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct EvmAddress([u8; ADDRESS_LEN]);

impl EvmAddress {
    /// `keccak256(pubkey[1..])[12..]`, where `pubkey` is the 65-byte
    /// uncompressed form.
    pub fn from_public_key(pubkey: &[u8]) -> Result<Self, HdError> {
        if pubkey.len() != 65 || pubkey[0] != 0x04 {
            return Err(HdError::BadPublicKeyLen(pubkey.len()));
        }
        let hash = keccak(&pubkey[1..]);
        let mut a = [0u8; ADDRESS_LEN];
        a.copy_from_slice(&hash[12..]);
        Ok(EvmAddress(a))
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, HdError> {
        if b.len() != ADDRESS_LEN {
            return Err(HdError::BadEvmAddressLen(b.len() * 2));
        }
        let mut a = [0u8; ADDRESS_LEN];
        a.copy_from_slice(b);
        Ok(EvmAddress(a))
    }

    pub fn as_bytes(&self) -> &[u8; ADDRESS_LEN] {
        &self.0
    }

    /// Parse `0x`-prefixed hex.
    ///
    /// A mixed-case address is checked against its EIP-55 checksum; an
    /// all-lowercase or all-uppercase one has no checksum to check and is
    /// accepted as-is. Both cases are deliberate: mixed case that fails is a
    /// typo and must be refused, while single-case is simply the older format.
    pub fn parse(s: &str) -> Result<Self, HdError> {
        let body = s
            .strip_prefix("0x")
            .or_else(|| s.strip_prefix("0X"))
            .ok_or(HdError::MissingHexPrefix)?;
        if body.len() != ADDRESS_LEN * 2 {
            return Err(HdError::BadEvmAddressLen(body.len()));
        }
        if !body.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(HdError::NotHex);
        }
        let mut a = [0u8; ADDRESS_LEN];
        for i in 0..ADDRESS_LEN {
            a[i] = u8::from_str_radix(&body[i * 2..i * 2 + 2], 16).map_err(|_| HdError::NotHex)?;
        }
        let addr = EvmAddress(a);

        let has_upper = body.bytes().any(|c| c.is_ascii_uppercase());
        let has_lower = body.bytes().any(|c| c.is_ascii_lowercase());
        if has_upper && has_lower && addr.to_string() != format!("0x{body}") {
            return Err(HdError::BadEip55Checksum);
        }
        Ok(addr)
    }
}

/// EIP-55: capitalise hex digit `i` when nibble `i` of `keccak256(lowercase
/// hex)` is 8 or greater.
impl std::fmt::Display for EvmAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let lower: String = self.0.iter().map(|b| format!("{b:02x}")).collect();
        let hash = keccak(lower.as_bytes());
        f.write_str("0x")?;
        for (i, c) in lower.chars().enumerate() {
            let nibble = if i % 2 == 0 {
                hash[i / 2] >> 4
            } else {
                hash[i / 2] & 0x0f
            };
            if nibble >= 8 {
                write!(f, "{}", c.to_ascii_uppercase())?;
            } else {
                write!(f, "{c}")?;
            }
        }
        Ok(())
    }
}

impl std::fmt::Debug for EvmAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EvmAddress({self})")
    }
}

impl std::str::FromStr for EvmAddress {
    type Err = HdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        EvmAddress::parse(s)
    }
}

fn keccak(data: &[u8]) -> [u8; 32] {
    use sha3::{Digest, Keccak256};
    let mut h = Keccak256::new();
    h.update(data);
    h.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four vectors from EIP-55 itself.
    #[test]
    fn eip55_matches_the_specification() {
        for want in [
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed",
            "0xfB6916095ca1df60bB79Ce92cE3Ea74c37c5d359",
            "0xdbF03B407c01E7cD3CBea99509d93f8DDDC8C6FB",
            "0xD1220A0cf47c7B9Be7A2E6BA89F429762e7b9aDb",
        ] {
            let a = EvmAddress::parse(&want.to_lowercase()).unwrap();
            assert_eq!(a.to_string(), want, "EIP-55 casing is wrong");
            // And the checksummed form parses back to the same bytes.
            assert_eq!(EvmAddress::parse(want).unwrap(), a);
        }
    }

    /// Mixed case is a checksum and must be verified; single case is the older
    /// format and carries none.
    #[test]
    fn a_mistyped_checksummed_address_is_refused() {
        let good = "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed";
        assert!(EvmAddress::parse(good).is_ok());

        // One letter's case flipped: same bytes, broken checksum.
        let bad = good.replacen('A', "a", 1);
        assert_ne!(bad, good);
        assert!(
            EvmAddress::parse(&bad).is_err(),
            "a broken EIP-55 checksum was accepted"
        );

        assert!(
            EvmAddress::parse(&good.to_lowercase()).is_ok(),
            "lowercase is valid"
        );
        assert!(
            EvmAddress::parse(&format!("0x{}", good[2..].to_uppercase())).is_ok(),
            "uppercase is valid"
        );
    }

    #[test]
    fn malformed_addresses_are_refused() {
        for s in [
            "",
            "0x",
            "5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAed", // no prefix
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAe", // one short
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAedd", // one long
            "0x5aAeb6053F3E94C9b9A09f33669435E7Ef1BeAeZ", // not hex
            "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t",       // a TRON address
        ] {
            assert!(EvmAddress::parse(s).is_err(), "{s:?} was accepted");
        }
    }

    /// The 20 bytes are the same construction TRON uses; only the text form
    /// differs. Pinning that keeps the two from drifting apart.
    #[test]
    fn shares_its_bytes_with_the_tron_address() {
        // secp256k1 generator point, uncompressed.
        let pk = hex_lit("0479be667ef9dcbbac55a06295ce870b07029bfcdb2dce28d959f2815b16f81798483ada7726a3c4655da4fbfc0e1108a8fd17b448a68554199c47d08ffb10d4b8");
        let evm = EvmAddress::from_public_key(&pk).unwrap();
        let tron = crate::address::Address::from_public_key(&pk).unwrap();
        assert_eq!(evm.as_bytes(), &tron.to_evm_bytes());
    }

    fn hex_lit(s: &str) -> Vec<u8> {
        (0..s.len() / 2)
            .map(|i| u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap())
            .collect()
    }
}
