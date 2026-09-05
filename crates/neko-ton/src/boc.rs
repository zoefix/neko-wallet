//! Bags of cells: how a tree of cells becomes bytes and back.
//!
//! A cell tree is a DAG, not a list, so it cannot simply be written out in
//! order. A BoC flattens it into an indexed array with every cell placed before
//! the ones it references, plus a header saying how wide the indices and
//! offsets are. Reading one wrong does not fail - it produces a different tree,
//! which hashes to a different address.

use std::collections::HashMap;
use std::sync::Arc;

use crate::cell::Cell;
use crate::error::TonError;

const MAGIC: [u8; 4] = [0xb5, 0xee, 0x9c, 0x72];
const HAS_CRC32C: u8 = 0x40;

/// Parse a bag of cells and return its first root.
pub fn parse(bytes: &[u8]) -> Result<Arc<Cell>, TonError> {
    let bad = |m: &str| TonError::BadBoc(m.to_string());
    if bytes.len() < 6 || bytes[..4] != MAGIC {
        return Err(bad("not a bag of cells"));
    }
    let flags = bytes[4];
    let ref_size = (flags & 0b111) as usize;
    let off_size = bytes[5] as usize;
    if ref_size == 0 || ref_size > 4 || off_size == 0 || off_size > 8 {
        return Err(bad("implausible index widths"));
    }

    let mut i = 6usize;
    let read = |n: usize, i: &mut usize| -> Result<usize, TonError> {
        if *i + n > bytes.len() {
            return Err(TonError::BadBoc("truncated".into()));
        }
        let mut v = 0usize;
        for b in &bytes[*i..*i + n] {
            v = (v << 8) | *b as usize;
        }
        *i += n;
        Ok(v)
    };

    let cell_count = read(ref_size, &mut i)?;
    let root_count = read(ref_size, &mut i)?;
    let _absent = read(ref_size, &mut i)?;
    let _tot_size = read(off_size, &mut i)?;
    if root_count == 0 || cell_count == 0 {
        return Err(bad("no roots"));
    }
    let mut roots = Vec::with_capacity(root_count);
    for _ in 0..root_count {
        roots.push(read(ref_size, &mut i)?);
    }

    // Each cell: two descriptor bytes, its data, then its reference indices.
    let mut raw: Vec<(usize, Vec<u8>)> = Vec::with_capacity(cell_count);
    let mut refs: Vec<Vec<usize>> = Vec::with_capacity(cell_count);
    for _ in 0..cell_count {
        if i + 2 > bytes.len() {
            return Err(bad("truncated cell header"));
        }
        let (d1, d2) = (bytes[i], bytes[i + 1]);
        i += 2;
        if d1 & 0b1_0000 != 0 {
            // Exotic cells - pruned branches, library references, merkle
            // proofs. Nothing this wallet builds or needs to read.
            return Err(bad("exotic cell"));
        }
        let n_refs = (d1 & 0b111) as usize;
        let n_bytes = (d2 as usize).div_ceil(2);
        if i + n_bytes > bytes.len() {
            return Err(bad("truncated cell data"));
        }
        let data = bytes[i..i + n_bytes].to_vec();
        i += n_bytes;

        // An odd `d2` means the final byte carries a completion tag rather
        // than eight bits of content.
        let bits = if d2 % 2 == 0 {
            d2 as usize * 4
        } else {
            let last = *data.last().ok_or_else(|| bad("empty tagged cell"))?;
            if last == 0 {
                return Err(bad("completion tag is missing"));
            }
            (n_bytes - 1) * 8 + (7 - last.trailing_zeros() as usize)
        };

        let mut r = Vec::with_capacity(n_refs);
        for _ in 0..n_refs {
            let ix = read(ref_size, &mut i)?;
            if ix >= cell_count {
                return Err(bad("reference out of range"));
            }
            r.push(ix);
        }
        raw.push((bits, data));
        refs.push(r);
    }

    // Built bottom-up. A BoC always places a cell before the ones it points
    // at, so a reference to an earlier index would be a cycle.
    let mut built: HashMap<usize, Arc<Cell>> = HashMap::new();
    for ix in (0..cell_count).rev() {
        let mut children = Vec::with_capacity(refs[ix].len());
        for r in &refs[ix] {
            if *r <= ix {
                return Err(bad("a cell references itself or an earlier cell"));
            }
            children.push(
                built
                    .get(r)
                    .cloned()
                    .ok_or_else(|| bad("reference to a cell that was not read"))?,
            );
        }
        let (bits, data) = raw[ix].clone();
        built.insert(ix, Arc::new(Cell::from_parts(data, bits, children)?));
    }
    built
        .get(&roots[0])
        .cloned()
        .ok_or_else(|| bad("root is missing"))
}

