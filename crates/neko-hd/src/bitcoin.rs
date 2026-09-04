//! Bitcoin addresses, and the BIP84 derivation that produces ours.
//!
//! Two roles that must not be collapsed:
//!
//! * **Ours.** Derived at `m/84'/0'/0'/0/{i}`, always P2WPKH - a witness
//!   version 0 program holding `HASH160(compressed public key)`, printed as
//!   `bc1q...`. This is what the wallet shows and what it signs for.
//! * **Anyone's.** A destination may be any of five script types, three of
//!   which predate segwit. Refusing to pay a `1...` address because this wallet
//!   does not derive one would make the wallet unusable for paying anybody who
//!   has an older one.
//!
//! So parsing is broad and derivation is narrow, and the type carries which
//! kind it is - because the script that locks the money, and the number of
//! bytes it costs, both follow from that.

use crate::error::HdError;
use ripemd::Ripemd160;
use sha2::{Digest, Sha256};

/// Mainnet only, like the rest of this program.
pub const HRP: &str = "bc";
/// Mainnet version bytes for the two base58check forms.
const P2PKH_VERSION: u8 = 0x00;
const P2SH_VERSION: u8 = 0x05;

/// SLIP-44 coin type for Bitcoin.
pub const COIN_TYPE_BTC: u32 = 0;
/// BIP84's purpose. The purpose *is* the script type: 44 means P2PKH, 49 means
/// P2SH-wrapped segwit, 84 means native segwit, 86 means Taproot. Deriving with
/// one and building the script for another produces an address nobody can
/// spend from.
pub const PURPOSE_BIP84: u32 = 84;

/// What locks an output.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum BtcAddress {
    /// `1...` - pay to public key hash.
    P2pkh([u8; 20]),
    /// `3...` - pay to script hash. Also how pre-Taproot wallets wrapped segwit.
    P2sh([u8; 20]),
    /// `bc1q...` (42 chars) - native segwit, what this wallet derives.
    P2wpkh([u8; 20]),
    /// `bc1q...` (62 chars) - a segwit script, usually multisig.
    P2wsh([u8; 32]),
    /// `bc1p...` - Taproot. Payable, not derivable here.
    P2tr([u8; 32]),
}

/// `RIPEMD160(SHA256(x))`. Bitcoin's hash for everything address-shaped.
pub fn hash160(data: &[u8]) -> [u8; 20] {
    let sha = Sha256::digest(data);
    let mut out = [0u8; 20];
    out.copy_from_slice(&Ripemd160::digest(sha));
    out
}

impl BtcAddress {
    /// Our own address, from a 33-byte compressed public key.
    ///
    /// Compressed, specifically: segwit v0 requires it, and a signature over an
    /// uncompressed key produces a different `HASH160`, so an uncompressed key
    /// here yields an address whose coins cannot be spent by the key that made
    /// it.
    pub fn p2wpkh_from_public_key(pubkey: &[u8]) -> Result<Self, HdError> {
        if pubkey.len() != 33 || !(pubkey[0] == 0x02 || pubkey[0] == 0x03) {
            return Err(HdError::BadPublicKeyLen(pubkey.len()));
        }
        Ok(BtcAddress::P2wpkh(hash160(pubkey)))
    }

