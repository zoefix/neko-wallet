//! Wallet v4R2: the contract that holds the money.
//!
//! There is no account on this chain that a private key simply owns. A wallet
//! is a deployed smart contract whose storage holds a public key, and whose
//! *address is the hash of its own initial code and storage*. So the address
//! depends on the contract version as much as on the key: the same phrase under
//! v4R2 and under v5R1 is two different addresses, both real, and coins sent to
//! one are not visible at the other.
//!
//! One consequence catches people out and is worth stating plainly: the address
//! exists before the contract does. Coins can be sent to a wallet that has never
//! been deployed, and they arrive. What cannot happen is spending them - that
//! needs the contract, so the first outgoing transfer has to carry the code with
//! it and deploy it on the way.

use std::sync::Arc;

use crate::address::{TonAddress, BASECHAIN};
use crate::cell::{Cell, CellBuilder};
use crate::error::TonError;

/// Wallet v4R2's compiled code, as deployed on mainnet.
///
/// Verified by its hash rather than by trust: see [`CODE_HASH`].
const CODE_BOC: &[u8] = include_bytes!("../vectors/wallet_v4r2.boc");

/// The published code hash of wallet v4R2. Checked on every load, because this
/// is the byte string that decides where every address in this wallet is.
pub const CODE_HASH: &str = "feb5ff6820e2ff0d9483e7e0d62c817d846789fb4ae580c878866d959dabd5c0";

/// The subwallet id every standard wallet uses.
///
/// It is part of the contract's storage and therefore part of the address, so
/// it is not a preference: change it and the same key is a different wallet
/// that no other software will find.
pub const DEFAULT_SUBWALLET_ID: u32 = 698_983_191;

/// The code cell, parsed once.
pub fn code() -> Result<Arc<Cell>, TonError> {
    let c = crate::boc::parse(CODE_BOC)?;
    if hex::encode(c.hash()) != CODE_HASH {
        return Err(TonError::BadBoc(
            "the embedded wallet code is not wallet v4R2".into(),
        ));
    }
    Ok(c)
}

/// A wallet's storage at the moment it is deployed.
///
/// `seqno` is zero here and *only* here. It counts messages the wallet has
/// sent, so a live wallet's storage differs from this - but the address was
/// fixed by this, and computing it from current storage gives an address that
/// does not exist.
pub fn initial_data(public_key: &[u8; 32], subwallet_id: u32) -> Result<Arc<Cell>, TonError> {
    let mut b = CellBuilder::new();
    b.store_uint(0, 32)?
        .store_uint(subwallet_id as u64, 32)?
        .store_bytes(public_key)?
        // An empty plugin dictionary. One bit, and leaving it out shifts
        // nothing but changes the hash, which is the address.
        .store_bit(false)?;
    b.build_arc()
}

/// `StateInit`, the pair of code and storage that an address is the hash of.
///
/// The five leading bits are the schema's four absent optionals and the two
/// present ones: no split depth, not special, code here, data here, no library.
pub fn state_init(code: Arc<Cell>, data: Arc<Cell>) -> Result<Arc<Cell>, TonError> {
    let mut b = CellBuilder::new();
    b.store_bit(false)? // split_depth: nothing
        .store_bit(false)? // special: nothing
        .store_bit(true)? // code: present
        .store_bit(true)? // data: present
        .store_bit(false)? // library: nothing
        .store_ref(code)?
        .store_ref(data)?;
    b.build_arc()
}

/// Where a key's wallet lives, deployed or not.
pub fn address_for(public_key: &[u8; 32]) -> Result<TonAddress, TonError> {
    let init = state_init(code()?, initial_data(public_key, DEFAULT_SUBWALLET_ID)?)?;
    Ok(TonAddress::new(BASECHAIN, init.hash()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A wallet that exists on mainnet: its public key, and the address the
    /// chain says that key's wallet is at.
    ///
    /// This is the test the whole crate rests on. It exercises the cell
    /// builder's bit packing, the completion tag, the twenty-cell code tree
    /// parsed out of a bag, the recursive hash with its per-reference depths,
    /// the storage layout, and the StateInit schema - and the answer is an
    /// address, so anything wrong anywhere gives a different one.
    const PUBLIC_KEY: &str = "95d590294ec78f23b0f9afb5d1a911e0e5c1e37aafd6ebb215a4d5a3ac4a7890";
    const ADDRESS: &str = "0:5661bcb42ba847235760ce9aaa2dfff103eb7365db06e5df053120bacb77ddfd";

    fn key() -> [u8; 32] {
        hex::decode(PUBLIC_KEY).unwrap().try_into().unwrap()
    }

    #[test]
    fn a_real_wallet_address_is_reproduced() {
        assert_eq!(address_for(&key()).unwrap().to_raw_string(), ADDRESS);
    }

    #[test]
    fn the_embedded_code_is_wallet_v4r2() {
        assert_eq!(hex::encode(code().unwrap().hash()), CODE_HASH);
    }

    /// The address is fixed by the storage a wallet was *born* with. A live
    /// wallet's seqno is whatever it has sent; using that gives an address that
    /// belongs to nobody.
    #[test]
    fn the_address_comes_from_the_initial_storage_not_the_current() {
        let real = address_for(&key()).unwrap();

        // The same wallet after 2,548 messages - which is what it actually
        // holds today, and what the first attempt at this used.
        let mut b = CellBuilder::new();
        b.store_uint(2548, 32)
            .unwrap()
            .store_uint(DEFAULT_SUBWALLET_ID as u64, 32)
            .unwrap()
            .store_bytes(&key())
            .unwrap()
            .store_bit(false)
            .unwrap();
        let current = state_init(code().unwrap(), b.build_arc().unwrap()).unwrap();
        assert_ne!(current.hash(), real.hash);
    }

    /// The subwallet id is part of the address, not a setting.
    #[test]
    fn a_different_subwallet_id_is_a_different_wallet() {
        let a = state_init(
            code().unwrap(),
            initial_data(&key(), DEFAULT_SUBWALLET_ID).unwrap(),
        )
        .unwrap();
        let b = state_init(code().unwrap(), initial_data(&key(), 0).unwrap()).unwrap();
        assert_ne!(a.hash(), b.hash());
        assert_eq!(DEFAULT_SUBWALLET_ID, 698_983_191);
    }

    /// The storage cell is 321 bits: two 32-bit counters, a 256-bit key, and
    /// one bit of empty dictionary. The chain reports exactly that.
    #[test]
    fn the_storage_layout_is_what_the_chain_holds() {
        let d = initial_data(&key(), DEFAULT_SUBWALLET_ID).unwrap();
        assert_eq!(d.bits(), 32 + 32 + 256 + 1);
        assert!(d.refs().is_empty());
        assert_eq!(&d.data()[8..40], &key());
    }

    /// Different keys are different wallets, which is the whole point.
    #[test]
    fn each_key_gets_its_own_address() {
        let seed = [4u8; 64];
        let mut seen = Vec::new();
        for i in 0..4 {
            let sk = neko_hd::ton::private_key_at(&seed, i).unwrap();
            let a = address_for(&neko_hd::ton::public_key(&sk)).unwrap();
            assert!(!seen.contains(&a.hash), "account {i} repeated an address");
            seen.push(a.hash);
        }
    }
}
