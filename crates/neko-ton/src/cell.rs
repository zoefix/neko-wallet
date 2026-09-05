//! Cells, the only data structure TON has.
//!
//! Everything on this chain - a contract's code, its storage, a message, an
//! address - is a *cell*: up to 1023 bits of data and up to four references to
//! other cells. Nothing is a flat byte string, and there is no length field
//! anywhere; a value's width is whatever the schema says it is, and reading one
//! bit too few shifts every field after it.
//!
//! Two consequences shape this module.
//!
//! **A cell's identity is a hash over its shape.** Not over its bytes - over
//! the descriptors, the padded data, the *depth* of each reference, and then
//! each reference's own hash. That recursion is what makes an address on this
//! chain: a wallet's address is the hash of its initial code and storage, so
//! deriving one wrong produces a valid address holding nothing.
//!
//! **The last byte is padded, not zero-filled.** A cell of 5 bits stores them
//! in the high bits of one byte, then a single `1` to mark where the data
//! stopped. Get that wrong and two different cells hash the same.

use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::error::TonError;

/// A cell's data ceiling. Fixed by the protocol.
pub const MAX_BITS: usize = 1023;
/// And its reference ceiling.
pub const MAX_REFS: usize = 4;

#[derive(Clone, PartialEq, Eq)]
pub struct Cell {
    /// `bits` bits, most significant first. Bits past `bits` are zero.
    data: Vec<u8>,
    bits: usize,
    refs: Vec<Arc<Cell>>,
}

impl Cell {
    pub fn bits(&self) -> usize {
        self.bits
    }
    pub fn data(&self) -> &[u8] {
        &self.data
    }
    pub fn refs(&self) -> &[Arc<Cell>] {
        &self.refs
    }

    pub fn from_parts(data: Vec<u8>, bits: usize, refs: Vec<Arc<Cell>>) -> Result<Self, TonError> {
        if bits > MAX_BITS {
            return Err(TonError::CellOverflow(bits));
        }
        if refs.len() > MAX_REFS {
            return Err(TonError::TooManyRefs);
        }
        Ok(Cell { data, bits, refs })
    }

    /// The two descriptor bytes that begin a cell's representation.
    ///
    /// `d1` counts references (plus flags for exotic cells and levels, neither
    /// of which this wallet builds). `d2` encodes the bit length in a way that
    /// is deliberately *not* a byte count: `bits/8 + ceil(bits/8)`, so an odd
    /// value says the last byte is partly padding.
    fn descriptors(&self) -> [u8; 2] {
        let d1 = self.refs.len() as u8;
        let d2 = (self.bits / 8) as u8 + self.bits.div_ceil(8) as u8;
        [d1, d2]
    }

    /// The data with the completion tag applied.
    ///
    /// A cell that does not end on a byte boundary marks the end of its data
    /// with a single `1` bit followed by zeros. Without it, five bits of zero
    /// and six bits of zero would be the same bytes and hash identically.
    fn padded(&self) -> Vec<u8> {
        let mut out = self.data.clone();
        out.resize(self.bits.div_ceil(8), 0);
        let spare = self.bits % 8;
        if spare != 0 {
            let last = out.len() - 1;
            let keep = 0xffu8 << (8 - spare);
            out[last] = (out[last] & keep) | (1 << (7 - spare));
        }
        out
    }

    /// How deep the tree below this cell goes. Part of the hash, so a cell
    /// cannot be swapped for one with the same content and a different shape.
    pub fn depth(&self) -> u16 {
        self.refs.iter().map(|r| r.depth() + 1).max().unwrap_or(0)
    }

    /// This cell's identity.
    pub fn hash(&self) -> [u8; 32] {
        let mut h = Sha256::new();
        h.update(self.descriptors());
        h.update(self.padded());
        for r in &self.refs {
            h.update(r.depth().to_be_bytes());
        }
        for r in &self.refs {
            h.update(r.hash());
        }
        let mut out = [0u8; 32];
        out.copy_from_slice(&h.finalize());
        out
    }
}

