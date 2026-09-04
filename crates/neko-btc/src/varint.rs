//! Bitcoin's CompactSize length prefix.
//!
//! Every count and every script in a transaction is prefixed with this, so an
//! encoding that is off by a byte does not fail to parse - it shifts everything
//! after it, and the signature then covers a transaction nobody wrote.

/// Append `n` in CompactSize form.
pub fn encode(out: &mut Vec<u8>, n: u64) {
    match n {
        0..=0xfc => out.push(n as u8),
        0xfd..=0xffff => {
            out.push(0xfd);
            out.extend_from_slice(&(n as u16).to_le_bytes());
        }
        0x1_0000..=0xffff_ffff => {
            out.push(0xfe);
            out.extend_from_slice(&(n as u32).to_le_bytes());
        }
        _ => {
            out.push(0xff);
            out.extend_from_slice(&n.to_le_bytes());
        }
    }
}

/// How many bytes `n` takes. Needed to size a transaction before building it.
pub fn len(n: u64) -> usize {
    match n {
        0..=0xfc => 1,
        0xfd..=0xffff => 3,
        0x1_0000..=0xffff_ffff => 5,
        _ => 9,
    }
}

/// A length-prefixed byte string, which is how every script appears.
pub fn encode_bytes(out: &mut Vec<u8>, data: &[u8]) {
    encode(out, data.len() as u64);
    out.extend_from_slice(data);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The boundaries, where an off-by-one in the range picks the wrong width
    /// and every following byte moves.
    #[test]
    fn the_boundaries_are_right() {
        for (n, want) in [
            (0u64, vec![0x00]),
            (1, vec![0x01]),
            (0xfc, vec![0xfc]),
            (0xfd, vec![0xfd, 0xfd, 0x00]),
            (0xff, vec![0xfd, 0xff, 0x00]),
            (0xffff, vec![0xfd, 0xff, 0xff]),
            (0x1_0000, vec![0xfe, 0x00, 0x00, 0x01, 0x00]),
            (0xffff_ffff, vec![0xfe, 0xff, 0xff, 0xff, 0xff]),
            (
                0x1_0000_0000,
                vec![0xff, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00],
            ),
        ] {
            let mut out = Vec::new();
            encode(&mut out, n);
            assert_eq!(out, want, "encoding {n:#x}");
            assert_eq!(len(n), want.len(), "length of {n:#x}");
        }
    }
}
