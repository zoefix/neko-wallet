//! Building and signing an Aptos transaction.
//!
//! The shape is fixed by the chain and every field is signed over, so this is
//! the file where a mistake is silent rather than loud: BCS carries no types,
//! so a misplaced length byte does not produce a malformed transaction, it
//! produces *a different valid one*, and the signature over it is correct.
//!
//! What is signed is not the transaction bytes but `sha3_256("APTOS::" ...)`
//! followed by them. That prefix is what stops a signature over a transaction
//! being replayed as a signature over anything else the chain hashes.

use zeroize::Zeroizing;

use crate::address::AptosAddress;
use crate::bcs::Writer;
use crate::error::AptosError;

/// The domain separator, hashed and prepended before signing.
///
/// Aptos derives it as `sha3_256(b"APTOS::RawTransaction")`. Computed here
/// rather than written down as a constant, because a wrong constant would
/// still sign and the chain would simply reject every transaction.
pub fn signing_prefix() -> [u8; 32] {
    use sha3::{Digest, Sha3_256};
    let mut h = Sha3_256::new();
    h.update(b"APTOS::RawTransaction");
    let mut out = [0u8; 32];
    out.copy_from_slice(&h.finalize());
    out
}

/// Everything the chain needs that this wallet cannot work out for itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxParams {
    /// The account's next sequence number. Aptos's replay protection, and the
    /// equivalent of a nonce: a transaction is valid at exactly one value.
    pub sequence_number: u64,
    /// The ceiling on gas *units*, not on the fee.
    pub max_gas_amount: u64,
    /// Octas per gas unit.
    pub gas_unit_price: u64,
    /// Unix seconds after which the chain will not accept this transaction.
    pub expiration_timestamp_secs: u64,
    /// 1 on mainnet. Signed over, so a transaction built for one network
    /// cannot be replayed on another.
    pub chain_id: u8,
}

impl TxParams {
    /// The most this transaction can cost, in octas.
    ///
    /// Gas units times price, which is what the chain checks the balance
    /// against - not what it is expected to spend.
    pub fn max_fee(&self) -> u128 {
        self.max_gas_amount as u128 * self.gas_unit_price as u128
    }
}

/// A call to an entry function, which is the only kind of transaction this
/// wallet builds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntryFunction {
    pub module_address: AptosAddress,
    pub module_name: String,
    pub function: String,
    /// Generic parameters, as fully-qualified struct tags.
    pub ty_args: Vec<StructTag>,
    /// Each argument already BCS-encoded, then written as a byte string.
    pub args: Vec<Vec<u8>>,
}

/// A fully-qualified Move type, used for the generic parameter on a fungible
/// asset transfer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructTag {
    pub address: AptosAddress,
    pub module: String,
    pub name: String,
}

impl StructTag {
    fn write(&self, w: &mut Writer) {
        // TypeTag::Struct is variant 7.
        w.variant(7);
        w.fixed(self.address.as_bytes());
        w.str(&self.module);
        w.str(&self.name);
        // No nested type parameters on anything this wallet sends.
        w.uleb(0);
    }
}

impl EntryFunction {
    fn write(&self, w: &mut Writer) {
        w.fixed(self.module_address.as_bytes());
        w.str(&self.module_name);
        w.str(&self.function);
        w.uleb(self.ty_args.len() as u64);
        for t in &self.ty_args {
            t.write(w);
        }
        w.uleb(self.args.len() as u64);
        for a in &self.args {
            w.bytes(a);
        }
    }
}

/// The transaction as the chain sees it, before a signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawTransaction {
    pub sender: AptosAddress,
    pub payload: EntryFunction,
    pub params: TxParams,
}

