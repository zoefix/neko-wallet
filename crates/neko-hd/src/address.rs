//! TRON address encoding and decoding.
//!
//! A TRON address is 21 bytes: the constant prefix `0x41` followed by the low
//! 20 bytes of `keccak256(uncompressed_public_key[1..])`. The user-facing form
//! is Base58Check over those 21 bytes, which always starts with `T`.
//!
//! The 21-byte form is canonical in memory: comparing addresses byte-wise is
//! both faster and safer than comparing base58 strings.

use crate::error::HdError;
use sha2::{Digest as _, Sha256};
use sha3::Keccak256;

pub const ADDRESS_LEN: usize = 21;
pub const PREFIX: u8 = 0x41;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Address([u8; ADDRESS_LEN]);

impl Address {
    /// `0x41 || keccak256(pubkey[1..])[12..]`
    ///
    /// The public key must be the **uncompressed** 65-byte SEC1 form. BIP32
    /// hands you a 33-byte compressed key, so decompress first — getting this
    /// wrong yields a valid-looking address nobody holds the key to.
    pub fn from_public_key(pubkey: &[u8]) -> Result<Self, HdError> {
        if pubkey.len() != 65 {
            return Err(HdError::BadPublicKeyLen(pubkey.len()));
        }
        if pubkey[0] != 0x04 {
            return Err(HdError::PublicKeyNotUncompressed);
        }
        let digest = Keccak256::digest(&pubkey[1..]);
        let mut a = [0u8; ADDRESS_LEN];
        a[0] = PREFIX;
        a[1..].copy_from_slice(&digest[12..]);
        Ok(Address(a))
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, HdError> {
        if b.len() != ADDRESS_LEN {
            return Err(HdError::BadAddressLen(b.len()));
        }
        if b[0] != PREFIX {
            return Err(HdError::BadAddressPrefix(b[0]));
        }
        let mut a = [0u8; ADDRESS_LEN];
        a.copy_from_slice(b);
        Ok(Address(a))
    }

    pub fn as_bytes(&self) -> &[u8; ADDRESS_LEN] {
        &self.0
    }

    /// Lowercase `41...` hex — the form every TronGrid HTTP body wants.
    pub fn to_hex(self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The 20-byte form used for ABI arguments, i.e. **without** the 0x41
    /// prefix. This asymmetry is a TRON-specific trap.
    pub fn to_evm_bytes(self) -> [u8; 20] {
        let mut out = [0u8; 20];
        out.copy_from_slice(&self.0[1..]);
        out
    }

    /// Rebuild from a 20-byte ABI/event address by re-adding the prefix.
    pub fn from_evm_bytes(b: &[u8]) -> Result<Self, HdError> {
        if b.len() != 20 {
            return Err(HdError::BadAddressLen(b.len()));
        }
        let mut a = [0u8; ADDRESS_LEN];
        a[0] = PREFIX;
        a[1..].copy_from_slice(b);
        Ok(Address(a))
    }

    pub fn parse(s: &str) -> Result<Self, HdError> {
        let raw = bs58::decode(s).into_vec().map_err(|_| HdError::BadBase58)?;
        // 21 payload + 4 checksum. Check length before touching the checksum so
        // a truncated string cannot panic.
        if raw.len() != ADDRESS_LEN + 4 {
            return Err(HdError::BadAddressLen(raw.len()));
        }
        let (payload, want) = raw.split_at(ADDRESS_LEN);
        if payload[0] != PREFIX {
            return Err(HdError::BadAddressPrefix(payload[0]));
        }
        if checksum(payload) != want {
            return Err(HdError::BadChecksum);
        }
        Address::from_bytes(payload)
    }
}

/// Base58Check's checksum: the first 4 bytes of a double SHA-256.
fn checksum(payload: &[u8]) -> [u8; 4] {
    let d = Sha256::digest(Sha256::digest(payload));
    let mut c = [0u8; 4];
    c.copy_from_slice(&d[..4]);
    c
}

impl std::fmt::Display for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&bs58::encode(self.0).with_check().into_string())
    }
}

impl std::fmt::Debug for Address {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Address({self})")
    }
}

impl std::str::FromStr for Address {
    type Err = HdError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Address::parse(s)
    }
}
