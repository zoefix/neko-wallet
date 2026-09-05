//! Building and signing a Sui transaction.
//!
//! Sui does not have a "transfer" instruction. Every transaction is a
//! *programmable block*: a list of inputs and a list of commands that refer to
//! them and to each other's results. A payment is two commands - split a coin,
//! then hand the piece to somebody - and the second refers to the first's
//! output rather than to anything named.
//!
//! The shape here was read off the chain rather than out of a document. A
//! transfer built by a Sui node's own `unsafe_paySui` was decoded byte by
//! byte, all 219 of them, and this file reproduces it.
//!
//! What is signed is not the transaction but
//! `blake2b256(intent || bcs(TransactionData))`, where the intent is three
//! bytes naming what is being signed. Without it a signature over a
//! transaction could be replayed as a signature over anything else the chain
//! hashes.

use zeroize::Zeroizing;

use crate::address::SuiAddress;
use crate::bcs::Writer;
use crate::error::SuiError;

/// A coin object, which on this chain is what a balance is made of.
///
/// All three fields are signed over. The version in particular: a coin object
/// changes version every time it is touched, and a transaction naming a stale
/// one is rejected rather than applied to the newer state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObjectRef {
    pub id: [u8; 32],
    pub version: u64,
    pub digest: [u8; 32],
}

impl ObjectRef {
    fn write(&self, w: &mut Writer) {
        w.fixed(&self.id);
        w.u64(self.version);
        // The digest is length-prefixed even though it is always 32 bytes.
        w.bytes(&self.digest);
    }
}

/// Where a command's operand comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Argument {
    /// The coin paying for gas, which can also be spent from.
    GasCoin,
    /// One of the transaction's inputs.
    Input(u16),
    /// A whole command's result.
    Result(u16),
    /// One value out of a command's result. Splitting a coin produces a list,
    /// so its first piece is `NestedResult(0, 0)` rather than `Result(0)`.
    NestedResult(u16, u16),
}

impl Argument {
    fn write(&self, w: &mut Writer) {
        match *self {
            Argument::GasCoin => {
                w.variant(0);
            }
            Argument::Input(i) => {
                w.variant(1);
                w.u16(i);
            }
            Argument::Result(i) => {
                w.variant(2);
                w.u16(i);
            }
            Argument::NestedResult(a, b) => {
                w.variant(3);
                w.u16(a);
                w.u16(b);
            }
        }
    }
}

/// A transaction's input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallArg {
    /// A value, BCS-encoded and then wrapped as bytes.
    Pure(Vec<u8>),
    /// An object this address owns - here, always a coin.
    OwnedObject(ObjectRef),
}

impl CallArg {
    fn write(&self, w: &mut Writer) {
        match self {
            CallArg::Pure(b) => {
                w.variant(0);
                w.bytes(b);
            }
            CallArg::OwnedObject(r) => {
                w.variant(1);
                // ObjectArg::ImmOrOwnedObject
                w.variant(0);
                r.write(w);
            }
        }
    }
}

/// The two commands a payment needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Cut `amounts` off `coin`, leaving the remainder where it was.
    SplitCoins {
        coin: Argument,
        amounts: Vec<Argument>,
    },
    /// Give `objects` to `address`.
    TransferObjects {
        objects: Vec<Argument>,
        address: Argument,
    },
    /// Fold several coin objects into the first.
    ///
    /// Needed when no single object holds enough: a balance on this chain is
    /// spread across objects, and a transfer spends objects rather than a
    /// balance.
    MergeCoins {
        destination: Argument,
        sources: Vec<Argument>,
    },
}

impl Command {
    fn write(&self, w: &mut Writer) {
        match self {
            // The variant numbers are the chain's, read off a transaction it
            // built: MoveCall 0, TransferObjects 1, SplitCoins 2, MergeCoins 3.
            Command::TransferObjects { objects, address } => {
                w.variant(1);
                w.uleb(objects.len() as u64);
                for o in objects {
                    o.write(w);
                }
                address.write(w);
            }
            Command::SplitCoins { coin, amounts } => {
                w.variant(2);
                coin.write(w);
                w.uleb(amounts.len() as u64);
                for a in amounts {
                    a.write(w);
                }
            }
            Command::MergeCoins {
                destination,
                sources,
            } => {
                w.variant(3);
                destination.write(w);
                w.uleb(sources.len() as u64);
                for s in sources {
                    s.write(w);
                }
            }
        }
    }
}

