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

// The routers and wrapped coins live on `EvmChain`: PancakeSwap on BNB Chain,
// Uniswap V2 on Ethereum, and the second is a fork of the first, so one
// `getAmountsOut` serves both.
//
// BTCB - Binance-pegged Bitcoin - is in `chain_consts` for a different reason:
// Bitcoin has no exchange on its own chain, so BTC is priced from the BTCB pool
// on BNB Chain rather than from a price service, which would be a new
// destination learning which addresses this wallet cares about.

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
    /// What one native coin is worth, in this chain's USDT.
    ///
    /// Quoted at the chain's own `usdt_decimals`, which differ - six on
    /// Ethereum, eighteen on BNB Chain - so the caller must rescale rather than
    /// assume.
    pub async fn native_price_in_usdt(&self) -> Result<u128, EvmError> {
        let chain = self.chain();
        // A chain that names somewhere else for its price has no pool worth
        // reading here, and reading it anyway returns a number rather than an
        // error - Base's answers about seventeen dollars for an ether. Refused
        // rather than returned: the caller has to go to `price_chain()`.
        if let Some(elsewhere) = chain.prices_on {
            return Err(EvmError::Rpc(format!(
                "chain {} has no pool for its own coin; it is priced on chain {elsewhere}",
                chain.chain_id
            )));
        }
        let data = amounts_out_call(
            10u128.pow(chain.native_decimals as u32),
            chain.wrapped_native_address(),
            chain.usdt_address(),
        );
        read_last_amount(&self.eth_call(chain.router_address(), &data).await?)
    }

    /// What one BTCB is worth, in USDT, at [`crate::USDT_DECIMALS`].
    ///
    /// One BTCB, not one satoshi: the caller converts. Asking for a whole unit
    /// keeps the quote out of the part of the curve where a tiny trade prices
    /// badly.
    /// What one BTCB is worth, in USDT. Only meaningful on BNB Chain, which is
    /// where BTCB lives.
    pub async fn btcb_price_in_usdt(&self) -> Result<u128, EvmError> {
        let chain = self.chain();
        let btcb = EvmAddress::parse(crate::BTCB)?;
        let data = amounts_out_call(
            10u128.pow(crate::BTCB_DECIMALS as u32),
            btcb,
            chain.usdt_address(),
        );
        read_last_amount(&self.eth_call(chain.router_address(), &data).await?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mistyped contract address would quote something else entirely, and a
    /// round trip through EIP-55 proves both that it is well-formed and that
    /// no character was transposed.
    #[test]
    fn the_selector_matches_its_signature() {
        assert_eq!(
            &crate::tx::keccak(b"getAmountsOut(uint256,address[])")[..4],
            &SEL_GET_AMOUNTS_OUT
        );
    }

    #[test]
    fn the_call_has_the_expected_layout() {
        let wbnb = crate::BSC.wrapped_native_address();
        let usdt = crate::BSC.usdt_address();
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
        let a = crate::BSC.wrapped_native_address();
        let w = word_addr(a);
        assert!(w[..12].iter().all(|b| *b == 0));
        assert_eq!(&w[12..], a.as_bytes());
    }
}

#[cfg(test)]
mod pricing_chain {
    /// Base's pool is not a price, and asking for one has to fail rather than
    /// answer.
    ///
    /// The pair exists - 0.0069 WETH against 17 USDT when this was written -
    /// so `getAmountsOut` returns a number and nothing about the reply says it
    /// is meaningless. That number was about 17, for an asset worth 2,447.
    #[tokio::test]
    async fn a_chain_that_prices_elsewhere_refuses_to_price_itself() {
        let rpc = crate::client::Rpc::new(crate::BASE, None);
        let err = rpc
            .native_price_in_usdt()
            .await
            .expect_err("Base answered with its own empty pool");
        let msg = err.to_string();
        assert!(msg.contains("8453"), "{msg}");
        assert!(msg.contains("priced on chain 1"), "{msg}");
    }
}
