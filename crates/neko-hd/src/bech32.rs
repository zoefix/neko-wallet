//! Bech32 and bech32m, the encodings segwit addresses use.
//!
//! Worth writing out rather than pulling in: it is a hundred lines, it sits in
//! the path that decides where money goes, and the checksum is the whole point
//! of the format. Getting it right is verified against BIP-173's and BIP-350's
//! own vectors, including the ones designed to be *rejected*.
//!
//! The two variants differ by a single constant, and mixing them up is a real
//! failure mode: witness version 0 uses bech32, every later version uses
//! bech32m, and an address checked against the wrong one is refused - which is
//! the safe direction, but only because the check is done at all.

/// Bech32's alphabet. Deliberately excludes `1`, `b`, `i` and `o`, the
/// characters people confuse when reading an address aloud or off a screen.
const CHARSET: &[u8; 32] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

const GENERATOR: [u32; 5] = [
    0x3b6a_57b2,
    0x2650_8e6d,
    0x1ea1_19fa,
    0x3d42_33dd,
    0x2a14_62b3,
];

/// The constant the checksum is expected to equal. This is the only difference
/// between the two encodings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variant {
    /// Witness version 0: P2WPKH and P2WSH.
    Bech32,
    /// Witness version 1 and later: Taproot.
    Bech32m,
}

impl Variant {
    fn constant(self) -> u32 {
        match self {
            Variant::Bech32 => 1,
            Variant::Bech32m => 0x2bc8_30a3,
        }
    }

