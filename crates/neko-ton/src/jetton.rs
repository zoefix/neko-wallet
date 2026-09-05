//! Jettons: tokens, which on this chain are contracts of their own.
//!
//! Your USDT is not a row in the token's ledger keyed by your address. It is a
//! balance inside a *jetton wallet* - a separate contract, one per holder per
//! token, at an address derived from both. Three consequences, and every one of
//! them surprises somebody:
//!
//! * **Sending USDT costs GRAM.** The transfer is a message to your jetton
//!   wallet, which messages the recipient's, and each hop needs gas. That gas
//!   travels attached to the message; whatever is unused comes back.
//! * **The destination is the recipient's own address, not their jetton
//!   wallet.** The contracts work out the rest between them. Sending to
//!   somebody's jetton wallet address instead is a real address that will not
//!   credit them.
//! * **A first transfer deploys the recipient's jetton wallet**, out of the
//!   attached coin.

use std::sync::Arc;

use crate::address::TonAddress;
use crate::cell::{Cell, CellBuilder};
use crate::error::TonError;

/// `transfer`, from the jetton standard.
pub const OP_TRANSFER: u32 = 0x0f8a_7ea5;
/// `transfer_notification`, which a recipient's jetton wallet sends on. This is
/// how an incoming token transfer is recognised in a history.
pub const OP_TRANSFER_NOTIFICATION: u32 = 0x7362_d09c;
/// `internal_transfer`, the hop between two jetton wallets.
pub const OP_INTERNAL_TRANSFER: u32 = 0x178d_4519;

/// The body of a transfer, addressed to *your own* jetton wallet.
///
/// `response_destination` is where the unused part of the attached coin is
/// returned. Leaving it absent does not save anything - it burns the remainder.
pub fn transfer_body(
    amount: u128,
    to_owner: &TonAddress,
    response_to: &TonAddress,
    forward_ton_amount: u128,
) -> Result<Arc<Cell>, TonError> {
    let mut b = CellBuilder::new();
    b.store_uint(OP_TRANSFER as u64, 32)?
        .store_uint(0, 64)? // query_id
        .store_coins(amount)?;
    store_address(&mut b, to_owner)?;
    store_address(&mut b, response_to)?;
    b.store_bit(false)? // no custom payload
        .store_coins(forward_ton_amount)?
        .store_bit(false)?; // forward payload, inline and empty
    b.build_arc()
}

fn store_address(b: &mut CellBuilder, a: &TonAddress) -> Result<(), TonError> {
    b.store_uint(0b10, 2)?;
    b.store_bit(false)?;
    b.store_uint(a.workchain as u8 as u64, 8)?;
    b.store_bytes(&a.hash)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> TonAddress {
        TonAddress::parse(s).unwrap()
    }

    /// The opcode is the first thing a jetton wallet reads. Get it wrong and
    /// the contract does not recognise the message - which, for a message
    /// carrying coin, means the coin stays there.
    #[test]
    fn the_body_starts_with_the_transfer_opcode() {
        let body = transfer_body(
            4_700_000,
            &addr("EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs"),
            &addr("0:5661bcb42ba847235760ce9aaa2dfff103eb7365db06e5df053120bacb77ddfd"),
            1,
        )
        .unwrap();
        assert_eq!(
            u32::from_be_bytes(body.data()[..4].try_into().unwrap()),
            OP_TRANSFER
        );
        assert_eq!(OP_TRANSFER, 0x0f8a_7ea5);

        // op + query_id + coins(4.7e6: 4 + 3 bytes) + two addresses + the
        // three trailing flags and the forward amount.
        let amount_bits = 4 + 3 * 8;
        let fwd_bits = 4 + 8;
        assert_eq!(
            body.bits(),
            32 + 64 + amount_bits + 267 + 267 + 1 + fwd_bits + 1
        );
    }

    /// The destination is the recipient, not their jetton wallet. Their jetton
    /// wallet is a real address that will not credit them.
    #[test]
    fn the_destination_is_the_owner() {
        let owner = addr("EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs");
        let other = addr("0:5661bcb42ba847235760ce9aaa2dfff103eb7365db06e5df053120bacb77ddfd");
        let a = transfer_body(1, &owner, &other, 1).unwrap();
        let b = transfer_body(1, &other, &owner, 1).unwrap();
        assert_ne!(
            a.hash(),
            b.hash(),
            "the two addresses are not interchangeable"
        );
    }

    /// Amounts are minimally encoded here as everywhere, so the body's size
    /// tracks the number rather than being fixed.
    #[test]
    fn the_amount_is_minimally_encoded() {
        let owner = addr("EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs");
        let zero = transfer_body(0, &owner, &owner, 0).unwrap();
        let one = transfer_body(1, &owner, &owner, 0).unwrap();
        assert_eq!(one.bits() - zero.bits(), 8);
    }
}

