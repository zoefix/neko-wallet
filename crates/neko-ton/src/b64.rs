//! Base64, because TON's API speaks it everywhere.
//!
//! Hand-written for the same reason the rest of this crate is: one fewer
//! dependency in the path between a private key and a signature. Both the
//! standard and URL-safe alphabets decode, because addresses use one and
//! message bodies the other.

const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub fn encode(input: &[u8]) -> String {
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for c in input.chunks(3) {
        let b = [c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 {
            T[(n >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if c.len() > 2 {
            T[n as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

pub fn decode(s: &str) -> Option<Vec<u8>> {
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
            // Whitespace turns up in JSON strings that were wrapped somewhere.
            b'\n' | b'\r' | b' ' | b'\t' => continue,
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

    #[test]
    fn it_matches_the_rfc() {
        for (input, want) in [
            (&b""[..], ""),
            (b"f", "Zg=="),
            (b"fo", "Zm8="),
            (b"foo", "Zm9v"),
            (b"foob", "Zm9vYg=="),
            (b"foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(input), want);
            assert_eq!(decode(want).as_deref(), Some(input));
        }
    }

    /// Addresses are URL-safe base64 and message bodies are not. Both have to
    /// decode, and to the same bytes.
    #[test]
    fn both_alphabets_decode_alike() {
        let bytes = [0xfbu8, 0xff, 0xbe];
        let standard = encode(&bytes);
        assert!(standard.contains('+') || standard.contains('/'));
        let url_safe: String = standard
            .chars()
            .map(|c| match c {
                '+' => '-',
                '/' => '_',
                other => other,
            })
            .collect();
        assert_eq!(decode(&standard), decode(&url_safe));
        assert_eq!(decode(&url_safe).unwrap(), bytes);
    }

    #[test]
    fn something_that_is_not_base64_is_not_decoded() {
        assert_eq!(decode("!!!!"), None);
    }
}
