//! Messages: the only way anything happens on this chain.
//!
//! A transfer is not a transaction here. It is an *external message* asking
//! your wallet contract to act, carrying inside it the *internal message* the
//! wallet should then send. Only the inner one moves money; the outer one is
//! the instruction, and it is what gets signed.
//!
//! Three things follow, and each is a way to lose a transfer:
//!
//! * **The signature covers a hash, not the bytes.** What is signed is the hash
//!   of a cell holding the subwallet id, an expiry, the sequence number and the
//!   messages. Assembling that cell differently signs a different instruction.
//! * **`seqno` replaces a nonce, and it is read from the contract.** Sending
//!   twice with the same one is not a double spend - the second is simply
//!   ignored, which looks exactly like a transfer that vanished.
//! * **The message expires.** `valid_until` is part of what is signed, so a
//!   message that sat too long cannot be replayed later. That is a feature and
//!   it means the clock matters.

use std::sync::Arc;

use zeroize::Zeroizing;

use crate::address::TonAddress;
use crate::cell::{Cell, CellBuilder};
use crate::error::TonError;

/// Pay the forwarding fees out of the message's own value, and ignore errors
/// rather than bouncing. What an ordinary transfer uses.
pub const MODE_PAY_FEES_SEPARATELY: u8 = 1;
pub const MODE_IGNORE_ERRORS: u8 = 2;
pub const MODE_ORDINARY: u8 = MODE_PAY_FEES_SEPARATELY | MODE_IGNORE_ERRORS;

/// Send the entire remaining balance of the account.
///
/// This is how "send everything" is done here, and it is better than the
/// arithmetic every other chain needs: the contract works out the amount after
/// fees itself, at execution time, so there is no figure to get a few units
/// wrong.
pub const MODE_CARRY_ALL_BALANCE: u8 = 128;

/// How long a signed message stays valid. Long enough to survive a password
/// prompt and a slow network, short enough that an intercepted message cannot
/// be held and replayed.
pub const VALID_FOR_SECS: u32 = 120;

/// `op` for a simple transfer in wallet v4R2. Non-zero values address the
/// plugin machinery, which this wallet does not use.
const OP_SIMPLE_SEND: u8 = 0;

/// Store a destination as `MsgAddressInt`.
///
/// 267 bits: the tag, an absent anycast, a signed workchain and the 256-bit
/// account. Writing 256 and forgetting the eleven in front shifts every field
/// after it.
fn store_address(b: &mut CellBuilder, a: &TonAddress) -> Result<(), TonError> {
    b.store_uint(0b10, 2)?; // addr_std
    b.store_bit(false)?; // no anycast
    b.store_uint(a.workchain as u8 as u64, 8)?;
    b.store_bytes(&a.hash)?;
    Ok(())
}

/// An internal message: the part that actually moves value.
///
/// `bounce` decides what happens if the destination cannot accept it. To a
/// contract, bounceable means a failure returns the coins; to an ordinary
/// wallet that has never been deployed, bounceable means they come *back*
/// rather than arriving - which is why paying a fresh wallet uses the
/// non-bounceable form.
pub fn internal_message(
    to: &TonAddress,
    value: u128,
    bounce: bool,
    body: Option<Arc<Cell>>,
) -> Result<Arc<Cell>, TonError> {
    let mut b = CellBuilder::new();
    b.store_bit(false)? // int_msg_info
        .store_bit(true)? // ihr_disabled
        .store_bit(bounce)?
        .store_bit(false)?; // not itself a bounce
                            // Source is left absent: the wallet contract fills in its own address, and
                            // anything written here would be overwritten or rejected.
    b.store_uint(0, 2)?;
    store_address(&mut b, to)?;
    b.store_coins(value)?
        .store_bit(false)? // no extra currencies
        .store_coins(0)? // ihr_fee
        .store_coins(0)? // fwd_fee, filled in by the network
        .store_uint(0, 64)? // created_lt
        .store_uint(0, 32)?; // created_at
    b.store_bit(false)?; // no StateInit on the inner message
    match body {
        // A body large enough to need its own cell, referenced.
        Some(c) => {
            b.store_bit(true)?;
            b.store_ref(c)?;
        }
        None => {
            b.store_bit(false)?;
        }
    }
    b.build_arc()
}

/// A text comment, which is what an exchange's memo field becomes.
///
/// The four zero bytes in front are the opcode that says "what follows is
/// text". Without them a deposit arrives with no memo attached and, at an
/// exchange, is credited to nobody.
pub fn comment(text: &str) -> Result<Arc<Cell>, TonError> {
    let mut b = CellBuilder::new();
    b.store_uint(0, 32)?.store_bytes(text.as_bytes())?;
    b.build_arc()
}