impl std::fmt::Debug for Cell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Cell({} bits, {} refs, {})",
            self.bits,
            self.refs.len(),
            hex::encode(&self.hash()[..8])
        )
    }
}

/// Writes a cell, one field at a time.
///
/// Every method is bit-precise, because the schema is. There is no alignment
/// and no padding between fields: a 267-bit address sits directly against the
/// 4-bit length prefix of the value that follows it.
#[derive(Default, Clone)]
pub struct CellBuilder {
    data: Vec<u8>,
    bits: usize,
    refs: Vec<Arc<Cell>>,
}

impl CellBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn bits(&self) -> usize {
        self.bits
    }

    pub fn store_bit(&mut self, one: bool) -> Result<&mut Self, TonError> {
        if self.bits >= MAX_BITS {
            return Err(TonError::CellOverflow(self.bits + 1));
        }
        if self.bits % 8 == 0 {
            self.data.push(0);
        }
        if one {
            let byte = self.bits / 8;
            self.data[byte] |= 1 << (7 - (self.bits % 8));
        }
        self.bits += 1;
        Ok(self)
    }

    /// `n` bits of `v`, most significant first.
    pub fn store_uint(&mut self, v: u64, n: usize) -> Result<&mut Self, TonError> {
        if n > 64 {
            return Err(TonError::CellOverflow(n));
        }
        for i in (0..n).rev() {
            self.store_bit((v >> i) & 1 == 1)?;
        }
        Ok(self)
    }

    pub fn store_u128(&mut self, v: u128, n: usize) -> Result<&mut Self, TonError> {
        if n > 128 {
            return Err(TonError::CellOverflow(n));
        }
        for i in (0..n).rev() {
            self.store_bit((v >> i) & 1 == 1)?;
        }
        Ok(self)
    }

    pub fn store_bytes(&mut self, b: &[u8]) -> Result<&mut Self, TonError> {
        for byte in b {
            self.store_uint(*byte as u64, 8)?;
        }
        Ok(self)
    }

    /// `VarUInteger 16`, which is what an amount of the native coin is.
    ///
    /// Four bits saying how many bytes follow, then that many bytes. Zero is a
    /// single zero nibble and no bytes at all - not one zero byte, which would
    /// be a different encoding of the same number and hash differently.
    pub fn store_coins(&mut self, v: u128) -> Result<&mut Self, TonError> {
        if v == 0 {
            return self.store_uint(0, 4);
        }
        let bytes = (128 - v.leading_zeros()).div_ceil(8) as usize;
        self.store_uint(bytes as u64, 4)?;
        for i in (0..bytes).rev() {
            self.store_uint(((v >> (i * 8)) & 0xff) as u64, 8)?;
        }
        Ok(self)
    }

    pub fn store_ref(&mut self, c: Arc<Cell>) -> Result<&mut Self, TonError> {
        if self.refs.len() >= MAX_REFS {
            return Err(TonError::TooManyRefs);
        }
        self.refs.push(c);
        Ok(self)
    }

    /// Append another builder's bits, which is how a body is assembled before
    /// it is signed and then rebuilt with the signature in front.
    pub fn store_builder(&mut self, other: &CellBuilder) -> Result<&mut Self, TonError> {
        for i in 0..other.bits {
            let byte = other.data[i / 8];
            self.store_bit((byte >> (7 - (i % 8))) & 1 == 1)?;
        }
        for r in &other.refs {
            self.store_ref(r.clone())?;
        }
        Ok(self)
    }

    pub fn build(self) -> Result<Cell, TonError> {
        Cell::from_parts(self.data, self.bits, self.refs)
    }

    pub fn build_arc(self) -> Result<Arc<Cell>, TonError> {
        Ok(Arc::new(self.build()?))
    }
}