    pub fn parse(s: &str) -> Result<Self, HdError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(HdError::BadBtcAddress);
        }
        if s.len() > 3 && s[..3].eq_ignore_ascii_case("bc1") {
            return Self::parse_bech32(s);
        }
        Self::parse_base58(s)
    }

    fn parse_bech32(s: &str) -> Result<Self, HdError> {
        let (hrp, data, variant) = crate::bech32::decode(s).map_err(|_| HdError::BadBtcAddress)?;
        if hrp != HRP {
            return Err(HdError::BadBtcAddress);
        }
        let (version, rest) = data.split_first().ok_or(HdError::BadBtcAddress)?;
        if *version > 16 {
            return Err(HdError::BadBtcAddress);
        }
        // The version decides the checksum constant. A Taproot address that
        // passed the bech32 check would be one whose checksum was computed for
        // a different format - which means it is not the address anyone meant.
        if variant != crate::bech32::Variant::for_witness_version(*version) {
            return Err(HdError::BadBtcAddress);
        }
        let program =
            crate::bech32::convert_bits(rest, 5, 8, false).ok_or(HdError::BadBtcAddress)?;

        match (*version, program.len()) {
            (0, 20) => Ok(BtcAddress::P2wpkh(fixed(&program)?)),
            (0, 32) => Ok(BtcAddress::P2wsh(fixed32(&program)?)),
            (1, 32) => Ok(BtcAddress::P2tr(fixed32(&program)?)),
            // Every other combination is either invalid today or a future
            // version whose spending rules are not defined yet. Paying one
            // would be paying into something this program cannot reason about.
            _ => Err(HdError::BadBtcAddress),
        }
    }

    fn parse_base58(s: &str) -> Result<Self, HdError> {
        let raw = bs58::decode(s)
            .with_check(None)
            .into_vec()
            .map_err(|_| HdError::BadBtcAddress)?;
        if raw.len() != 21 {
            return Err(HdError::BadBtcAddress);
        }
        let hash = fixed(&raw[1..])?;
        match raw[0] {
            P2PKH_VERSION => Ok(BtcAddress::P2pkh(hash)),
            P2SH_VERSION => Ok(BtcAddress::P2sh(hash)),
            // Testnet versions land here, which is the point: this wallet is
            // mainnet only, and a testnet address is a real address on a chain
            // where the coins are worthless.
            _ => Err(HdError::BadBtcAddress),
        }
    }

    /// The script that locks an output paying here.
    ///
    /// This is what actually goes in the transaction; the text form is only how
    /// people move it around.
    pub fn script_pubkey(&self) -> Vec<u8> {
        match self {
            // OP_DUP OP_HASH160 <20> OP_EQUALVERIFY OP_CHECKSIG
            BtcAddress::P2pkh(h) => {
                let mut v = vec![0x76, 0xa9, 0x14];
                v.extend_from_slice(h);
                v.extend_from_slice(&[0x88, 0xac]);
                v
            }
            // OP_HASH160 <20> OP_EQUAL
            BtcAddress::P2sh(h) => {
                let mut v = vec![0xa9, 0x14];
                v.extend_from_slice(h);
                v.push(0x87);
                v
            }
            // OP_0 <20>
            BtcAddress::P2wpkh(h) => {
                let mut v = vec![0x00, 0x14];
                v.extend_from_slice(h);
                v
            }
            // OP_0 <32>
            BtcAddress::P2wsh(h) => {
                let mut v = vec![0x00, 0x20];
                v.extend_from_slice(h);
                v
            }
            // OP_1 <32>
            BtcAddress::P2tr(h) => {
                let mut v = vec![0x51, 0x20];
                v.extend_from_slice(h);
                v
            }
        }
    }

    /// The 20-byte key hash, for the one script type this wallet can sign for.
    pub fn witness_key_hash(&self) -> Option<[u8; 20]> {
        match self {
            BtcAddress::P2wpkh(h) => Some(*h),
            _ => None,
        }
    }

    /// Storage form: the script, which is unique per address and is what an
    /// incoming payment is matched on.
    pub fn as_bytes(&self) -> Vec<u8> {
        self.script_pubkey()
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, HdError> {
        match b {
            [0x76, 0xa9, 0x14, h @ .., 0x88, 0xac] if h.len() == 20 => {
                Ok(BtcAddress::P2pkh(fixed(h)?))
            }
            [0xa9, 0x14, h @ .., 0x87] if h.len() == 20 => Ok(BtcAddress::P2sh(fixed(h)?)),
            [0x00, 0x14, h @ ..] if h.len() == 20 => Ok(BtcAddress::P2wpkh(fixed(h)?)),
            [0x00, 0x20, h @ ..] if h.len() == 32 => Ok(BtcAddress::P2wsh(fixed32(h)?)),
            [0x51, 0x20, h @ ..] if h.len() == 32 => Ok(BtcAddress::P2tr(fixed32(h)?)),
            _ => Err(HdError::BadBtcAddress),
        }
    }
}