// ── The token describing itself ────────────────────────────────────────────

/// TEP-64 metadata: a dictionary keyed by the sha256 of each attribute's name.
///
/// `sha256("decimals")`, computed once and pinned rather than hashed at
/// runtime, so the constant is visible and the test can check it.
pub const KEY_DECIMALS: [u8; 32] = [
    0xee, 0x80, 0xfd, 0x2f, 0x1e, 0x03, 0x48, 0x0e, 0x22, 0x82, 0x36, 0x35, 0x96, 0xee, 0x75, 0x2d,
    0x7b, 0xb2, 0x7f, 0x50, 0x77, 0x6b, 0x95, 0x08, 0x6a, 0x02, 0x79, 0x18, 0x96, 0x75, 0x92, 0x3e,
];

/// How TEP-64 says a metadata blob is stored.
const CONTENT_ONCHAIN: u64 = 0x00;
/// How a string is stored inside one: a byte of tag, then the bytes,
/// continuing into a reference when they do not fit.
const STRING_SNAKE: u64 = 0x00;

/// What a jetton says its precision is, out of the master's own content cell.
///
/// This is the number worth checking. A symbol that is wrong is a cosmetic
/// problem; a precision that is wrong moves a million times the intended
/// amount, and the same token name is six decimals on four of these chains and
/// eighteen on the fifth.
///
/// **The symbol is deliberately not read.** TON's USDT publishes only its
/// decimals and a URI on chain - the name and symbol live in a JSON file on the
/// issuer's web server. Fetching that would mean this wallet talking to a host
/// the user never chose, to learn a string, which is not a trade worth making.
pub fn decimals_from_content(content: &Arc<Cell>) -> Result<u8, TonError> {
    let mut s = crate::dict::Slice::new(content);
    let kind = s.load_uint(8)?;
    if kind != CONTENT_ONCHAIN {
        return Err(TonError::BadReply(
            "this jetton keeps its metadata off chain, so its decimals cannot be checked".into(),
        ));
    }
    let leaf = crate::dict::lookup_maybe_empty(&mut s, 256, &KEY_DECIMALS)?
        .ok_or_else(|| TonError::BadReply("this jetton does not state its decimals".into()))?;
    let mut leaf = leaf;
    let text = snake_string(leaf.load_ref()?)?;
    text.trim()
        .parse::<u8>()
        .map_err(|_| TonError::BadReply(format!("this jetton states its decimals as {text:?}")))
}

/// A string as TEP-64 stores one: a tag byte, then bytes that carry on into a
/// reference when a cell runs out of room.
fn snake_string(cell: &Arc<Cell>) -> Result<String, TonError> {
    let mut s = crate::dict::Slice::new(cell);
    if s.load_uint(8)? != STRING_SNAKE {
        return Err(TonError::BadReply(
            "a metadata string is not in the expected form".into(),
        ));
    }
    let mut bytes = s.load_rest_bytes()?;
    let mut cur = cell.clone();
    // Continuation cells carry no tag of their own. Bounded because a cell tree
    // that referred to itself would otherwise be read forever.
    for _ in 0..32 {
        let Some(next) = cur.refs().first().cloned() else {
            break;
        };
        let mut s = crate::dict::Slice::new(&next);
        bytes.extend(s.load_rest_bytes()?);
        cur = next;
    }
    String::from_utf8(bytes).map_err(|_| TonError::BadReply("a metadata string is not text".into()))
}