impl RawTransaction {
    /// BCS, in the order the chain defines. The order is part of the
    /// signature, so it is written out here rather than derived.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.fixed(self.sender.as_bytes());
        w.u64(self.params.sequence_number);
        // TransactionPayload::EntryFunction is variant 2.
        w.variant(2);
        self.payload.write(&mut w);
        w.u64(self.params.max_gas_amount);
        w.u64(self.params.gas_unit_price);
        w.u64(self.params.expiration_timestamp_secs);
        w.u8(self.params.chain_id);
        w.into_bytes()
    }

    /// What the key actually signs: the prefix hash, then the transaction.
    pub fn signing_message(&self) -> Vec<u8> {
        let mut m = signing_prefix().to_vec();
        m.extend_from_slice(&self.to_bytes());
        m
    }
}

/// A transaction and the authenticator that makes it spendable.
pub struct SignedTransaction {
    pub raw: Vec<u8>,
    /// The transaction hash, which is what an explorer link needs.
    pub hash: String,
}

/// Sign, and check the signature before handing it back.
///
/// The verification is not ceremony: an Ed25519 signature is deterministic, so
/// a wrong one here means the key or the message was wrong, and both produce a
/// transaction that is broadcast, accepted into a mempool, and then silently
/// dropped.
pub fn sign(
    raw: &RawTransaction,
    sk: &Zeroizing<[u8; 32]>,
) -> Result<SignedTransaction, AptosError> {
    let pk = neko_hd::aptos::public_key(sk);
    let derived = AptosAddress::from_public_key(&pk);
    if derived != raw.sender {
        return Err(AptosError::BadReply(format!(
            "this key controls {derived}, not {}",
            raw.sender
        )));
    }

    let message = raw.signing_message();
    let sig = neko_hd::aptos::sign(sk, &message);
    verify(&pk, &message, &sig)?;

    let mut w = Writer::new();
    w.fixed(&raw.to_bytes());
    // TransactionAuthenticator::Ed25519 is variant 0.
    w.variant(0);
    // Both the key and the signature are length-prefixed byte strings.
    w.bytes(&pk);
    w.bytes(&sig);
    let bytes = w.into_bytes();

    Ok(SignedTransaction {
        hash: transaction_hash(&bytes),
        raw: bytes,
    })
}

fn verify(pk: &[u8; 32], message: &[u8], sig: &[u8; 64]) -> Result<(), AptosError> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let vk = VerifyingKey::from_bytes(pk)
        .map_err(|_| AptosError::BadReply("the derived public key is not on the curve".into()))?;
    vk.verify(message, &Signature::from_bytes(sig))
        .map_err(|_| {
            AptosError::BadReply("the signature this wallet just made does not verify".into())
        })
}

/// The hash an explorer knows this transaction by.
///
/// `sha3_256("APTOS::Transaction" hashed, then the enum variant, then the
/// signed bytes)`. The variant byte is there because `Transaction` is an enum
/// and a user transaction is its first arm; leaving it out produces a hash
/// that is wrong in a way nothing local can detect.
pub fn transaction_hash(signed: &[u8]) -> String {
    use sha3::{Digest, Sha3_256};
    let mut prefix = Sha3_256::new();
    prefix.update(b"APTOS::Transaction");
    let mut h = Sha3_256::new();
    h.update(prefix.finalize());
    h.update([0u8]);
    h.update(signed);
    format!("0x{}", hex::encode(h.finalize()))
}

// ── The two transfers this wallet builds ────────────────────────────────────

/// `0x1::aptos_account::transfer(to, amount)`.
///
/// Not `0x1::coin::transfer`. The `aptos_account` version creates the
/// recipient's account if it does not exist yet; the `coin` version fails at
/// the node with an error about a missing store, which reads like the wallet
/// being broken rather than the recipient being new.
pub fn transfer_apt(to: AptosAddress, octas: u64) -> EntryFunction {
    EntryFunction {
        module_address: framework(),
        module_name: "aptos_account".into(),
        function: "transfer".into(),
        ty_args: Vec::new(),
        args: vec![to.as_bytes().to_vec(), octas.to_le_bytes().to_vec()],
    }
}