fn fixed(b: &[u8]) -> Result<[u8; 20], HdError> {
    b.try_into().map_err(|_| HdError::BadBtcAddress)
}
fn fixed32(b: &[u8]) -> Result<[u8; 32], HdError> {
    b.try_into().map_err(|_| HdError::BadBtcAddress)
}

impl std::fmt::Display for BtcAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            BtcAddress::P2pkh(h) => base58check(P2PKH_VERSION, h),
            BtcAddress::P2sh(h) => base58check(P2SH_VERSION, h),
            BtcAddress::P2wpkh(h) => segwit(0, h),
            BtcAddress::P2wsh(h) => segwit(0, h),
            BtcAddress::P2tr(h) => segwit(1, h),
        };
        f.write_str(&s)
    }
}

impl std::fmt::Debug for BtcAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

fn base58check(version: u8, hash: &[u8]) -> String {
    let mut v = Vec::with_capacity(1 + hash.len());
    v.push(version);
    v.extend_from_slice(hash);
    bs58::encode(v).with_check().into_string()
}

fn segwit(version: u8, program: &[u8]) -> String {
    let mut data = vec![version];
    data.extend(
        crate::bech32::convert_bits(program, 8, 5, true).expect("8->5 with padding cannot fail"),
    );
    crate::bech32::encode(
        HRP,
        &data,
        crate::bech32::Variant::for_witness_version(version),
    )
}

// ── Derivation ─────────────────────────────────────────────────────────────

/// `m/84'/0'/{account}'/{change}/{index}`.
///
/// `change` is 0 for addresses shown to people and 1 for change returned to
/// yourself. Both are this wallet's own money; the split is a privacy
/// convention, not a security boundary.
pub fn path_for(account: u32, change: u32, index: u32) -> String {
    format!("m/{PURPOSE_BIP84}'/{COIN_TYPE_BTC}'/{account}'/{change}/{index}")
}

pub fn private_key_at(
    seed: &[u8; 64],
    account: u32,
    change: u32,
    index: u32,
) -> Result<crate::derive::PrivKey, HdError> {
    if index >= crate::derive::MAX_INDEX {
        return Err(HdError::IndexOutOfRange(index));
    }
    let xprv = crate::derive::derive_xprv(seed, &path_for(account, change, index))?;
    Ok(zeroize::Zeroizing::new(
        xprv.private_key().to_bytes().into(),
    ))
}

/// The 33-byte compressed public key for a private key.
pub fn compressed_public_key(sk: &[u8; 32]) -> Result<[u8; 33], HdError> {
    use k256::elliptic_curve::sec1::ToEncodedPoint;
    let secret = k256::SecretKey::from_slice(sk).map_err(|_| HdError::BadPrivateKey)?;
    let point = secret.public_key().to_encoded_point(true);
    let bytes = point.as_bytes();
    let mut out = [0u8; 33];
    if bytes.len() != 33 {
        return Err(HdError::BadPublicKeyLen(bytes.len()));
    }
    out.copy_from_slice(bytes);
    Ok(out)
}

pub fn address_from_private_key(sk: &[u8; 32]) -> Result<BtcAddress, HdError> {
    BtcAddress::p2wpkh_from_public_key(&compressed_public_key(sk)?)
}

