//! Reading a cell, and looking a key up in a TON dictionary.
//!
//! Cells are written by [`CellBuilder`](crate::cell::CellBuilder) and read
//! here. The two are not symmetrical, because nothing this wallet *signs* needs
//! parsing - the reading is all of contracts describing themselves, and a
//! dictionary is how they do it.
//!
//! A TON dictionary is a binary radix tree whose edges carry compressed labels,
//! not a flat table. Finding one key means walking down from the root matching
//! label against key, and the label is stored in one of three encodings chosen
//! per edge to save bits. There is no way to read one entry without
//! implementing all three.

use std::sync::Arc;

use crate::cell::Cell;
use crate::error::TonError;

/// A cursor into a cell: a bit position and how many references have been
/// taken.
#[derive(Clone)]
pub struct Slice<'a> {
    cell: &'a Arc<Cell>,
    bit: usize,
    next_ref: usize,
}

impl<'a> Slice<'a> {
    pub fn new(cell: &'a Arc<Cell>) -> Self {
        Slice {
            cell,
            bit: 0,
            next_ref: 0,
        }
    }

    pub fn remaining(&self) -> usize {
        self.cell.bits().saturating_sub(self.bit)
    }

    pub fn load_bit(&mut self) -> Result<bool, TonError> {
        if self.bit >= self.cell.bits() {
            return Err(TonError::BadBoc("read past the end of a cell".into()));
        }
        let b = (self.cell.data()[self.bit / 8] >> (7 - (self.bit % 8))) & 1;
        self.bit += 1;
        Ok(b == 1)
    }

    /// Up to 64 bits, most significant first.
    pub fn load_uint(&mut self, n: usize) -> Result<u64, TonError> {
        if n > 64 {
            return Err(TonError::BadBoc(format!("{n} bits do not fit in a u64")));
        }
        let mut v = 0u64;
        for _ in 0..n {
            v = (v << 1) | self.load_bit()? as u64;
        }
        Ok(v)
    }

    /// The rest of this cell as bytes. Refuses a slice that does not end on a
    /// byte boundary rather than inventing the padding.
    pub fn load_rest_bytes(&mut self) -> Result<Vec<u8>, TonError> {
        let left = self.remaining();
        if left % 8 != 0 {
            return Err(TonError::BadBoc(format!(
                "{left} bits left, which is not whole bytes"
            )));
        }
        let mut out = Vec::with_capacity(left / 8);
        for _ in 0..left / 8 {
            out.push(self.load_uint(8)? as u8);
        }
        Ok(out)
    }

    /// A `VarUInteger 16`, which is how every amount on this chain is stored:
    /// four bits saying how many bytes follow, then those bytes. Zero is one
    /// nibble and nothing else, which is why an amount is never a fixed width.
    pub fn load_coins(&mut self) -> Result<u128, TonError> {
        let len = self.load_uint(4)? as usize;
        if len > 16 {
            return Err(TonError::BadBoc(format!("{len}-byte amount")));
        }
        let mut v = 0u128;
        for _ in 0..len * 8 {
            v = (v << 1) | self.load_bit()? as u128;
        }
        Ok(v)
    }

    /// A `MsgAddress`, which may legitimately be nothing.
    ///
    /// `addr_none` is a real value in these bodies - a jetton transfer with
    /// nowhere to return its change writes one - so it is `None` rather than an
    /// error. Anything that is neither none nor a standard address is a body
    /// this code does not understand, and that *is* an error.
    pub fn load_address(&mut self) -> Result<Option<crate::TonAddress>, TonError> {
        match self.load_uint(2)? {
            0b00 => Ok(None),
            0b10 => {
                if self.load_bit()? {
                    return Err(TonError::BadBoc("anycast addresses are not read".into()));
                }
                let wc = self.load_uint(8)? as u8 as i8;
                let mut hash = [0u8; 32];
                for byte in hash.iter_mut() {
                    *byte = self.load_uint(8)? as u8;
                }
                Ok(Some(crate::TonAddress::new(wc, hash)))
            }
            other => Err(TonError::BadBoc(format!("address tag {other:#04b}"))),
        }
    }

