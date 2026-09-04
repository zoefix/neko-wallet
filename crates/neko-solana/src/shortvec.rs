//! Solana's compact-u16 length prefix.
//!
//! Every array in a transaction is length-prefixed with this, so getting it
//! wrong does not produce a rejected transaction - it produces a *different*
//! transaction, because the cluster reads the following bytes as something
//! else. Encoding and decoding are both here so the round trip can be tested.

/// The largest value this encoding is allowed to carry. Solana's decoder
/// rejects anything above `u16::MAX`, and in practice a transaction cannot
/// approach it.
pub const MAX: usize = u16::MAX as usize;

/// Append `n` in compact-u16 form: seven bits per byte, high bit set on every
/// byte but the last.
pub fn encode(out: &mut Vec<u8>, n: usize) {
    let mut v = n;
    loop {
        let mut byte = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(byte);
            return;
        }
        byte |= 0x80;
        out.push(byte);
    }
}

/// Read a compact-u16, returning the value and how many bytes it took.
pub fn decode(buf: &[u8]) -> Option<(usize, usize)> {
    let mut value = 0usize;
    for (i, &byte) in buf.iter().enumerate().take(3) {
        value |= ((byte & 0x7f) as usize) << (i * 7);
        if byte & 0x80 == 0 {
            // Canonical form only: a value that could have been written in
            // fewer bytes is a different encoding of the same number, and
            // accepting both would make a transaction's bytes ambiguous.
            if i > 0 && byte == 0 {
                return None;
            }
            return Some((value, i + 1));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The boundaries are where an off-by-one in the shift shows up.
    #[test]
    fn known_encodings() {
        for (n, want) in [
            (0usize, vec![0x00]),
            (1, vec![0x01]),
            (5, vec![0x05]),
            (0x7f, vec![0x7f]),
            (0x80, vec![0x80, 0x01]),
            (0xff, vec![0xff, 0x01]),
            (0x100, vec![0x80, 0x02]),
            (0x3fff, vec![0xff, 0x7f]),
            (0x4000, vec![0x80, 0x80, 0x01]),
            (MAX, vec![0xff, 0xff, 0x03]),
        ] {
            let mut out = Vec::new();
            encode(&mut out, n);
            assert_eq!(out, want, "encoding {n}");
            assert_eq!(decode(&out), Some((n, want.len())), "decoding {n}");
        }
    }

    #[test]
    fn round_trips_across_the_whole_range() {
        for n in (0..=MAX).step_by(37) {
            let mut out = Vec::new();
            encode(&mut out, n);
            assert_eq!(decode(&out).map(|(v, _)| v), Some(n));
        }
    }

    /// A trailing zero continuation byte encodes a value that had a shorter
    /// form. Two byte strings meaning one number is how a signature ends up
    /// covering something other than what was sent.
    #[test]
    fn non_canonical_encodings_are_rejected() {
        assert_eq!(decode(&[0x80, 0x00]), None);
        assert_eq!(decode(&[0x81, 0x80, 0x00]), None);
        // Unterminated.
        assert_eq!(decode(&[0x80]), None);
        assert_eq!(decode(&[0x80, 0x80, 0x80]), None);
    }
}
