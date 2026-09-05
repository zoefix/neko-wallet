//! TON addresses.
//!
//! An address is a workchain number and a 256-bit hash - and the hash is the
//! hash of a *contract's initial state*, because on this chain there is nothing
//! else to be. There is no key-to-address function; there is a key, which goes
//! into a contract's storage, which with the contract's code hashes to a place.
//!
//! Two text forms, and a wallet has to read both:
//!
//! * **Raw**, `0:b113a9…`, which says exactly what it is and nothing else.
//! * **Friendly**, `EQCxE6mU…`, which is base64url over the same bytes plus
//!   flags and a CRC-16. The flags carry two facts worth more than the
//!   convenience: whether the address is *bounceable*, and whether it is
//!   testnet. Sending to the wrong one of the first is how funds come back or
//!   do not; the second is a real address on a chain where coins are worthless.

use crate::error::TonError;

/// The workchain ordinary accounts live in. Masterchain is -1 and is for
/// validators; a wallet there would work and cost far more.
pub const BASECHAIN: i8 = 0;

const FLAG_BOUNCEABLE: u8 = 0x11;
const FLAG_NON_BOUNCEABLE: u8 = 0x51;
const FLAG_TESTNET: u8 = 0x80;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct TonAddress {
    pub workchain: i8,
    pub hash: [u8; 32],
    /// How it was written, and how it will be written back.
    ///
    /// Not part of the address - the same account is both - but it is part of
    /// what somebody pasted, and echoing it back differently makes a correct
    /// address look wrong.
    pub bounceable: bool,
}

impl TonAddress {
    pub fn new(workchain: i8, hash: [u8; 32]) -> Self {
        TonAddress {
            workchain,
            hash,
            // What a contract-to-contract payment uses, and the default a
            // wallet shows. A payment to a *wallet* is usually sent
            // non-bounceable, but the address itself is the same account.
            bounceable: true,
        }
    }

    pub fn parse(s: &str) -> Result<Self, TonError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(TonError::Address(neko_hd::HdError::BadTonAddress));
        }
        if s.contains(':') {
            return Self::parse_raw(s);
        }
        Self::parse_friendly(s)
    }

    fn parse_raw(s: &str) -> Result<Self, TonError> {
        let (wc, hash) = s
            .split_once(':')
            .ok_or(TonError::Address(neko_hd::HdError::BadTonAddress))?;
        let workchain: i8 = wc
            .parse()
            .map_err(|_| TonError::Address(neko_hd::HdError::BadTonAddress))?;
        let raw =
            hex::decode(hash).map_err(|_| TonError::Address(neko_hd::HdError::BadTonAddress))?;
        let hash: [u8; 32] = raw
            .try_into()
            .map_err(|_| TonError::Address(neko_hd::HdError::BadTonAddress))?;
        Ok(TonAddress::new(workchain, hash))
    }

    fn parse_friendly(s: &str) -> Result<Self, TonError> {
        let bad = || TonError::Address(neko_hd::HdError::BadTonAddress);
        // base64url, and plain base64 too: the two differ in two characters and
        // addresses get pasted through things that transcode them.
        let raw = base64_decode(s).ok_or_else(bad)?;
        if raw.len() != 36 {
            return Err(bad());
        }
        let crc = crc16(&raw[..34]);
        if crc.to_be_bytes() != raw[34..36] {
            return Err(bad());
        }
        let flags = raw[0];
        if flags & FLAG_TESTNET != 0 {
            // A real address on a chain where the coins are worth nothing.
            return Err(bad());
        }
        let bounceable = match flags & !FLAG_TESTNET {
            FLAG_BOUNCEABLE => true,
            FLAG_NON_BOUNCEABLE => false,
            _ => return Err(bad()),
        };
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&raw[2..34]);
        Ok(TonAddress {
            workchain: raw[1] as i8,
            hash,
            bounceable,
        })
    }

    /// `0:b113a9…`. What the protocol and every API use.
    pub fn to_raw_string(&self) -> String {
        format!("{}:{}", self.workchain, hex::encode(self.hash))
    }

    /// `EQCxE6mU…`, in the form it was read in.
    pub fn to_friendly_string(&self) -> String {
        let mut raw = Vec::with_capacity(36);
        raw.push(if self.bounceable {
            FLAG_BOUNCEABLE
        } else {
            FLAG_NON_BOUNCEABLE
        });
        raw.push(self.workchain as u8);
        raw.extend_from_slice(&self.hash);
        raw.extend_from_slice(&crc16(&raw).to_be_bytes());
        base64_encode(&raw)
    }

    /// Storage form: workchain and hash, which is the address itself. The
    /// bounceable flag is presentation and is not stored.
    pub fn as_bytes(&self) -> Vec<u8> {
        let mut v = Vec::with_capacity(33);
        v.push(self.workchain as u8);
        v.extend_from_slice(&self.hash);
        v
    }

    pub fn from_bytes(b: &[u8]) -> Result<Self, TonError> {
        if b.len() != 33 {
            return Err(TonError::Address(neko_hd::HdError::BadTonAddress));
        }
        let mut hash = [0u8; 32];
        hash.copy_from_slice(&b[1..]);
        Ok(TonAddress::new(b[0] as i8, hash))
    }
}

impl std::fmt::Display for TonAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_friendly_string())
    }
}