/// The part of a wallet v4R2 message that the signature covers.
pub fn signing_body(
    subwallet_id: u32,
    valid_until: u32,
    seqno: u32,
    mode: u8,
    message: Arc<Cell>,
) -> Result<Arc<Cell>, TonError> {
    let mut b = CellBuilder::new();
    b.store_uint(subwallet_id as u64, 32)?
        .store_uint(valid_until as u64, 32)?
        .store_uint(seqno as u64, 32)?
        .store_uint(OP_SIMPLE_SEND as u64, 8)?
        .store_uint(mode as u64, 8)?
        .store_ref(message)?;
    b.build_arc()
}

/// Sign that body and put the signature in front of it.
///
/// The signature is over the body cell's *hash*, which is what makes the whole
/// tree below it - including the internal message, its destination and its
/// value - part of what was signed.
pub fn signed_body(body: Arc<Cell>, key: &Zeroizing<[u8; 32]>) -> Result<Arc<Cell>, TonError> {
    let signature = neko_hd::ton::sign(key, &body.hash());
    let mut b = CellBuilder::new();
    b.store_bytes(&signature)?;
    // The signed content is appended inline, not referenced: the signature and
    // the fields it covers are one cell.
    append(&mut b, &body)?;
    b.build_arc()
}

/// Copy a cell's bits and references into a builder.
fn append(b: &mut CellBuilder, c: &Cell) -> Result<(), TonError> {
    for i in 0..c.bits() {
        let byte = c.data()[i / 8];
        b.store_bit((byte >> (7 - (i % 8))) & 1 == 1)?;
    }
    for r in c.refs() {
        b.store_ref(r.clone())?;
    }
    Ok(())
}