/// `0x1::primary_fungible_store::transfer<Metadata>(metadata, to, amount)`.
///
/// Aptos's USDT is a *fungible asset*, not a coin, and the two have different
/// entry points. A coin transfer against a fungible asset does not move a
/// smaller amount or a different token - it does not compile against the
/// chain, and the transaction aborts.
pub fn transfer_fungible_asset(
    metadata: AptosAddress,
    to: AptosAddress,
    amount: u64,
) -> EntryFunction {
    EntryFunction {
        module_address: framework(),
        module_name: "primary_fungible_store".into(),
        function: "transfer".into(),
        ty_args: vec![StructTag {
            address: framework(),
            module: "fungible_asset".into(),
            name: "Metadata".into(),
        }],
        args: vec![
            metadata.as_bytes().to_vec(),
            to.as_bytes().to_vec(),
            amount.to_le_bytes().to_vec(),
        ],
    }
}

/// `0x1`, where the Aptos framework lives.
pub fn framework() -> AptosAddress {
    AptosAddress::parse("0x1").expect("0x1 is a valid address")
}

/// The bytes to hand a simulation.
///
/// A simulation is a real transaction with a signature-shaped field the node
/// deliberately ignores, so it can be asked what a transfer would cost before
/// a key is ever touched. The zeros here are not a signature and can never
/// become one: the node rejects a *submitted* transaction whose signature does
/// not verify, so these bytes are useless for anything but asking.
pub fn simulation_bytes(raw: &RawTransaction, public_key: &[u8; 32]) -> Vec<u8> {
    let mut w = Writer::new();
    w.fixed(&raw.to_bytes());
    w.variant(0);
    // The *real* public key. The node checks it against the account's
    // authentication key and refuses with INVALID_AUTH_KEY otherwise, so a
    // placeholder here does not simulate - it fails.
    w.bytes(public_key);
    // And a signature of zeros, which the node is documented to ignore for a
    // simulation. It cannot become a real transaction: a submitted one whose
    // signature does not verify is rejected.
    w.bytes(&[0u8; 64]);
    w.into_bytes()
}

#[cfg(test)]
mod encoding {
    use super::*;

    fn params() -> TxParams {
        TxParams {
            sequence_number: 11,
            max_gas_amount: 2_000,
            gas_unit_price: 100,
            expiration_timestamp_secs: 1_900_000_000,
            chain_id: crate::CHAIN_ID,
        }
    }

    fn sender() -> AptosAddress {
        AptosAddress::parse("0xeb663b681209e7087d681c5d3eed12aaa8e1915e7c87794542c3f96e94b3d3bf")
            .unwrap()
    }

    fn recipient() -> AptosAddress {
        AptosAddress::parse("0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef")
            .unwrap()
    }

    /// The signing message, byte for byte, as Aptos's own encoder produces it.
    ///
    /// Not a self-consistent fixture: these bytes came from the node's
    /// `/transactions/encode_submission`, which takes the transaction as JSON
    /// and returns the bytes a signature must cover. Two independent encoders
    /// agreeing on all 197 is what makes this file trustworthy, because BCS is
    /// not self-describing - a wrong length byte here would not be rejected as
    /// malformed, it would be a different transaction, signed correctly.
    ///
    /// The prefix at the front is `sha3_256("APTOS::RawTransaction")`, which
    /// the node includes too.
    #[test]
    fn an_apt_transfer_matches_the_nodes_own_encoding() {
        let raw = RawTransaction {
            sender: sender(),
            payload: transfer_apt(recipient(), 12_345_678),
            params: params(),
        };
        assert_eq!(
            hex::encode(raw.signing_message()),
            "b5e97db07fa0bd0e5598aa3643a9bc6f6693bddc1a9fec9e674a461eaa00b193\
             eb663b681209e7087d681c5d3eed12aaa8e1915e7c87794542c3f96e94b3d3bf\
             0b00000000000000\
             02\
             0000000000000000000000000000000000000000000000000000000000000001\
             0d6170746f735f6163636f756e74\
             087472616e73666572\
             00\
             02\
             201234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef\
             084e61bc0000000000\
             d007000000000000\
             6400000000000000\
             00b33f7100000000\
             01"
        );
    }