impl std::fmt::Debug for TonAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_raw_string())
    }
}

/// CRC-16/XMODEM, which is what the friendly form checksums with.
fn crc16(data: &[u8]) -> u16 {
    let mut crc: u16 = 0;
    for b in data {
        crc ^= (*b as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 {
                (crc << 1) ^ 0x1021
            } else {
                crc << 1
            };
        }
    }
    crc
}

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

fn base64_encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for c in input.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(B64URL[(n >> 18) as usize & 63] as char);
        out.push(B64URL[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            B64URL[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            B64URL[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

/// Accepts both base64 alphabets: `+/` and `-_` mean the same bytes, and an
/// address copied through a URL or a chat client may arrive as either.
fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for ch in s.bytes() {
        if ch == b'=' {
            break;
        }
        let v = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tether's jetton master, in both forms, as the chain and its explorers
    /// give them. One pair checks the flags, the CRC-16 and the base64url
    /// alphabet at once.
    const USDT_FRIENDLY: &str = "EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs";
    const USDT_RAW: &str = "0:b113a994b5024a16719f69139328eb759596c38a25f59028b146fecdc3621dfe";

    #[test]
    fn the_two_forms_are_the_same_address() {
        let a = TonAddress::parse(USDT_FRIENDLY).unwrap();
        let b = TonAddress::parse(USDT_RAW).unwrap();
        assert_eq!(a.hash, b.hash);
        assert_eq!(a.workchain, 0);
        assert_eq!(a.to_raw_string(), USDT_RAW);
        assert_eq!(a.to_friendly_string(), USDT_FRIENDLY);
        assert!(a.bounceable, "EQ… is the bounceable prefix");
    }

    /// The checksum is what makes a mistyped address fail rather than land
    /// somewhere real. Every single-character change has to break it.
    #[test]
    fn one_wrong_character_is_caught() {
        const ALPHABET: &[u8] = B64URL;
        let good = USDT_FRIENDLY.as_bytes();
        let mut checked = 0;
        for i in 0..good.len() {
            for c in ALPHABET.iter().copied() {
                if c == good[i] {
                    continue;
                }
                let mut bad = good.to_vec();
                bad[i] = c;
                let s = String::from_utf8(bad).unwrap();
                assert!(
                    TonAddress::parse(&s).is_err(),
                    "{s} was accepted as an address"
                );
                checked += 1;
            }
        }
        assert!(checked > 2_000, "the sweep did not run: {checked}");
    }

    /// Bounceable and non-bounceable are the same account written two ways, and
    /// the form somebody pasted is the form they get back - an address that
    /// looks different from the one they sent reads as the wrong address.
    #[test]
    fn the_bounceable_flag_survives_a_round_trip() {
        let bounceable = TonAddress::parse(USDT_FRIENDLY).unwrap();
        let non = TonAddress {
            bounceable: false,
            ..bounceable
        };
        let text = non.to_friendly_string();
        assert!(text.starts_with("UQ"), "non-bounceable starts UQ: {text}");
        let back = TonAddress::parse(&text).unwrap();
        assert_eq!(back.hash, bounceable.hash, "same account");
        assert!(!back.bounceable);
        assert_eq!(back.to_friendly_string(), text);
    }

    /// Testnet addresses are valid addresses on a chain where the coins are
    /// worth nothing, and they use the same alphabet.
    #[test]
    fn what_must_not_parse() {
        // The same account with the testnet flag set.
        let mut raw = vec![FLAG_BOUNCEABLE | FLAG_TESTNET, 0u8];
        raw.extend_from_slice(&TonAddress::parse(USDT_RAW).unwrap().hash);
        raw.extend_from_slice(&crc16(&raw).to_be_bytes());
        let testnet = base64_encode(&raw);

        for (s, why) in [
            (testnet.as_str(), "testnet"),
            (
                "EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sD",
                "truncated",
            ),
            (
                "0:b113a994b5024a16719f69139328eb759596c38a25f59028b146fecdc3621d",
                "raw too short",
            ),
            (
                "bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4",
                "a Bitcoin address",
            ),
            (
                "0xA41811CF4D41e306310CB82B47258C22b80475cC",
                "an EVM address",
            ),
            ("", "empty"),
        ] {
            assert!(TonAddress::parse(s).is_err(), "{s:?} was accepted ({why})");
        }
    }

    /// Both base64 alphabets mean the same bytes, and an address pasted through
    /// a URL or a chat client may arrive as either.
    #[test]
    fn either_base64_alphabet_is_accepted() {
        let url = USDT_FRIENDLY;
        let standard = url.replace('-', "+").replace('_', "/");
        assert_ne!(url, standard, "this vector should exercise the difference");
        assert_eq!(
            TonAddress::parse(&standard).unwrap().hash,
            TonAddress::parse(url).unwrap().hash
        );
    }

    /// The stored form is the account, and nothing else.
    #[test]
    fn storage_round_trips() {
        let a = TonAddress::parse(USDT_FRIENDLY).unwrap();
        let bytes = a.as_bytes();
        assert_eq!(bytes.len(), 33);
        assert_eq!(TonAddress::from_bytes(&bytes).unwrap().hash, a.hash);
        assert!(TonAddress::from_bytes(&bytes[..32]).is_err());
    }
}
