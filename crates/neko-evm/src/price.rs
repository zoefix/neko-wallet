//! What one BNB is worth, asked of the chain itself.
//!
//! Quoted from PancakeSwap rather than a price service, so the wallet still
//! talks to the node you point it at and to nothing else. A portfolio figure
//! is a convenience; the property it would cost is not.
//!
//! Two things follow from that choice, and the interface states both rather
//! than hiding them:
//!
//! * The unit is **USDT**, not dollars. They track each other closely and are
//!   not the same thing.
//! * It is a spot quote from one pool, not a market-wide average, so it will
//!   differ from an exchange's headline number by a fraction of a percent.

use neko_hd::EvmAddress;

use crate::error::EvmError;

/// PancakeSwap V2 router. Used only to *quote* - this wallet never trades.
pub const PANCAKE_ROUTER: &str = "0x10ED43C718714eb63d5aA57B78B54704E256024E";
/// Wrapped BNB, the pair's other side.
pub const WBNB: &str = "0xbb4CdB9CBd36B01bD1cBaEBF2De08d9173bc095c";

/// `keccak256("getAmountsOut(uint256,address[])")[..4]`
pub const SEL_GET_AMOUNTS_OUT: [u8; 4] = [0xd0, 0x6c, 0xa6, 0x1f];

/// Calldata asking what `amount_in` of `from` buys in `to`.
pub fn amounts_out_call(amount_in: u128, from: EvmAddress, to: EvmAddress) -> Vec<u8> {
    let mut d = SEL_GET_AMOUNTS_OUT.to_vec();
    d.extend_from_slice(&word_u(amount_in));
    // A dynamic array: the offset to it, then its length, then its elements.
    d.extend_from_slice(&word_u(0x40));
    d.extend_from_slice(&word_u(2));
    d.extend_from_slice(&word_addr(from));
    d.extend_from_slice(&word_addr(to));
    d
}

/// Read the last element of the returned `uint256[]`.
pub fn read_last_amount(out: &[u8]) -> Result<u128, EvmError> {
    // offset, length, amounts[0], amounts[1]
    if out.len() < 128 {
        return Err(EvmError::BadReply(format!(
            "getAmountsOut returned {} bytes, expected at least 128",
            out.len()
        )));
    }
    crate::abi::read_u256(&out[96..128])
}

fn word_u(v: u128) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&v.to_be_bytes());
    w
}

fn word_addr(a: EvmAddress) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a.as_bytes());
    w
}

impl crate::client::Rpc {
    /// One BNB in USDT, in USDT's minimal units.
    pub async fn bnb_price_in_usdt(&self) -> Result<u128, EvmError> {
        let router = EvmAddress::parse(PANCAKE_ROUTER)?;
        let wbnb = EvmAddress::parse(WBNB)?;
        let data = amounts_out_call(
            10u128.pow(crate::BNB_DECIMALS as u32),
            wbnb,
            crate::usdt_address(),
        );
        read_last_amount(&self.eth_call(router, &data).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_selector_matches_its_signature() {
        assert_eq!(
            &crate::tx::keccak(b"getAmountsOut(uint256,address[])")[..4],
            &SEL_GET_AMOUNTS_OUT
        );
    }

    #[test]
    fn the_call_has_the_expected_layout() {
        let wbnb = EvmAddress::parse(WBNB).unwrap();
        let usdt = crate::usdt_address();
        let d = amounts_out_call(10u128.pow(18), wbnb, usdt);
        assert_eq!(d.len(), 4 + 32 * 5);
        assert_eq!(&d[..4], &SEL_GET_AMOUNTS_OUT);
        assert_eq!(crate::abi::read_u256(&d[4..36]).unwrap(), 10u128.pow(18));
        assert_eq!(
            crate::abi::read_u256(&d[36..68]).unwrap(),
            0x40,
            "array offset"
        );
        assert_eq!(
            crate::abi::read_u256(&d[68..100]).unwrap(),
            2,
            "array length"
        );
        assert_eq!(&d[112..132], wbnb.as_bytes());
        assert_eq!(&d[144..164], usdt.as_bytes());
    }

    #[test]
    fn a_short_or_absent_reply_is_an_error_not_a_price() {
        assert!(read_last_amount(&[]).is_err());
        assert!(read_last_amount(&[0u8; 96]).is_err());
        // Well-formed: offset, length, then two amounts.
        let mut ok = vec![0u8; 128];
        ok[127] = 42;
        assert_eq!(read_last_amount(&ok).unwrap(), 42);
    }

    /// The address argument is the bare twenty bytes, left-padded - the same
    /// trap as a token transfer's recipient.
    #[test]
    fn addresses_are_left_padded_to_a_word() {
        let a = EvmAddress::parse(WBNB).unwrap();
        let w = word_addr(a);
        assert!(w[..12].iter().all(|b| *b == 0));
        assert_eq!(&w[12..], a.as_bytes());
    }
}