    /// Which encoding a witness version must use. Fixed by BIP-350.
    pub fn for_witness_version(v: u8) -> Self {
        if v == 0 {
            Variant::Bech32
        } else {
            Variant::Bech32m
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bech32Error {
    BadCharacter,
    MixedCase,
    NoSeparator,
    BadChecksum,
    TooLong,
    BadHrp,
    BadPadding,
}

fn polymod(values: &[u8]) -> u32 {
    let mut chk: u32 = 1;
    for &v in values {
        let top = chk >> 25;
        chk = ((chk & 0x1ff_ffff) << 5) ^ (v as u32);
        for (i, g) in GENERATOR.iter().enumerate() {
            if (top >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

fn hrp_expand(hrp: &str) -> Vec<u8> {
    let mut v: Vec<u8> = hrp.bytes().map(|b| b >> 5).collect();
    v.push(0);
    v.extend(hrp.bytes().map(|b| b & 31));
    v
}

/// Regroup bits. `pad` is set when encoding, because 8-bit data rarely divides
/// into 5-bit groups; it must be *unset* when decoding, so that a payload with
/// stray bits is rejected rather than silently truncated.
pub fn convert_bits(data: &[u8], from: u32, to: u32, pad: bool) -> Option<Vec<u8>> {
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let mut out = Vec::with_capacity(data.len() * from as usize / to as usize + 1);
    let max = (1u32 << to) - 1;
    for &value in data {
        if (value as u32) >> from != 0 {
            return None;
        }
        acc = (acc << from) | value as u32;
        bits += from;
        while bits >= to {
            bits -= to;
            out.push(((acc >> bits) & max) as u8);
        }
    }
    if pad {
        if bits > 0 {
            out.push(((acc << (to - bits)) & max) as u8);
        }
    } else if bits >= from || ((acc << (to - bits)) & max) != 0 {
        // Left-over bits that are not zero mean the payload was not a whole
        // number of bytes. Accepting it would decode two different strings to
        // the same address.
        return None;
    }
    Some(out)
}

/// Encode 5-bit `data` under `hrp`.
pub fn encode(hrp: &str, data: &[u8], variant: Variant) -> String {
    let mut values = hrp_expand(hrp);
    values.extend_from_slice(data);
    values.extend_from_slice(&[0; 6]);
    let m = polymod(&values) ^ variant.constant();
    let checksum: Vec<u8> = (0..6).map(|i| ((m >> (5 * (5 - i))) & 31) as u8).collect();

    let mut out = String::with_capacity(hrp.len() + 1 + data.len() + 6);
    out.push_str(hrp);
    out.push('1');
    for b in data.iter().chain(checksum.iter()) {
        out.push(CHARSET[*b as usize] as char);
    }
    out
}

/// Decode to `(hrp, 5-bit data, variant)`.
///
/// Case is significant only in that it must not be mixed: BIP-173 allows an
/// address in either case, because uppercase encodes more densely in a QR code,
/// but a mixture is how a corrupted string looks.
pub fn decode(s: &str) -> Result<(String, Vec<u8>, Variant), Bech32Error> {
    if s.len() > 90 {
        return Err(Bech32Error::TooLong);
    }
    let has_lower = s.chars().any(|c| c.is_ascii_lowercase());
    let has_upper = s.chars().any(|c| c.is_ascii_uppercase());
    if has_lower && has_upper {
        return Err(Bech32Error::MixedCase);
    }
    let s = s.to_ascii_lowercase();

    let sep = s.rfind('1').ok_or(Bech32Error::NoSeparator)?;
    if sep == 0 || sep + 7 > s.len() {
        return Err(Bech32Error::NoSeparator);
    }
    let hrp = &s[..sep];
    if hrp.bytes().any(|b| !(33..=126).contains(&b)) {
        return Err(Bech32Error::BadHrp);
    }

    let mut data = Vec::with_capacity(s.len() - sep - 1);
    for c in s[sep + 1..].bytes() {
        let v = CHARSET
            .iter()
            .position(|&x| x == c)
            .ok_or(Bech32Error::BadCharacter)?;
        data.push(v as u8);
    }

    let mut values = hrp_expand(hrp);
    values.extend_from_slice(&data);
    let m = polymod(&values);
    let variant = if m == Variant::Bech32.constant() {
        Variant::Bech32
    } else if m == Variant::Bech32m.constant() {
        Variant::Bech32m
    } else {
        return Err(Bech32Error::BadChecksum);
    };

    data.truncate(data.len() - 6);
    Ok((hrp.to_string(), data, variant))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BIP-173's and BIP-350's valid strings. Round-tripping every one of them
    /// pins the checksum, the alphabet and the separator rule at once.
    #[test]
    fn the_bip_vectors_round_trip() {
        for (s, variant) in [
            ("A12UEL5L", Variant::Bech32),
            ("a12uel5l", Variant::Bech32),
            (
                "an83characterlonghumanreadablepartthatcontainsthenumber1andtheexcludedcharactersbio1tt5tgs",
                Variant::Bech32,
            ),
            ("abcdef1qpzry9x8gf2tvdw0s3jn54khce6mua7lmqqqxw", Variant::Bech32),
            ("split1checkupstagehandshakeupstreamerranterredcaperred2y9e3w", Variant::Bech32),
            ("?1ezyfcl", Variant::Bech32),
            ("A1LQFN3A", Variant::Bech32m),
            ("a1lqfn3a", Variant::Bech32m),
            ("abcdef1l7aum6echk45nj3s0wdvt2fg8x9yrzpqzd3ryx", Variant::Bech32m),
            ("?1v759aa", Variant::Bech32m),
        ] {
            let (hrp, data, v) = decode(s).unwrap_or_else(|e| panic!("{s}: {e:?}"));
            assert_eq!(v, variant, "{s}: wrong variant");
            assert_eq!(encode(&hrp, &data, v), s.to_ascii_lowercase(), "{s}");
        }
    }

    /// The invalid strings matter more than the valid ones: a decoder that
    /// accepts these accepts a corrupted address, and the checksum exists
    /// precisely to stop that.
    #[test]
    fn the_bip_vectors_that_must_fail_do() {
        for (s, why) in [
            ("\u{20}1nwldj5", "HRP character out of range"),
            ("\u{7F}1axkwrx", "HRP character out of range"),
            ("pzry9x0s0muk", "no separator"),
            ("1pzry9x0s0muk", "empty HRP"),
            ("x1b4n0q5v", "invalid data character"),
            ("li1dgmt3", "too short checksum"),
            ("A1G7SGD8", "checksum calculated with uppercase HRP"),
            ("10a06t8", "empty HRP"),
            ("1qzzfhee", "empty HRP"),
            (
                "abcdef1qpzry9x8gf2tvdw0s3jn54khce6mua7lmqqqxw1",
                "trailing separator junk",
            ),
            ("in1muywd", "empty HRP after lowercasing"),
            ("M1VUXWEZ", "bech32m constant with bech32 data"),
        ] {
            assert!(decode(s).is_err(), "{s:?} was accepted ({why})");
        }
    }

    /// A wrong constant has to be caught, or a Taproot address would decode as
    /// a segwit-v0 one and funds would be sent to a script nobody can spend.
    #[test]
    fn the_two_variants_are_told_apart() {
        let data = vec![0u8, 1, 2, 3, 4];
        let a = encode("bc", &data, Variant::Bech32);
        let b = encode("bc", &data, Variant::Bech32m);
        assert_ne!(a, b);
        assert_eq!(decode(&a).unwrap().2, Variant::Bech32);
        assert_eq!(decode(&b).unwrap().2, Variant::Bech32m);
        assert_eq!(Variant::for_witness_version(0), Variant::Bech32);
        assert_eq!(Variant::for_witness_version(1), Variant::Bech32m);
    }

    /// Padding is allowed when packing bytes into 5-bit groups and forbidden
    /// when unpacking, or two different strings would decode to one address.
    #[test]
    fn bit_conversion_rejects_stray_bits() {
        let bytes = [0xffu8; 20];
        let five = convert_bits(&bytes, 8, 5, true).unwrap();
        assert_eq!(
            convert_bits(&five, 5, 8, false).as_deref(),
            Some(&bytes[..])
        );

        // One group too many: the left-over bits are non-zero.
        let mut junk = five.clone();
        junk.push(1);
        assert_eq!(convert_bits(&junk, 5, 8, false), None);
    }

    /// A single changed character has to break the checksum. This is the
    /// property the whole format exists for.
    #[test]
    fn one_wrong_character_is_caught() {
        let good = encode(
            "bc",
            &convert_bits(&[7u8; 20], 8, 5, true).unwrap(),
            Variant::Bech32,
        );
        let mut caught = 0;
        for i in 3..good.len() {
            for c in CHARSET.iter().map(|b| *b as char) {
                if good.as_bytes()[i] as char == c {
                    continue;
                }
                let mut bad: Vec<char> = good.chars().collect();
                bad[i] = c;
                let bad: String = bad.into_iter().collect();
                assert!(decode(&bad).is_err(), "{bad} was accepted");
                caught += 1;
            }
        }
        assert!(caught > 500, "the sweep did not actually run: {caught}");
    }
}
