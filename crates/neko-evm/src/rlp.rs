//! The minimum RLP needed to encode a transaction.
//!
//! Hand-written for the same reason the TRON protobuf encoder is: an
//! transaction encoder decides what gets signed, and a general-purpose library
//! brings a general-purpose parser with it. Encoding is all this needs, and
//! encoding is about sixty lines.
//!
//! Two rules carry all the risk. Integers are big-endian with **no leading
//! zeros**, and zero is the *empty* string rather than a zero byte - get that
//! wrong and the transaction hashes differently, so the signature is for
//! something the network will not accept, and the error says nothing about
//! encoding. Both are pinned by the EIP-155 vector in `tx.rs`.

/// A single byte string.
pub fn bytes(out: &mut Vec<u8>, v: &[u8]) {
    if v.len() == 1 && v[0] <= 0x7f {
        out.push(v[0]);
    } else if v.len() <= 55 {
        out.push(0x80 + v.len() as u8);
        out.extend_from_slice(v);
    } else {
        let len = be_bytes(v.len() as u128);
        out.push(0xb7 + len.len() as u8);
        out.extend_from_slice(&len);
        out.extend_from_slice(v);
    }
}

/// An unsigned integer, in RLP's canonical minimal form.
pub fn uint(out: &mut Vec<u8>, v: u128) {
    bytes(out, &be_bytes(v));
}

/// A list whose payload is already encoded.
pub fn list(out: &mut Vec<u8>, payload: &[u8]) {
    if payload.len() <= 55 {
        out.push(0xc0 + payload.len() as u8);
    } else {
        let len = be_bytes(payload.len() as u128);
        out.push(0xf7 + len.len() as u8);
        out.extend_from_slice(&len);
    }
    out.extend_from_slice(payload);
}

/// Big-endian, minimal width. Zero is empty - that is RLP's rule, not an
/// oversight, and it is the single easiest thing to get wrong here.
pub fn be_bytes(v: u128) -> Vec<u8> {
    if v == 0 {
        return Vec::new();
    }
    let full = v.to_be_bytes();
    let first = full.iter().position(|b| *b != 0).unwrap_or(full.len() - 1);
    full[first..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enc_bytes(v: &[u8]) -> Vec<u8> {
        let mut o = Vec::new();
        bytes(&mut o, v);
        o
    }
    fn enc_uint(v: u128) -> Vec<u8> {
        let mut o = Vec::new();
        uint(&mut o, v);
        o
    }

    /// Vectors from the RLP specification.
    #[test]
    fn matches_the_specification() {
        assert_eq!(enc_bytes(b"dog"), vec![0x83, b'd', b'o', b'g']);
        assert_eq!(enc_bytes(b""), vec![0x80]);
        assert_eq!(enc_bytes(b"\x00"), vec![0x00]);
        assert_eq!(enc_bytes(b"\x0f"), vec![0x0f]);
        assert_eq!(enc_bytes(b"\x04\x00"), vec![0x82, 0x04, 0x00]);

        // 55 bytes is the last length with a one-byte header; 56 is the first
        // that needs a length-of-length. Both sides of that boundary.
        let a = vec![b'a'; 55];
        assert_eq!(enc_bytes(&a)[0], 0x80 + 55);
        let b = vec![b'a'; 56];
        assert_eq!(&enc_bytes(&b)[..2], &[0xb8, 56]);

        let mut o = Vec::new();
        list(&mut o, &[]);
        assert_eq!(o, vec![0xc0]);
    }

    /// Zero is the empty string, and no integer carries a leading zero. A
    /// transaction that gets this wrong hashes differently and is rejected by
    /// the network with an error that mentions neither.
    #[test]
    fn integers_are_minimal_and_zero_is_empty() {
        assert_eq!(enc_uint(0), vec![0x80]);
        assert_eq!(enc_uint(1), vec![0x01]);
        assert_eq!(enc_uint(0x7f), vec![0x7f]);
        assert_eq!(enc_uint(0x80), vec![0x81, 0x80]);
        assert_eq!(enc_uint(1024), vec![0x82, 0x04, 0x00]);
        assert_eq!(be_bytes(0), Vec::<u8>::new());
        assert_eq!(be_bytes(0x0000_0001), vec![0x01]);
        // 20 gwei, the gas price in the EIP-155 example.
        assert_eq!(be_bytes(20_000_000_000), vec![0x04, 0xa8, 0x17, 0xc8, 0x00]);
    }

    /// A long payload crosses into the length-of-length form. Calldata for a
    /// token transfer is 68 bytes, so this is the everyday path, not an edge.
    #[test]
    fn long_payloads_use_a_length_prefix() {
        let mut o = Vec::new();
        list(&mut o, &vec![0u8; 300]);
        assert_eq!(&o[..3], &[0xf7 + 2, 0x01, 0x2c]);
        assert_eq!(o.len(), 3 + 300);
    }
}