/// What pays for the transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GasData {
    /// The coin objects that will pay. Always SUI, whatever is being sent.
    pub payment: Vec<ObjectRef>,
    pub owner: SuiAddress,
    /// Per unit of computation, from the chain's reference price.
    pub price: u64,
    /// The ceiling. Unused budget is not charged, but the whole of it has to
    /// be available in the gas coins or the transaction is refused.
    pub budget: u64,
}

/// A whole transaction, before a signature.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionData {
    pub inputs: Vec<CallArg>,
    pub commands: Vec<Command>,
    pub sender: SuiAddress,
    pub gas: GasData,
}

impl TransactionData {
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut w = Writer::new();
        // TransactionData::V1
        w.variant(0);
        // TransactionKind::ProgrammableTransaction
        w.variant(0);
        w.uleb(self.inputs.len() as u64);
        for i in &self.inputs {
            i.write(&mut w);
        }
        w.uleb(self.commands.len() as u64);
        for c in &self.commands {
            c.write(&mut w);
        }
        w.fixed(self.sender.as_bytes());
        w.uleb(self.gas.payment.len() as u64);
        for p in &self.gas.payment {
            p.write(&mut w);
        }
        w.fixed(self.gas.owner.as_bytes());
        w.u64(self.gas.price);
        w.u64(self.gas.budget);
        // TransactionExpiration::None
        w.variant(0);
        w.into_bytes()
    }

    /// What the key signs: the intent, then the transaction, hashed.
    ///
    /// The intent is three bytes - scope, version, application - and naming
    /// the scope is the point: a signature made over a transaction cannot be
    /// presented as a signature over a personal message or a checkpoint.
    pub fn signing_digest(&self) -> [u8; 32] {
        let mut m = INTENT_TRANSACTION_DATA.to_vec();
        m.extend_from_slice(&self.to_bytes());
        crate::blake2b256(&m)
    }
}

/// Scope `TransactionData` (0), version 0, application Sui (0).
pub const INTENT_TRANSACTION_DATA: [u8; 3] = [0, 0, 0];

pub struct SignedTransaction {
    /// The transaction, BCS, as the node wants it.
    pub data: Vec<u8>,
    /// `flag || signature || public key`, which is how Sui carries a
    /// signature.
    pub signature: Vec<u8>,
    /// The transaction digest, which is the name an explorer knows it by.
    pub digest: String,
}

/// Sign, and verify the signature before handing it back.
pub fn sign(
    data: &TransactionData,
    sk: &Zeroizing<[u8; 32]>,
) -> Result<SignedTransaction, SuiError> {
    let pk = neko_hd::sui::public_key(sk);
    let derived = SuiAddress::from_public_key(&pk);
    if derived != data.sender {
        return Err(SuiError::BadReply(format!(
            "this key controls {derived}, not {}",
            data.sender
        )));
    }

    let digest = data.signing_digest();
    let sig = neko_hd::sui::sign(sk, &digest);
    verify(&pk, &digest, &sig)?;

    // flag, signature, public key - in that order, and the flag is what says
    // which curve to check it against.
    let mut serialized = Vec::with_capacity(1 + 64 + 32);
    serialized.push(crate::address::SCHEME_ED25519);
    serialized.extend_from_slice(&sig);
    serialized.extend_from_slice(&pk);

    let bytes = data.to_bytes();
    Ok(SignedTransaction {
        digest: transaction_digest(&bytes),
        data: bytes,
        signature: serialized,
    })
}

fn verify(pk: &[u8; 32], digest: &[u8; 32], sig: &[u8; 64]) -> Result<(), SuiError> {
    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let vk = VerifyingKey::from_bytes(pk)
        .map_err(|_| SuiError::BadReply("the derived public key is not on the curve".into()))?;
    vk.verify(digest, &Signature::from_bytes(sig))
        .map_err(|_| SuiError::BadReply("the signature this wallet just made does not verify".into()))
}

/// The name the chain knows this transaction by.
///
/// `blake2b256("TransactionData" as an intent-tagged struct)`, base58-encoded,
/// which is the form every Sui explorer shows.
pub fn transaction_digest(data: &[u8]) -> String {
    let mut m = INTENT_TRANSACTION_DATA.to_vec();
    m.extend_from_slice(data);
    bs58_encode(&crate::blake2b256(&m))
}

/// Base58 of raw bytes. Exposed for the round-trip test in `client`.
pub fn transaction_digest_of_bytes(b: &[u8]) -> String {
    bs58_encode(b)
}