pub fn address_at(
    seed: &[u8; 64],
    account: u32,
    change: u32,
    index: u32,
) -> Result<BtcAddress, HdError> {
    let sk = private_key_at(seed, account, change, index)?;
    address_from_private_key(&sk)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The mnemonic every BIP publishes vectors for.
    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    fn seed() -> zeroize::Zeroizing<[u8; 64]> {
        crate::derive::seed_from_mnemonic(ABANDON, "").unwrap()
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// BIP-84's own vectors.
    ///
    /// The load-bearing test in this file: it pins the derivation path, the
    /// compressed public key and the bech32 address together, against numbers
    /// published by the specification. Derivation that is subtly wrong does not
    /// fail - it produces a different valid address, and the money goes
    /// somewhere nobody can reach.
    #[test]
    fn matches_the_bip84_vectors() {
        let seed = seed();
        for (change, index, want_pub, want_addr) in [
            (
                0,
                0,
                "0330d54fd0dd420a6e5f8d3624f5f3482cae350f79d5f0753bf5beef9c2d91af3c",
                "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu",
            ),
            (
                0,
                1,
                "03e775fd51f0dfb8cd865d9ff1cca2a158cf651fe997fdc9fee9c1d3b5e995ea77",
                "bc1qnjg0jd8228aq7egyzacy8cys3knf9xvrerkf9g",
            ),
            // The first change address. Same account, the other branch.
            (
                1,
                0,
                "03025324888e429ab8e3dbaf1f7802648b9cd01e9b418485c5fa4c1b9b5700e1a6",
                "bc1q8c6fshw2dlwun7ekn9qwf37cu2rn755upcp6el",
            ),
        ] {
            let sk = private_key_at(&seed, 0, change, index).unwrap();
            assert_eq!(
                hex(&compressed_public_key(&sk).unwrap()),
                want_pub,
                "public key at m/84'/0'/0'/{change}/{index}"
            );
            assert_eq!(
                address_at(&seed, 0, change, index).unwrap().to_string(),
                want_addr,
                "address at m/84'/0'/0'/{change}/{index}"
            );
        }
    }

    /// The purpose is the script type. Deriving under 44' and building a segwit
    /// script - or the reverse - yields an address that is valid, empty, and
    /// unspendable by the key that made it.
    #[test]
    fn the_path_is_bip84s() {
        assert_eq!(path_for(0, 0, 0), "m/84'/0'/0'/0/0");
        assert_eq!(path_for(0, 1, 7), "m/84'/0'/0'/1/7");
        assert_eq!(path_for(2, 0, 3), "m/84'/0'/2'/0/3");
        assert_eq!(PURPOSE_BIP84, 84);
        assert_eq!(COIN_TYPE_BTC, 0);
    }

    /// Every script type this wallet can pay, with the bytes that lock the
    /// output. A wrong script sends real money into something unspendable.
    #[test]
    fn every_payable_script_type_parses_to_the_right_bytes() {
        for (addr, script) in [
            // Satoshi's address, the first P2PKH there was.
            (
                "1A1zP1eP5QGefi2DMPTfTL5SLmv7DivfNa",
                "76a91462e907b15cbf27d5425399ebf6f0fb50ebb88f1888ac",
            ),
            // BIP-173's P2WPKH and P2WSH vectors, with the scriptPubKeys the
            // BIP states for them.
            (
                "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4",
                "0014751e76e8199196d454941c45d1b3a323f1433bd6",
            ),
            (
                "bc1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3qccfmv3",
                "00201863143c14c5166804bd19203356da136c985678cd4d27a1b8c6329604903262",
            ),
        ] {
            let a = BtcAddress::parse(addr).unwrap_or_else(|e| panic!("{addr}: {e}"));
            assert_eq!(hex(&a.script_pubkey()), script, "script for {addr}");
            assert_eq!(a.to_string(), addr, "round trip changed {addr}");
            // And the script is enough to rebuild it, which is how a stored
            // address comes back.
            assert_eq!(BtcAddress::from_bytes(&a.script_pubkey()).unwrap(), a);
        }
    }

    /// Taproot is payable but not derivable. It has to survive a round trip and
    /// produce the version-1 script, or a payment lands somewhere else.
    #[test]
    fn taproot_is_payable() {
        let a = BtcAddress::parse("bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0")
            .unwrap();
        assert!(matches!(a, BtcAddress::P2tr(_)));
        let script = a.script_pubkey();
        assert_eq!(script[0], 0x51, "witness version 1 is OP_1");
        assert_eq!(script[1], 0x20, "a 32-byte program");
        assert_eq!(script.len(), 34);
        assert_eq!(
            a.to_string(),
            "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vqzk5jj0"
        );
        assert!(a.witness_key_hash().is_none(), "we cannot sign for Taproot");
    }

    /// Uppercase is valid bech32 - QR codes encode it more densely - so it has
    /// to be accepted and normalised rather than refused.
    #[test]
    fn an_uppercase_address_is_the_same_address() {
        let lower = BtcAddress::parse("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").unwrap();
        let upper = BtcAddress::parse("BC1QW508D6QEJXTDG4Y5R3ZARVARY0C5XW7KV8F3T4").unwrap();
        assert_eq!(lower, upper);
        assert_eq!(
            upper.to_string(),
            "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4"
        );
    }

    /// Everything that must not be accepted.
    ///
    /// Testnet is the one worth naming: `tb1...` and `m.../n...` are perfectly
    /// valid addresses on a chain where the coins are worth nothing, and a
    /// wallet that accepted one would send real money to it.
    #[test]
    fn what_must_not_parse() {
        for (s, why) in [
            (
                "tb1qw508d6qejxtdg4y5r3zarvary0c5xw7kxpjzsx",
                "testnet segwit",
            ),
            ("mipcBbFg9gMiCh81Kj8tqqdgoZub1ZJRfn", "testnet P2PKH"),
            ("2N2JD6wb56AFK4d9ppEmmemsKGXCJRAcTNb", "testnet P2SH"),
            (
                "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t5",
                "one character changed",
            ),
            (
                "1A1zP1eP5QGefi2DMPTfTL5SLmv7Divfna",
                "base58 checksum broken",
            ),
            // Taproot's checksum constant is bech32m; the same data under
            // bech32 is a different string that must not decode.
            (
                "bc1p0xlxvlhemja6c4dqv22uapctqupfhlxm9h8z3k2e72q4k9hcz7vq5zuyut",
                "P2TR under bech32",
            ),
            ("TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH", "a TRON address"),
            (
                "0xA41811CF4D41e306310CB82B47258C22b80475cC",
                "an EVM address",
            ),
            (
                "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM",
                "a Solana address",
            ),
            ("", "empty"),
        ] {
            assert!(BtcAddress::parse(s).is_err(), "{s:?} was accepted ({why})");
        }
    }

    /// Segwit v0 requires a compressed key. An uncompressed one hashes
    /// differently, so the address would not be spendable by the key that made
    /// it - which is a loss that looks exactly like a success until you try.
    #[test]
    fn an_uncompressed_key_is_refused() {
        let sk = [3u8; 32];
        let compressed = compressed_public_key(&sk).unwrap();
        assert!(BtcAddress::p2wpkh_from_public_key(&compressed).is_ok());

        use k256::elliptic_curve::sec1::ToEncodedPoint;
        let secret = k256::SecretKey::from_slice(&sk).unwrap();
        let uncompressed = secret.public_key().to_encoded_point(false);
        assert!(BtcAddress::p2wpkh_from_public_key(uncompressed.as_bytes()).is_err());
    }

    /// HASH160 is RIPEMD160 over SHA256, in that order. Swapping them produces
    /// a valid-looking address for a key that cannot spend from it.
    #[test]
    fn hash160_is_ripemd_of_sha() {
        // The empty string, whose SHA256 and RIPEMD160 are both published.
        assert_eq!(
            hex(&hash160(b"")),
            "b472a266d0bd89c13706a4132ccfb16f7c3b9fcb"
        );
        assert_eq!(
            hex(&hash160(b"abc")),
            "bb1be98c142444d7a56aa3981c3942a978e4dc33"
        );
    }
}