    /// The fungible-asset transfer, which carries a type argument the coin one
    /// does not - and which is the path Aptos's USDT actually takes.
    #[test]
    fn a_fungible_asset_transfer_matches_the_nodes_own_encoding() {
        let raw = RawTransaction {
            sender: sender(),
            payload: transfer_fungible_asset(crate::usdt_metadata(), recipient(), 1_000_000),
            params: params(),
        };
        assert_eq!(
            hex::encode(raw.signing_message()),
            "b5e97db07fa0bd0e5598aa3643a9bc6f6693bddc1a9fec9e674a461eaa00b193\
             eb663b681209e7087d681c5d3eed12aaa8e1915e7c87794542c3f96e94b3d3bf\
             0b00000000000000\
             02\
             0000000000000000000000000000000000000000000000000000000000000001\
             167072696d6172795f66756e6769626c655f73746f7265\
             087472616e73666572\
             01\
             07\
             0000000000000000000000000000000000000000000000000000000000000001\
             0e66756e6769626c655f6173736574\
             084d65746164617461\
             00\
             03\
             20357b0b74bc833e95a115ad22604854d6b0fca151cecd94111770e5d6ffc9dc2b\
             201234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef\
             0840420f0000000000\
             d007000000000000\
             6400000000000000\
             00b33f7100000000\
             01"
        );
    }

    /// The domain separator, which is what stops a signature over one of these
    /// being a valid signature over anything else the chain hashes.
    #[test]
    fn the_prefix_is_the_hash_of_the_type_name() {
        assert_eq!(
            hex::encode(signing_prefix()),
            "b5e97db07fa0bd0e5598aa3643a9bc6f6693bddc1a9fec9e674a461eaa00b193"
        );
    }

    /// Changing any signed field changes the bytes.
    ///
    /// Each of these is a way a transaction could be replayed or misdirected
    /// if it were left out of the signature: the sequence number is the replay
    /// guard, the chain id is what keeps a mainnet transfer off a testnet, and
    /// the rest is the payment itself.
    #[test]
    fn every_field_is_covered_by_the_signature() {
        let base = RawTransaction {
            sender: sender(),
            payload: transfer_apt(recipient(), 12_345_678),
            params: params(),
        };
        let bytes = base.signing_message();

        let mut variants = Vec::new();
        let mut p = params();
        p.sequence_number += 1;
        variants.push(RawTransaction {
            params: p,
            ..base.clone()
        });
        let mut p = params();
        p.chain_id = 2;
        variants.push(RawTransaction {
            params: p,
            ..base.clone()
        });
        let mut p = params();
        p.gas_unit_price += 1;
        variants.push(RawTransaction {
            params: p,
            ..base.clone()
        });
        let mut p = params();
        p.max_gas_amount += 1;
        variants.push(RawTransaction {
            params: p,
            ..base.clone()
        });
        let mut p = params();
        p.expiration_timestamp_secs += 1;
        variants.push(RawTransaction {
            params: p,
            ..base.clone()
        });
        variants.push(RawTransaction {
            payload: transfer_apt(recipient(), 12_345_679),
            ..base.clone()
        });
        variants.push(RawTransaction {
            payload: transfer_apt(sender(), 12_345_678),
            ..base.clone()
        });
        variants.push(RawTransaction {
            sender: recipient(),
            ..base.clone()
        });

        for (i, v) in variants.iter().enumerate() {
            assert_ne!(
                v.signing_message(),
                bytes,
                "variant {i} produced the same signing message"
            );
        }
    }
}