/// Serialize one root and everything below it.
pub fn serialize(root: &Arc<Cell>) -> Result<Vec<u8>, TonError> {
    // Topological order, parents first, each cell once. Identity is the hash,
    // so a tree that uses one cell twice writes it once.
    let mut order: Vec<Arc<Cell>> = Vec::new();
    let mut index: HashMap<[u8; 32], usize> = HashMap::new();
    fn walk(
        c: &Arc<Cell>,
        order: &mut Vec<Arc<Cell>>,
        index: &mut HashMap<[u8; 32], usize>,
    ) -> usize {
        let h = c.hash();
        if let Some(ix) = index.get(&h) {
            return *ix;
        }
        let ix = order.len();
        order.push(c.clone());
        index.insert(h, ix);
        // Placed before its children, which is the ordering a reader assumes.
        for r in c.refs() {
            walk(r, order, index);
        }
        ix
    }
    walk(root, &mut order, &mut index);

    // Re-index so every reference points forward. `walk` already visits
    // parents first, so the order it produced is the order used.
    let positions: HashMap<[u8; 32], usize> = order
        .iter()
        .enumerate()
        .map(|(i, c)| (c.hash(), i))
        .collect();

    let ref_size = bytes_for(order.len());
    let mut body = Vec::new();
    for c in &order {
        let d1 = c.refs().len() as u8;
        let d2 = (c.bits() / 8) as u8 + c.bits().div_ceil(8) as u8;
        body.push(d1);
        body.push(d2);
        let mut data = c.data().to_vec();
        data.resize(c.bits().div_ceil(8), 0);
        if c.bits() % 8 != 0 {
            let spare = c.bits() % 8;
            let last = data.len() - 1;
            data[last] = (data[last] & (0xffu8 << (8 - spare))) | (1 << (7 - spare));
        }
        body.extend_from_slice(&data);
        for r in c.refs() {
            let ix = positions[&r.hash()];
            body.extend_from_slice(&ix.to_be_bytes()[8 - ref_size..]);
        }
    }

    let off_size = bytes_for(body.len());
    let mut out = Vec::with_capacity(body.len() + 32);
    out.extend_from_slice(&MAGIC);
    out.push(HAS_CRC32C | ref_size as u8);
    out.push(off_size as u8);
    out.extend_from_slice(&order.len().to_be_bytes()[8 - ref_size..]);
    out.extend_from_slice(&1usize.to_be_bytes()[8 - ref_size..]); // one root
    out.extend_from_slice(&0usize.to_be_bytes()[8 - ref_size..]); // none absent
    out.extend_from_slice(&body.len().to_be_bytes()[8 - off_size..]);
    out.extend_from_slice(&0usize.to_be_bytes()[8 - ref_size..]); // root index 0
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc32c(&out).to_le_bytes());
    Ok(out)
}

fn bytes_for(n: usize) -> usize {
    let mut w = 1;
    while n >= 1 << (8 * w) {
        w += 1;
    }
    w
}

/// CRC-32C (Castagnoli), which is what a BoC checksums with - not the CRC-32
/// everything else uses.
fn crc32c(data: &[u8]) -> u32 {
    const POLY: u32 = 0x82F6_3B78;
    let mut crc = 0xffff_ffffu32;
    for b in data {
        crc ^= *b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ POLY
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wallet v4R2's code, taken from a contract deployed on mainnet.
    const V4R2: &[u8] = include_bytes!("../vectors/wallet_v4r2.boc");

    /// The published code hash of wallet v4R2.
    ///
    /// This one constant checks the whole module at once: parsing the bag,
    /// rebuilding a twenty-cell tree, the completion tags, the reference
    /// indices, and the recursive hash with its per-reference depths. A single
    /// bit wrong anywhere and this is a different number.
    const V4R2_CODE_HASH: &str = "feb5ff6820e2ff0d9483e7e0d62c817d846789fb4ae580c878866d959dabd5c0";

    #[test]
    fn the_wallet_code_parses_to_its_known_hash() {
        let code = parse(V4R2).unwrap();
        assert_eq!(hex::encode(code.hash()), V4R2_CODE_HASH);
    }

    /// Round-tripping proves the writer agrees with the reader. It does not
    /// have to reproduce the original bytes - a BoC may legitimately be written
    /// several ways - but it has to describe the same tree.
    #[test]
    fn a_tree_survives_a_round_trip() {
        let code = parse(V4R2).unwrap();
        let again = parse(&serialize(&code).unwrap()).unwrap();
        assert_eq!(again.hash(), code.hash());
        assert_eq!(again.depth(), code.depth());
    }

    /// The checksum is CRC-32C, and a serialized bag has to carry one a reader
    /// would accept.
    #[test]
    fn the_checksum_is_castagnoli() {
        // The standard check value for CRC-32C over "123456789".
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn malformed_bags_are_refused() {
        for (bytes, why) in [
            (&b""[..], "empty"),
            (&[0u8; 16][..], "wrong magic"),
            (&V4R2[..8], "truncated"),
        ] {
            assert!(parse(bytes).is_err(), "accepted a bag that is {why}");
        }
    }
}