/// The external message: what gets broadcast.
///
/// `state_init` is present only for a wallet that has never sent anything. The
/// address exists and can hold coins before the contract does, so the first
/// outgoing transfer has to carry the code and deploy it on the way - and
/// including it once the wallet exists is rejected.
pub fn external_message(
    wallet: &TonAddress,
    state_init: Option<Arc<Cell>>,
    body: Arc<Cell>,
) -> Result<Arc<Cell>, TonError> {
    let mut b = CellBuilder::new();
    b.store_uint(0b10, 2)?; // ext_in_msg_info
    b.store_uint(0, 2)?; // src: absent
    store_address(&mut b, wallet)?;
    b.store_coins(0)?; // import_fee
    match state_init {
        Some(init) => {
            b.store_bit(true)?; // init present
            b.store_bit(true)?; // ...as a reference
            b.store_ref(init)?;
        }
        None => {
            b.store_bit(false)?;
        }
    }
    b.store_bit(true)?; // body as a reference
    b.store_ref(body)?;
    b.build_arc()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr() -> TonAddress {
        TonAddress::parse("EQCxE6mUtQJKFnGfaROTKOt1lZbDiiX1kCixRv7Nw2Id_sDs").unwrap()
    }

    /// The header is a fixed 4 + 2 + 267 bits before the value, and the value
    /// is a variable-length integer. Any drift and the destination is read out
    /// of the middle of something else.
    #[test]
    fn an_internal_message_has_the_shape_the_schema_says() {
        let m = internal_message(&addr(), 1_000_000_000, true, None).unwrap();
        // 4 flags + 2 src + 267 dest + coins(1 TON: 4 + 4 bytes) + 1 + 4 + 4
        // + 64 + 32 + 1 + 1
        let coins = 4 + 4 * 8;
        assert_eq!(m.bits(), 4 + 2 + 267 + coins + 1 + 4 + 4 + 64 + 32 + 1 + 1);
        assert!(m.refs().is_empty());

        // The destination is where the schema puts it: bit 6 onwards.
        let bits: Vec<u8> = (0..m.bits())
            .map(|i| (m.data()[i / 8] >> (7 - (i % 8))) & 1)
            .collect();
        assert_eq!(&bits[0..4], &[0, 1, 1, 0], "int_msg_info, ihr off, bounce");
        assert_eq!(&bits[4..6], &[0, 0], "source is absent");
        assert_eq!(&bits[6..8], &[1, 0], "addr_std");
        assert_eq!(bits[8], 0, "no anycast");
    }

    /// Zero is a single empty nibble, not a zero byte. Two encodings of the
    /// same number would hash differently, and the hash is what is signed.
    #[test]
    fn coins_are_minimally_encoded() {
        let zero = internal_message(&addr(), 0, true, None).unwrap();
        let one = internal_message(&addr(), 1, true, None).unwrap();
        assert_eq!(one.bits() - zero.bits(), 8, "one byte of value");
        let big = internal_message(&addr(), u64::MAX as u128, true, None).unwrap();
        assert_eq!(big.bits() - zero.bits(), 64);
    }

    /// A comment is an opcode and then text. Exchanges read the memo out of
    /// this, and a deposit without one is credited to nobody.
    #[test]
    fn a_comment_carries_its_opcode() {
        let c = comment("memo-12345").unwrap();
        assert_eq!(c.bits(), 32 + 10 * 8);
        assert_eq!(&c.data()[..4], &[0, 0, 0, 0]);
        assert_eq!(&c.data()[4..], b"memo-12345");
    }

    /// The signature is over the body's hash, so the destination and the
    /// amount are inside what was signed. Changing either has to invalidate it.
    #[test]
    fn the_signature_covers_the_whole_instruction() {
        use ed25519_dalek::Verifier;
        let sk = Zeroizing::new([7u8; 32]);
        let pk = neko_hd::ton::public_key(&sk);

        let msg = internal_message(&addr(), 1_000_000_000, true, None).unwrap();
        let body = signing_body(698_983_191, 1_800_000_000, 5, MODE_ORDINARY, msg).unwrap();
        let signed = signed_body(body.clone(), &sk).unwrap();

        assert_eq!(signed.bits(), 512 + body.bits());
        assert_eq!(signed.refs().len(), 1, "the message came along");

        let sig: [u8; 64] = signed.data()[..64].try_into().unwrap();
        ed25519_dalek::VerifyingKey::from_bytes(&pk)
            .unwrap()
            .verify(&body.hash(), &ed25519_dalek::Signature::from_bytes(&sig))
            .expect("the signature does not cover the body");

        // A different amount is a different hash, so the same signature does
        // not carry over.
        let other = internal_message(&addr(), 2_000_000_000, true, None).unwrap();
        let other_body = signing_body(698_983_191, 1_800_000_000, 5, MODE_ORDINARY, other).unwrap();
        assert_ne!(body.hash(), other_body.hash());
        assert!(ed25519_dalek::VerifyingKey::from_bytes(&pk)
            .unwrap()
            .verify(
                &other_body.hash(),
                &ed25519_dalek::Signature::from_bytes(&sig)
            )
            .is_err());
    }

    /// Every field of the instruction is inside the hash - including the two
    /// that stop a message being replayed.
    #[test]
    fn seqno_and_expiry_are_signed_over() {
        let msg = || internal_message(&addr(), 1, true, None).unwrap();
        let base = signing_body(698_983_191, 1_800_000_000, 5, MODE_ORDINARY, msg()).unwrap();
        for other in [
            signing_body(698_983_191, 1_800_000_000, 6, MODE_ORDINARY, msg()).unwrap(),
            signing_body(698_983_191, 1_800_000_001, 5, MODE_ORDINARY, msg()).unwrap(),
            signing_body(0, 1_800_000_000, 5, MODE_ORDINARY, msg()).unwrap(),
            signing_body(698_983_191, 1_800_000_000, 5, MODE_CARRY_ALL_BALANCE, msg()).unwrap(),
        ] {
            assert_ne!(base.hash(), other.hash());
        }
    }

    /// The deploying form carries the contract; the ordinary one must not.
    #[test]
    fn only_the_first_message_carries_the_code() {
        let sk = Zeroizing::new([9u8; 32]);
        let pk = neko_hd::ton::public_key(&sk);
        let wallet = crate::wallet::address_for(&pk).unwrap();
        let init = crate::wallet::state_init(
            crate::wallet::code().unwrap(),
            crate::wallet::initial_data(&pk, crate::wallet::DEFAULT_SUBWALLET_ID).unwrap(),
        )
        .unwrap();
        let body = signed_body(
            signing_body(
                crate::wallet::DEFAULT_SUBWALLET_ID,
                1_800_000_000,
                0,
                MODE_ORDINARY,
                internal_message(&addr(), 1, true, None).unwrap(),
            )
            .unwrap(),
            &sk,
        )
        .unwrap();

        let deploying = external_message(&wallet, Some(init), body.clone()).unwrap();
        let plain = external_message(&wallet, None, body).unwrap();
        assert_eq!(deploying.refs().len(), 2, "code and body");
        assert_eq!(plain.refs().len(), 1, "body only");
        // An absent StateInit is one bit of `Maybe`; a present one is that
        // bit plus the `Either` saying it is a reference.
        assert_eq!(plain.bits() + 1, deploying.bits());

        // And the deploying form has to serialize to something a node will
        // read back as the same tree.
        let raw = crate::boc::serialize(&deploying).unwrap();
        assert_eq!(crate::boc::parse(&raw).unwrap().hash(), deploying.hash());
    }
}
