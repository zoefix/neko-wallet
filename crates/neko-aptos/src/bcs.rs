//! Just enough BCS to build a transaction.
//!
//! BCS is Move's serialisation format, and it is not self-describing: the
//! bytes carry no types, so the reader has to know the shape in advance. That
//! makes writing it simple and getting it wrong silent - a transaction with a
//! misplaced length byte is not rejected as malformed, it is *a different
//! transaction*, and the signature over it is perfectly valid.
//!
//! Two rules cover everything used here. Integers are little-endian and fixed
//! width. Anything with a length - a string, a vector, an enum's variant index
//! - is prefixed with ULEB128.

/// A growable buffer that only knows how to append.
#[derive(Default)]
pub struct Writer(Vec<u8>);

impl Writer {
    pub fn new() -> Self {
        Writer(Vec::new())
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.0.push(v);
        self
    }

    pub fn u64(&mut self, v: u64) -> &mut Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }

    /// Raw bytes with no length in front. For fixed-width things - an address
    /// is always 32 bytes and BCS writes no length for it.
    pub fn fixed(&mut self, b: &[u8]) -> &mut Self {
        self.0.extend_from_slice(b);
        self
    }

    /// ULEB128, which is how BCS writes every length and every enum variant.
    pub fn uleb(&mut self, mut v: u64) -> &mut Self {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.0.push(byte);
                return self;
            }
            self.0.push(byte | 0x80);
        }
    }

    /// A length-prefixed byte string. `vector<u8>` and `String` share this
    /// encoding, which is why a `String` here is just its UTF-8 bytes.
    pub fn bytes(&mut self, b: &[u8]) -> &mut Self {
        self.uleb(b.len() as u64);
        self.0.extend_from_slice(b);
        self
    }

    pub fn str(&mut self, s: &str) -> &mut Self {
        self.bytes(s.as_bytes())
    }

    /// An enum variant index. The same encoding as a length, and given its own
    /// name because confusing the two is the mistake this format invites.
    pub fn variant(&mut self, i: u64) -> &mut Self {
        self.uleb(i)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ULEB128 against the values where the encoding changes width.
    #[test]
    fn uleb_matches_the_reference_encoding() {
        for (v, want) in [
            (0u64, vec![0x00]),
            (1, vec![0x01]),
            (127, vec![0x7f]),
            (128, vec![0x80, 0x01]),
            (255, vec![0xff, 0x01]),
            (16_383, vec![0xff, 0x7f]),
            (16_384, vec![0x80, 0x80, 0x01]),
        ] {
            let mut w = Writer::new();
            w.uleb(v);
            assert_eq!(w.into_bytes(), want, "uleb128 of {v}");
        }
    }

    /// Little-endian, and the byte order is the whole point: a u64 written
    /// big-endian is a different, valid number.
    #[test]
    fn integers_are_little_endian() {
        let mut w = Writer::new();
        w.u64(1);
        assert_eq!(w.as_slice(), &[1, 0, 0, 0, 0, 0, 0, 0]);
        let mut w = Writer::new();
        w.u64(0x0102_0304_0506_0708);
        assert_eq!(w.as_slice(), &[8, 7, 6, 5, 4, 3, 2, 1]);
    }

    /// A string is its length then its bytes, with no terminator and no
    /// padding.
    #[test]
    fn strings_carry_their_length() {
        let mut w = Writer::new();
        w.str("transfer");
        assert_eq!(w.as_slice(), b"\x08transfer");
    }
}
