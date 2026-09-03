//! BIP39 / BIP32 / BIP44 derivation.
//!
//! Path: `m/44'/{coin}'/{account}'/0/{index}`, where `coin` is the SLIP-44
//! coin type: 195 for TRON, 60 for EVM chains. BNB Chain uses 60 rather than a
//! number of its own, which is the convention every EVM wallet follows - so an
//! address derived here matches what MetaMask or a Ledger would show for the
//! same phrase.
//!
//! One consequence is worth stating plainly, because it surprises people: the
//! same mnemonic produces a *different* address on each chain. That is correct
//! and universal, not a bug.
//!
//! The account level is *hardened*, which is load-bearing: a compromised
//! account branch cannot be walked back up to the master key or sideways into
//! another account.

use bip32::{DerivationPath, XPrv};
use k256::elliptic_curve::sec1::ToEncodedPoint;
use zeroize::Zeroizing;

use crate::address::Address;
use crate::error::HdError;

/// SLIP-44 coin type for TRON.
pub const COIN_TYPE: u32 = 195;
/// SLIP-44 coin type for EVM chains, BNB Chain included.
pub const COIN_TYPE_EVM: u32 = 60;
/// Guard rail from the reference implementation; also keeps a typo'd index from
/// spending minutes in derivation.
pub const MAX_INDEX: u32 = 1 << 24;

pub type PrivKey = Zeroizing<[u8; 32]>;

/// TRON's path, kept as its own function because the frozen vectors call it.
pub fn path_for(account: u32, index: u32) -> String {
    path_for_coin(COIN_TYPE, account, index)
}

pub fn path_for_coin(coin: u32, account: u32, index: u32) -> String {
    format!("m/44'/{coin}'/{account}'/0/{index}")
}

/// Generate a fresh BIP39 mnemonic. 12 words (128 bits) by default.
///
/// English wordlist only, deliberately: cross-wallet compatibility with
/// non-English BIP39 wordlists is a recurring source of permanently lost funds.
/// Import accepts other languages; generation does not offer them.
pub fn generate_mnemonic(words: usize) -> Result<Zeroizing<String>, HdError> {
    let bytes = match words {
        12 => 16usize,
        24 => 32,
        _ => return Err(HdError::BadEntropyLen(words)),
    };
    let mut entropy = Zeroizing::new(vec![0u8; bytes]);
    getrandom_fill(&mut entropy)?;
    mnemonic_from_entropy(&entropy)
}

fn getrandom_fill(buf: &mut [u8]) -> Result<(), HdError> {
    use k256::elliptic_curve::rand_core::RngCore;
    k256::elliptic_curve::rand_core::OsRng.fill_bytes(buf);
    Ok(())
}

pub fn mnemonic_from_entropy(entropy: &[u8]) -> Result<Zeroizing<String>, HdError> {
    if entropy.len() != 16 && entropy.len() != 32 {
        return Err(HdError::BadEntropyLen(entropy.len()));
    }
    let m = bip39::Mnemonic::from_entropy(entropy).map_err(|_| HdError::BadMnemonic)?;
    Ok(Zeroizing::new(m.to_string()))
}

pub fn entropy_from_mnemonic(phrase: &str) -> Result<Zeroizing<Vec<u8>>, HdError> {
    let m = parse_mnemonic(phrase)?;
    let (bytes, len) = m.to_entropy_array();
    Ok(Zeroizing::new(bytes[..len].to_vec()))
}

pub fn validate_mnemonic(phrase: &str) -> bool {
    parse_mnemonic(phrase).is_ok()
}

/// Accepts any BIP39 wordlist the crate knows, so a wallet exported from
/// Chinese- or Japanese-language software can still be imported.
fn parse_mnemonic(phrase: &str) -> Result<bip39::Mnemonic, HdError> {
    bip39::Mnemonic::parse(phrase).map_err(|_| HdError::BadMnemonic)
}

