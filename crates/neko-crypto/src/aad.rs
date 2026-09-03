//! Associated data that binds a ciphertext to its exact location in the database.
//!
//! Without this, an attacker with database *write* access can copy one row's
//! ciphertext over another's — e.g. overwrite the cold wallet's encrypted key
//! with one they control — without ever decrypting anything.
//!
//! Byte-compatible with the Go and TypeScript implementations; verified against
//! `vectors/crypto.json`.

/// Length-prefixed, canonical AAD encoding.
///
/// Every variable-length field carries a 4-byte big-endian length prefix.
/// Without it, `{table:"ab", column:"c"}` and `{table:"a", column:"bc"}` encode
/// to identical bytes and ciphertext can be swapped between those two cells.
#[derive(Debug, Clone, Copy)]
pub struct Aad<'a> {
    pub table: &'a str,
    pub column: &'a str,
    pub row_id: i64,
    pub key_ver: u32,
    pub extra: &'a [u8],
}

impl<'a> Aad<'a> {
    pub fn new(table: &'a str, column: &'a str, row_id: i64, key_ver: u32) -> Self {
        Self {
            table,
            column,
            row_id,
            key_ver,
            extra: &[],
        }
    }

    pub fn with_extra(mut self, extra: &'a [u8]) -> Self {
        self.extra = extra;
        self
    }

    pub fn encode(&self) -> Vec<u8> {
        fn put(out: &mut Vec<u8>, s: &[u8]) {
            out.extend_from_slice(&(s.len() as u32).to_be_bytes());
            out.extend_from_slice(s);
        }
        let mut out =
            Vec::with_capacity(32 + self.table.len() + self.column.len() + self.extra.len());
        put(&mut out, self.table.as_bytes());
        put(&mut out, self.column.as_bytes());
        // i64 -> u64 reinterpretation, matching Go's uint64(a.RowID).
        out.extend_from_slice(&(self.row_id as u64).to_be_bytes());
        out.extend_from_slice(&self.key_ver.to_be_bytes());
        put(&mut out, self.extra);
        out
    }
}