    pub fn load_ref(&mut self) -> Result<&'a Arc<Cell>, TonError> {
        let r = self
            .cell
            .refs()
            .get(self.next_ref)
            .ok_or_else(|| TonError::BadBoc("a cell has fewer refs than expected".into()))?;
        self.next_ref += 1;
        Ok(r)
    }

    fn ref_at(&self, i: usize) -> Result<&'a Arc<Cell>, TonError> {
        self.cell
            .refs()
            .get(i)
            .ok_or_else(|| TonError::BadBoc("a dictionary fork has one branch".into()))
    }
}

/// Bits needed to hold a value in `0..=m`, which is how TON stores a bounded
/// integer. A label on a 255-bit-deep edge is counted in 8 bits; one bit deeper
/// and it takes 9.
fn bits_for(m: usize) -> usize {
    let mut bits = 0;
    while (1usize << bits) <= m {
        bits += 1;
    }
    bits
}

/// One edge's label: how many key bits it accounts for, and what they are.
fn read_label(s: &mut Slice, max: usize) -> Result<Vec<bool>, TonError> {
    if !s.load_bit()? {
        // hml_short$0: a unary length, then that many bits.
        let mut len = 0usize;
        while s.load_bit()? {
            len += 1;
            if len > max {
                return Err(TonError::BadBoc("a dictionary label is too long".into()));
            }
        }
        (0..len).map(|_| s.load_bit()).collect()
    } else if !s.load_bit()? {
        // hml_long$10: a bounded length, then that many bits.
        let len = s.load_uint(bits_for(max))? as usize;
        if len > max {
            return Err(TonError::BadBoc("a dictionary label is too long".into()));
        }
        (0..len).map(|_| s.load_bit()).collect()
    } else {
        // hml_same$11: one bit, repeated. This is what makes a key of mostly
        // zeroes cheap.
        let v = s.load_bit()?;
        let len = s.load_uint(bits_for(max))? as usize;
        if len > max {
            return Err(TonError::BadBoc("a dictionary label is too long".into()));
        }
        Ok(vec![v; len])
    }
}

fn key_bit(key: &[u8], i: usize) -> bool {
    (key[i / 8] >> (7 - (i % 8))) & 1 == 1
}

/// Look one key up in a `Hashmap n X`, given the tree's root cell.
///
/// Returns the leaf's remaining slice, positioned after the label - which is
/// where the value is. `None` means the key is not in the tree, which is a
/// normal answer and not a malformed dictionary.
pub fn lookup<'a>(
    root: &'a Arc<Cell>,
    key_bits: usize,
    key: &[u8],
) -> Result<Option<Slice<'a>>, TonError> {
    if key.len() * 8 < key_bits {
        return Err(TonError::BadBoc("the key is shorter than the tree".into()));
    }
    let mut slice = Slice::new(root);
    let mut left = key_bits;
    let mut pos = 0usize;

    loop {
        let label = read_label(&mut slice, left)?;
        if label.len() > left {
            return Err(TonError::BadBoc("a dictionary label overruns a key".into()));
        }
        for (i, want) in label.iter().enumerate() {
            if key_bit(key, pos + i) != *want {
                return Ok(None);
            }
        }
        pos += label.len();
        left -= label.len();
        if left == 0 {
            return Ok(Some(slice));
        }
        // Not a leaf: one more key bit chooses a branch.
        let branch = key_bit(key, pos);
        slice = Slice::new(slice.ref_at(branch as usize)?);
        pos += 1;
        left -= 1;
    }
}

/// A `HashmapE n X` - the same tree, but allowed to be empty.
pub fn lookup_maybe_empty<'a>(
    s: &mut Slice<'a>,
    key_bits: usize,
    key: &[u8],
) -> Result<Option<Slice<'a>>, TonError> {
    if !s.load_bit()? {
        return Ok(None); // hme_empty$0
    }
    lookup(s.load_ref()?, key_bits, key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bounded_integer_takes_as_many_bits_as_it_needs() {
        assert_eq!(bits_for(0), 0);
        assert_eq!(bits_for(1), 1);
        // The two that actually occur: a 256-bit key's root, and one edge down.
        assert_eq!(bits_for(255), 8);
        assert_eq!(bits_for(256), 9, "one bit deeper needs one more bit");
    }
}
