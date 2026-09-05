//! SLIP-0010 key derivation over Ed25519.
//!
//! Shared by Solana and TON, which both sign with Ed25519 and therefore cannot
//! use the BIP32 machinery the secp256k1 chains use. Ed25519 has no public-key
//! addition, so SLIP-0010 defines no non-hardened child at all: every level of
//! every path here is hardened, and that is a property of the curve rather than
//! a choice.

use hmac::{Hmac, Mac};
use sha2::Sha512;
use zeroize::Zeroizing;

const ED25519_SEED_KEY: &[u8] = b"ed25519 seed";
/// Every index is hardened. SLIP-0010 defines no other kind for this curve.
pub const HARDENED: u32 = 0x8000_0000;

type HmacSha512 = Hmac<Sha512>;

/// A key and its chain code. Zeroized on drop, because the key here is the
/// whole wallet for one path.
struct Node {
    key: Zeroizing<[u8; 32]>,
    chain_code: Zeroizing<[u8; 32]>,
}

fn split(i: &[u8]) -> Node {
    let mut key = Zeroizing::new([0u8; 32]);
    let mut chain_code = Zeroizing::new([0u8; 32]);
    key.copy_from_slice(&i[..32]);
    chain_code.copy_from_slice(&i[32..]);
    Node { key, chain_code }
}

fn master(seed: &[u8]) -> Node {
    let mut mac =
        HmacSha512::new_from_slice(ED25519_SEED_KEY).expect("HMAC accepts a key of any length");
    mac.update(seed);
    split(&mac.finalize().into_bytes())
}

/// One hardened step. `index` is given unhardened and hardened here, so no
/// caller can accidentally ask for the child that does not exist.
fn child(parent: &Node, index: u32) -> Node {
    let mut data = [0u8; 37];
    // The leading zero byte is what distinguishes SLIP-0010's Ed25519 form from
    // BIP32's; without it this derives a different, valid-looking wallet.
    data[0] = 0;
    data[1..33].copy_from_slice(&*parent.key);
    data[33..].copy_from_slice(&(index | HARDENED).to_be_bytes());

    let mut mac =
        HmacSha512::new_from_slice(&*parent.chain_code).expect("HMAC accepts a key of any length");
    mac.update(&data);
    let out = split(&mac.finalize().into_bytes());
    data.zeroize_now();
    out
}

trait ZeroizeNow {
    fn zeroize_now(&mut self);
}
impl ZeroizeNow for [u8; 37] {
    fn zeroize_now(&mut self) {
        use zeroize::Zeroize;
        self.zeroize();
    }
}

/// Walk a path of hardened indices from a seed.
///
/// The seed is a slice rather than the BIP39 `[u8; 64]` so the official
/// SLIP-0010 vectors, which use a 16-byte seed, can exercise this exact code.
pub fn derive_path(seed: &[u8], path: &[u32]) -> Zeroizing<[u8; 32]> {
    let mut node = master(seed);
    for &i in path {
        node = child(&node, i);
    }
    node.key
}
