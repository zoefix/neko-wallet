//! The four ERC-20 calls this wallet makes, encoded by hand.
//!
//! A full ABI coder is a parser for arbitrary types; what is needed here is
//! four fixed shapes, each a selector followed by 32-byte words. Writing them
//! out means the bytes that get signed are visible in this file.
//!
//! The trap specific to token transfers: an address argument is the **20-byte**
//! form, left-padded to 32 bytes. TRON has the mirror-image trap - there the
//! on-chain address carries a `0x41` prefix that must be dropped for ABI. Both
//! produce a call that looks well-formed and pays the wrong account.

use neko_hd::EvmAddress;

use crate::error::EvmError;

/// `keccak256("balanceOf(address)")[..4]`
pub const SEL_BALANCE_OF: [u8; 4] = [0x70, 0xa0, 0x82, 0x31];
/// `keccak256("transfer(address,uint256)")[..4]`
pub const SEL_TRANSFER: [u8; 4] = [0xa9, 0x05, 0x9c, 0xbb];
/// `keccak256("symbol()")[..4]`
pub const SEL_SYMBOL: [u8; 4] = [0x95, 0xd8, 0x9b, 0x41];
/// `keccak256("decimals()")[..4]`
pub const SEL_DECIMALS: [u8; 4] = [0x31, 0x3c, 0xe5, 0x67];

fn word_address(a: EvmAddress) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[12..].copy_from_slice(a.as_bytes());
    w
}

fn word_u256(v: u128) -> [u8; 32] {
    let mut w = [0u8; 32];
    w[16..].copy_from_slice(&v.to_be_bytes());
    w
}

pub fn balance_of(holder: EvmAddress) -> Vec<u8> {
    let mut out = SEL_BALANCE_OF.to_vec();
    out.extend_from_slice(&word_address(holder));
    out
}

pub fn transfer(to: EvmAddress, amount: u128) -> Vec<u8> {
    let mut out = SEL_TRANSFER.to_vec();
    out.extend_from_slice(&word_address(to));
    out.extend_from_slice(&word_u256(amount));
    out
}

pub fn symbol() -> Vec<u8> {
    SEL_SYMBOL.to_vec()
}

pub fn decimals() -> Vec<u8> {
    SEL_DECIMALS.to_vec()
}

/// Read a `uint256` return value.
///
/// Refuses anything above 128 bits rather than truncating. A silently wrapped
/// balance is worse than an error: it would be displayed as a real number.
pub fn read_u256(data: &[u8]) -> Result<u128, EvmError> {
    if data.len() < 32 {
        return Err(EvmError::BadReply(format!(
            "expected a 32-byte word, got {} bytes",
            data.len()
        )));
    }
    let w = &data[..32];
    if w[..16].iter().any(|b| *b != 0) {
        return Err(EvmError::AmountTooLarge);
    }
    let mut b = [0u8; 16];
    b.copy_from_slice(&w[16..]);
    Ok(u128::from_be_bytes(b))
}

/// Read a dynamically sized `string` return value.
///
/// Some older tokens - including well-known ones - return a fixed `bytes32`
/// here instead, so a reply that is exactly one word is read that way rather
/// than rejected.
pub fn read_string(data: &[u8]) -> Result<String, EvmError> {
    if data.len() == 32 {
        let s = String::from_utf8_lossy(data)
            .trim_end_matches('\0')
            .trim()
            .to_string();
        return Ok(s);
    }
    if data.len() < 64 {
        return Err(EvmError::BadReply("string reply is too short".into()));
    }
    let len = read_u256(&data[32..64])? as usize;
    let start = 64usize;
    let end = start
        .checked_add(len)
        .ok_or_else(|| EvmError::BadReply("string length overflows".into()))?;
    if end > data.len() {
        return Err(EvmError::BadReply(format!(
            "string claims {len} bytes but only {} follow",
            data.len() - start
        )));
    }
    Ok(String::from_utf8_lossy(&data[start..end]).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tx::keccak;

    /// The selectors are the first four bytes of the keccak hash of the
    /// signature. Hardcoded for legibility, checked here so a typo cannot turn
    /// a transfer into a call to some other function.
    #[test]
    fn selectors_match_their_signatures() {
        for (sel, sig) in [
            (SEL_BALANCE_OF, "balanceOf(address)"),
            (SEL_TRANSFER, "transfer(address,uint256)"),
            (SEL_SYMBOL, "symbol()"),
            (SEL_DECIMALS, "decimals()"),
        ] {
            assert_eq!(&keccak(sig.as_bytes())[..4], &sel, "selector for {sig}");
        }
    }

    /// The layout of a transfer call, byte for byte: selector, then the
    /// address right-aligned in a 32-byte word, then the amount.
    #[test]
    fn a_transfer_call_has_the_expected_layout() {
        let to = EvmAddress::parse("0x3535353535353535353535353535353535353535").unwrap();
        let data = transfer(to, 1_000_000_000_000_000_000);
        assert_eq!(data.len(), 4 + 32 + 32);
        assert_eq!(&data[..4], &SEL_TRANSFER);
        // Twelve zero bytes of padding, then the address.
        assert!(
            data[4..16].iter().all(|b| *b == 0),
            "address is not left-padded"
        );
        assert_eq!(&data[16..36], to.as_bytes());
        assert_eq!(read_u256(&data[36..]).unwrap(), 1_000_000_000_000_000_000);
    }

    /// A balance too large for 128 bits must be an error, never a wrapped
    /// number that would be shown to somebody as their balance.
    #[test]
    fn oversized_values_are_refused_not_truncated() {
        let mut w = [0u8; 32];
        w[0] = 1; // 2^248
        assert!(matches!(read_u256(&w), Err(EvmError::AmountTooLarge)));

        let mut max = [0xffu8; 32];
        assert!(read_u256(&max).is_err());
        max[..16].fill(0);
        assert_eq!(read_u256(&max).unwrap(), u128::MAX);
    }

    #[test]
    fn strings_are_read_in_both_shapes() {
        // The dynamic form: offset, length, bytes.
        let mut d = vec![0u8; 64];
        d[31] = 0x20;
        d[63] = 4;
        d.extend_from_slice(b"USDT");
        d.resize(96, 0);
        assert_eq!(read_string(&d).unwrap(), "USDT");

        // The old fixed-width form some tokens still use.
        let mut b32 = [0u8; 32];
        b32[..4].copy_from_slice(b"USDT");
        assert_eq!(read_string(&b32).unwrap(), "USDT");
    }

    /// A malformed reply must not panic - it arrives over the network from a
    /// node we do not control.
    #[test]
    fn malformed_replies_do_not_panic() {
        assert!(read_u256(&[]).is_err());
        assert!(read_u256(&[0u8; 31]).is_err());
        assert!(read_string(&[]).is_err());
        assert!(read_string(&[0u8; 63]).is_err());
        // Claims a huge length with nothing behind it.
        let mut d = vec![0u8; 64];
        d[63] = 0xff;
        assert!(read_string(&d).is_err());
    }
}