/// Base58, which Sui uses for digests where it uses hex for addresses.
fn bs58_encode(b: &[u8]) -> String {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut digits: Vec<u8> = vec![0];
    for &byte in b {
        let mut carry = byte as u32;
        for d in digits.iter_mut() {
            carry += (*d as u32) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let leading = b.iter().take_while(|&&x| x == 0).count();
    let mut out = String::with_capacity(leading + digits.len());
    for _ in 0..leading {
        out.push('1');
    }
    for d in digits.iter().rev() {
        out.push(ALPHABET[*d as usize] as char);
    }
    out
}

// ── The two transfers this wallet builds ────────────────────────────────────

/// Send SUI itself.
///
/// The coin being spent is the gas coin, which is the pattern the chain's own
/// builder uses: split the amount off whatever is paying for the transaction,
/// then hand the piece over. The remainder stays where it was.
pub fn pay_sui(
    sender: SuiAddress,
    to: SuiAddress,
    amount: u64,
    gas: GasData,
) -> TransactionData {
    TransactionData {
        inputs: vec![
            CallArg::Pure(amount.to_le_bytes().to_vec()),
            CallArg::Pure(to.as_bytes().to_vec()),
        ],
        commands: vec![
            Command::SplitCoins {
                coin: Argument::GasCoin,
                amounts: vec![Argument::Input(0)],
            },
            Command::TransferObjects {
                objects: vec![Argument::NestedResult(0, 0)],
                address: Argument::Input(1),
            },
        ],
        sender,
        gas,
    }
}

/// Send a token.
///
/// A balance here is a set of objects rather than a number, so `coins` may
/// hold several and they are folded into the first before the amount is split
/// off. The gas coin is separate and always SUI: a token cannot pay for its
/// own transfer.
pub fn pay_token(
    sender: SuiAddress,
    to: SuiAddress,
    amount: u64,
    coins: &[ObjectRef],
    gas: GasData,
) -> Result<TransactionData, SuiError> {
    let (first, rest) = coins.split_first().ok_or(SuiError::NotEnoughCoins)?;

    let mut inputs = vec![CallArg::OwnedObject(*first)];
    for c in rest {
        inputs.push(CallArg::OwnedObject(*c));
    }
    let amount_ix = inputs.len() as u16;
    inputs.push(CallArg::Pure(amount.to_le_bytes().to_vec()));
    let to_ix = inputs.len() as u16;
    inputs.push(CallArg::Pure(to.as_bytes().to_vec()));

    let mut commands = Vec::new();
    if !rest.is_empty() {
        commands.push(Command::MergeCoins {
            destination: Argument::Input(0),
            sources: (1..=rest.len() as u16).map(Argument::Input).collect(),
        });
    }
    let split_at = commands.len() as u16;
    commands.push(Command::SplitCoins {
        coin: Argument::Input(0),
        amounts: vec![Argument::Input(amount_ix)],
    });
    commands.push(Command::TransferObjects {
        objects: vec![Argument::NestedResult(split_at, 0)],
        address: Argument::Input(to_ix),
    });

    Ok(TransactionData {
        inputs,
        commands,
        sender,
        gas,
    })
}

#[cfg(test)]
mod encoding {
    use super::*;

    const REFERENCE: &str = concat!(
        "000002000840420f00000000000020000000000000000000000000000000",
        "000000000000000000000000000000000202020001010000010103000000",
        "00010100ffd4f043057226453aeba59732d41c6093516f54823ebc3a16d1",
        "7f8a77d2f0ad01aa649b915f683af20595631a46826e99a2bb6e0b093b5d",
        "d4a4a6ccee89cdaf232abebd3a000000002090668da58c70bbde13fc25de",
        "770c787c489498250bcf74115759be7a4ab98473ffd4f043057226453aeb",
        "a59732d41c6093516f54823ebc3a16d17f8a77d2f0ad6400000000000000",
        "c0c62d000000000000",
    );

    fn addr(h: &str) -> SuiAddress {
        SuiAddress::parse(h).unwrap()
    }

    fn bytes32(h: &str) -> [u8; 32] {
        hex::decode(h).unwrap().try_into().unwrap()
    }

    fn reference_gas() -> GasData {
        GasData {
            payment: vec![ObjectRef {
                id: bytes32("aa649b915f683af20595631a46826e99a2bb6e0b093b5dd4a4a6ccee89cdaf23"),
                version: 985_513_514,
                digest: bytes32(
                    "90668da58c70bbde13fc25de770c787c489498250bcf74115759be7a4ab98473",
                ),
            }],
            owner: addr("0xffd4f043057226453aeba59732d41c6093516f54823ebc3a16d17f8a77d2f0ad"),
            price: 100,
            budget: 3_000_000,
        }
    }

    /// A SUI transfer, byte for byte as Sui's own node built it.
    ///
    /// The reference came from `unsafe_paySui` on mainnet: the node was asked
    /// to build this exact payment and returned 219 bytes, which were decoded
    /// field by field before this builder was written. Two independent
    /// encoders agreeing on every byte is what makes the file trustworthy -
    /// BCS carries no types, so a misplaced length would be a different
    /// transaction rather than a broken one.
    #[test]
    fn a_sui_transfer_matches_the_nodes_own_encoding() {
        let sender = addr("0xffd4f043057226453aeba59732d41c6093516f54823ebc3a16d17f8a77d2f0ad");
        let to = addr("0x0000000000000000000000000000000000000000000000000000000000000002");
        let data = pay_sui(sender, to, 1_000_000, reference_gas());
        assert_eq!(hex::encode(data.to_bytes()), REFERENCE);
    }

    /// The intent that turns a transaction into something only signable as a
    /// transaction.
    #[test]
    fn the_intent_names_what_is_being_signed() {
        // Scope 0 = TransactionData, version 0, application 0 = Sui.
        assert_eq!(INTENT_TRANSACTION_DATA, [0, 0, 0]);
        let sender = addr("0xffd4f043057226453aeba59732d41c6093516f54823ebc3a16d17f8a77d2f0ad");
        let to = addr("0x0000000000000000000000000000000000000000000000000000000000000002");
        let data = pay_sui(sender, to, 1_000_000, reference_gas());
        // The digest is over the intent *and* the bytes, so it is not the hash
        // of the transaction alone.
        assert_ne!(data.signing_digest(), crate::blake2b256(&data.to_bytes()));
    }

    /// Every field that decides where the money goes changes the bytes.
    #[test]
    fn every_field_is_covered_by_the_signature() {
        let sender = addr("0xffd4f043057226453aeba59732d41c6093516f54823ebc3a16d17f8a77d2f0ad");
        let to = addr("0x0000000000000000000000000000000000000000000000000000000000000002");
        let base = pay_sui(sender, to, 1_000_000, reference_gas());
        let d = base.signing_digest();

        let other = addr("0x0000000000000000000000000000000000000000000000000000000000000003");
        assert_ne!(pay_sui(sender, other, 1_000_000, reference_gas()).signing_digest(), d);
        assert_ne!(pay_sui(sender, to, 1_000_001, reference_gas()).signing_digest(), d);
        assert_ne!(pay_sui(other, to, 1_000_000, reference_gas()).signing_digest(), d);

        // The gas coin's version is signed over too. A stale one is rejected
        // rather than applied to the newer state, which is the property that
        // makes an object chain safe to build for.
        let mut g = reference_gas();
        g.payment[0].version += 1;
        assert_ne!(pay_sui(sender, to, 1_000_000, g).signing_digest(), d);

        let mut g = reference_gas();
        g.budget += 1;
        assert_ne!(pay_sui(sender, to, 1_000_000, g).signing_digest(), d);
        let mut g = reference_gas();
        g.price += 1;
        assert_ne!(pay_sui(sender, to, 1_000_000, g).signing_digest(), d);
    }

    /// A token transfer folds several coin objects together first.
    ///
    /// A balance on this chain is a set of objects, so sending one number may
    /// mean spending several of them. The command list grows by one when it
    /// does, and the split then refers to the merged coin.
    #[test]
    fn a_token_transfer_merges_only_when_it_has_to() {
        let sender = addr("0xffd4f043057226453aeba59732d41c6093516f54823ebc3a16d17f8a77d2f0ad");
        let to = addr("0x0000000000000000000000000000000000000000000000000000000000000002");
        let coin = |n: u8| ObjectRef {
            id: [n; 32],
            version: 1,
            digest: [n; 32],
        };

        let one = pay_token(sender, to, 5, &[coin(1)], reference_gas()).unwrap();
        assert_eq!(one.commands.len(), 2, "split then transfer");
        assert!(matches!(one.commands[0], Command::SplitCoins { .. }));

        let many = pay_token(sender, to, 5, &[coin(1), coin(2), coin(3)], reference_gas()).unwrap();
        assert_eq!(many.commands.len(), 3, "merge, split, transfer");
        assert!(matches!(many.commands[0], Command::MergeCoins { .. }));
        // And the transfer takes the *split's* result, not the merge's.
        match many.commands[2] {
            Command::TransferObjects { ref objects, .. } => {
                assert_eq!(objects[0], Argument::NestedResult(1, 0));
            }
            _ => panic!("the last command should hand the coin over"),
        }

        // No coins at all is refused rather than encoded.
        assert!(pay_token(sender, to, 5, &[], reference_gas()).is_err());
    }
}
