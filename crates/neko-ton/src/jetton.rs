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