/// BIP39 seed. The full path is walked — entropy to phrase to PBKDF2 seed —
/// never entropy-as-seed, which is the shortcut that silently produces a
/// different (and unrecoverable) wallet.
pub fn seed_from_mnemonic(phrase: &str, passphrase: &str) -> Result<Zeroizing<[u8; 64]>, HdError> {
    let m = parse_mnemonic(phrase)?;
    Ok(Zeroizing::new(m.to_seed(passphrase)))
}

pub fn master_from_seed(seed: &[u8; 64]) -> Result<XPrv, HdError> {
    XPrv::new(seed).map_err(|_| HdError::Derive)
}

pub fn derive_xprv(seed: &[u8; 64], path: &str) -> Result<XPrv, HdError> {
    let p: DerivationPath = path.parse().map_err(|_| HdError::Derive)?;
    XPrv::derive_from_path(seed, &p).map_err(|_| HdError::Derive)
}

/// Private key at `m/44'/195'/{account}'/0/{index}`.
pub fn private_key_at(seed: &[u8; 64], account: u32, index: u32) -> Result<PrivKey, HdError> {
    private_key_at_coin(seed, COIN_TYPE, account, index)
}

pub fn private_key_at_coin(
    seed: &[u8; 64],
    coin: u32,
    account: u32,
    index: u32,
) -> Result<PrivKey, HdError> {
    if index >= MAX_INDEX {
        return Err(HdError::IndexOutOfRange(index));
    }
    let xprv = derive_xprv(seed, &path_for_coin(coin, account, index))?;
    Ok(Zeroizing::new(xprv.private_key().to_bytes().into()))
}

/// TRON address for a private key.
pub fn address_from_private_key(sk: &[u8; 32]) -> Result<Address, HdError> {
    let signing = k256::SecretKey::from_slice(sk).map_err(|_| HdError::BadPrivateKey)?;
    // BIP32 gives a compressed point; TRON hashes the *uncompressed* one.
    let point = signing.public_key().to_encoded_point(false);
    Address::from_public_key(point.as_bytes())
}

pub fn address_at(seed: &[u8; 64], account: u32, index: u32) -> Result<Address, HdError> {
    let sk = private_key_at(seed, account, index)?;
    address_from_private_key(&sk)
}

/// The 65-byte uncompressed public key for a private key.
///
/// BIP32 hands out a 33-byte compressed point; both TRON and the EVM chains
/// hash the uncompressed one. Getting this wrong yields a valid-looking
/// address that belongs to nobody, so it is done in exactly one place.
fn uncompressed_pubkey(sk: &[u8; 32]) -> Result<[u8; 65], HdError> {
    let secret = k256::SecretKey::from_slice(sk).map_err(|_| HdError::BadPrivateKey)?;
    let point = secret.public_key().to_encoded_point(false);
    let bytes = point.as_bytes();
    if bytes.len() != 65 {
        return Err(HdError::BadPublicKeyLen(bytes.len()));
    }
    let mut out = [0u8; 65];
    out.copy_from_slice(bytes);
    Ok(out)
}

/// EVM address for a private key.
pub fn evm_address_from_private_key(sk: &[u8; 32]) -> Result<crate::EvmAddress, HdError> {
    crate::EvmAddress::from_public_key(&uncompressed_pubkey(sk)?)
}

/// EVM address at `m/44'/60'/{account}'/0/{index}`.
pub fn evm_address_at(
    seed: &[u8; 64],
    account: u32,
    index: u32,
) -> Result<crate::EvmAddress, HdError> {
    let sk = private_key_at_coin(seed, COIN_TYPE_EVM, account, index)?;
    evm_address_from_private_key(&sk)
}

/// Private key at `m/44'/60'/{account}'/0/{index}`.
pub fn evm_private_key_at(seed: &[u8; 64], account: u32, index: u32) -> Result<PrivKey, HdError> {
    private_key_at_coin(seed, COIN_TYPE_EVM, account, index)
}