#[cfg(test)]
mod metadata_tests {
    use super::*;

    /// The exact bytes `get_jetton_data` returned for TON's USDT master.
    ///
    /// Two entries under a 256-bit key: a URI, and the decimals. Reading either
    /// one means walking a radix tree whose edges use two of the three label
    /// encodings, so this is not a test of a happy path so much as of the
    /// dictionary itself.
    const USDT_CONTENT: &str = "te6cckEBBwEAfQABAwDAAQIBIAIDAUO/+HLr21FNnJfCg7fwrlF5Ap4rYRnDlGJxnk9G7Y90E+ZABAFDv/dAfpePAaQHEUEbGst3Opa92T+oO7XKhDUBPIxLOskfQAYBAgAFAD5odHRwczovL3RldGhlci50by91c2R0LXRvbi5qc29uAAQANhfFQ3M=";

    fn content() -> Arc<Cell> {
        crate::boc::parse(&b64(USDT_CONTENT)).expect("the pinned content cell is malformed")
    }

    #[test]
    fn usdt_states_six_decimals_on_chain() {
        assert_eq!(decimals_from_content(&content()).unwrap(), 6);
        // Which is what the constant says, read from the token rather than
        // trusted from this file.
        assert_eq!(
            decimals_from_content(&content()).unwrap(),
            crate::chain_consts::USDT_DECIMALS
        );
    }

    /// The key is pinned rather than hashed at runtime, so something has to
    /// check it is the hash it claims to be.
    #[test]
    fn the_decimals_key_is_the_hash_of_the_word() {
        use sha2::{Digest, Sha256};
        let want: [u8; 32] = Sha256::digest(b"decimals").into();
        assert_eq!(KEY_DECIMALS, want);
    }

    /// A key that is not in the dictionary is a miss, not a parse failure -
    /// and the walk has to reach that answer rather than matching something
    /// else on the way.
    #[test]
    fn a_key_that_is_not_there_is_a_miss() {
        use sha2::{Digest, Sha256};
        let symbol: [u8; 32] = Sha256::digest(b"symbol").into();
        let c = content();
        let mut s = crate::dict::Slice::new(&c);
        assert_eq!(s.load_uint(8).unwrap(), CONTENT_ONCHAIN);
        assert!(
            crate::dict::lookup_maybe_empty(&mut s, 256, &symbol)
                .unwrap()
                .is_none(),
            "TON's USDT does not publish its symbol on chain"
        );

        // And the URI, which is there, is found - so the miss above is the
        // absence of the key and not a walk that fails on everything.
        let uri: [u8; 32] = Sha256::digest(b"uri").into();
        let mut s = crate::dict::Slice::new(&c);
        s.load_uint(8).unwrap();
        let mut leaf = crate::dict::lookup_maybe_empty(&mut s, 256, &uri)
            .unwrap()
            .expect("the URI is in the dictionary");
        assert_eq!(
            snake_string(leaf.load_ref().unwrap()).unwrap(),
            "https://tether.to/usdt-ton.json"
        );
    }

    fn b64(s: &str) -> Vec<u8> {
        let mut out = Vec::new();
        let (mut acc, mut bits) = (0u32, 0u32);
        for ch in s.bytes() {
            let v = match ch {
                b'A'..=b'Z' => ch - b'A',
                b'a'..=b'z' => ch - b'a' + 26,
                b'0'..=b'9' => ch - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                b'=' => break,
                _ => panic!("not base64"),
            } as u32;
            acc = (acc << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((acc >> bits) as u8);
            }
        }
        out
    }
}
