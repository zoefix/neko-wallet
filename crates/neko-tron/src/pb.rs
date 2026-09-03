//! A minimal protobuf writer: only what TRON transactions need.
//!
//! Hand-rolled rather than generated. `protoc` is not installed, the full TRON
//! `.proto` is a large dependency for seven messages, and — most importantly —
//! proto3's default-value omission has to be exactly right. Writing it out
//! explicitly makes that rule visible and testable instead of implicit.
//!
//! Encoding must match Go's `google.golang.org/protobuf` byte for byte. One
//! byte off changes the txid, the signature no longer matches, and the network
//! rejects the transaction with an error that says nothing about encoding.

#[derive(Default)]
pub struct Writer(Vec<u8>);

const WIRE_VARINT: u8 = 0;
const WIRE_BYTES: u8 = 2;

impl Writer {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    fn varint(&mut self, mut n: u64) {
        loop {
            let b = (n & 0x7f) as u8;
            n >>= 7;
            if n > 0 {
                self.0.push(b | 0x80);
            } else {
                self.0.push(b);
                break;
            }
        }
    }

    fn tag(&mut self, field: u32, wire: u8) {
        self.varint(((field as u64) << 3) | wire as u64);
    }

    /// A varint field. **Zero is skipped entirely** — proto3 does not encode
    /// default values. This is a hard requirement, not an optimisation.
    pub fn uint64(&mut self, field: u32, v: u64) -> &mut Self {
        if v != 0 {
            self.tag(field, WIRE_VARINT);
            self.varint(v);
        }
        self
    }

    /// A length-delimited field. Empty is skipped, same rule.
    pub fn bytes(&mut self, field: u32, v: &[u8]) -> &mut Self {
        if !v.is_empty() {
            self.tag(field, WIRE_BYTES);
            self.varint(v.len() as u64);
            self.0.extend_from_slice(v);
        }
        self
    }

    pub fn string(&mut self, field: u32, v: &str) -> &mut Self {
        self.bytes(field, v.as_bytes())
    }

    pub fn message(&mut self, field: u32, w: &Writer) -> &mut Self {
        self.bytes(field, w.as_slice())
    }

    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    pub fn finish(self) -> Vec<u8> {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_matches_the_spec() {
        let mut w = Writer::new();
        w.uint64(1, 1);
        assert_eq!(w.as_slice(), &[0x08, 0x01]);

        let mut w = Writer::new();
        w.uint64(1, 300);
        assert_eq!(w.as_slice(), &[0x08, 0xac, 0x02]);

        let mut w = Writer::new();
        w.uint64(2, u64::MAX);
        assert_eq!(w.as_slice()[0], 0x10);
        assert_eq!(w.as_slice().len(), 11);
    }

    /// The rule the whole encoding depends on.
    #[test]
    fn proto3_defaults_are_omitted() {
        let mut w = Writer::new();
        w.uint64(1, 0).bytes(2, &[]).string(3, "");
        assert!(w.as_slice().is_empty(), "a default value was encoded");
    }

    #[test]
    fn field_numbers_above_15_use_two_tag_bytes() {
        let mut w = Writer::new();
        w.uint64(18, 1);
        assert_eq!(w.as_slice(), &[0x90, 0x01, 0x01]);
    }
}
