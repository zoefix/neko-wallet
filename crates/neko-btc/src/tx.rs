//! Building, serializing and signing a Bitcoin transaction.
//!
//! The shape of the problem is unlike the other three chains, and it is worth
//! being explicit about why:
//!
//! * **There is no balance and no nonce.** A wallet holds *outputs*, and
//!   spending means naming particular ones. Which ones are chosen changes the
//!   size of the transaction, which changes the fee, which changes how many
//!   need to be chosen.
//! * **The fee is not a field.** It is inputs minus outputs, implicit. An
//!   arithmetic slip does not produce an error; it produces a transaction that
//!   pays a miner the difference, and a well-known way to lose a fortune is to
//!   forget the change output entirely.
//! * **Change comes back to you.** Spending 0.1 from a 1.0 output means paying
//!   yourself 0.9 in the same transaction, or the rest is the fee.
//!
//! The signature covers the amounts and scripts being spent (BIP-143), which is
//! what makes those first two properties checkable at signing time rather than
//! after broadcast.

use neko_hd::BtcAddress;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::error::BtcError;
use crate::varint;

/// SIGHASH_ALL: the signature commits to every input and every output. The only
/// mode this wallet uses, because any other lets somebody else change part of
/// the transaction after it is signed.
pub const SIGHASH_ALL: u32 = 1;

/// Signals that the transaction opts out of replace-by-fee and is final.
/// `0xffffffff` also disables locktime, which is what we want.
pub const SEQUENCE_FINAL: u32 = 0xffff_ffff;

pub const VERSION: i32 = 2;

/// `dSHA256`, Bitcoin's hash for everything that is not an address.
pub fn double_sha256(data: &[u8]) -> [u8; 32] {
    let once = Sha256::digest(data);
    let twice = Sha256::digest(once);
    let mut out = [0u8; 32];
    out.copy_from_slice(&twice);
    out
}

/// Which output of which transaction.
///
/// `txid` is held in *serialization* order - the little-endian form that goes
/// on the wire. Explorers and people print it reversed, which is a display
/// concern and is handled in one place, at the edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OutPoint {
    pub txid: [u8; 32],
    pub vout: u32,
}

impl OutPoint {
    /// Parse the reversed hex form people and APIs use.
    pub fn from_display_txid(txid: &str, vout: u32) -> Result<Self, BtcError> {
        let mut raw =
            hex::decode(txid).map_err(|_| BtcError::BadReply(format!("{txid} is not hex")))?;
        if raw.len() != 32 {
            return Err(BtcError::BadReply(format!("{txid} is not 32 bytes")));
        }
        raw.reverse();
        let mut id = [0u8; 32];
        id.copy_from_slice(&raw);
        Ok(OutPoint { txid: id, vout })
    }

    pub fn display_txid(&self) -> String {
        let mut r = self.txid;
        r.reverse();
        hex::encode(r)
    }
}

/// An output this wallet can spend.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Utxo {
    pub outpoint: OutPoint,
    pub value: u64,
    /// The script that locks it. Kept rather than assumed, because it is what
    /// the signature commits to.
    pub script_pubkey: Vec<u8>,
    /// `None` while still in the mempool.
    pub block_height: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxIn {
    pub prev: OutPoint,
    pub sequence: u32,
    /// Empty for the segwit inputs this wallet spends; the signature lives in
    /// the witness instead, which is the whole point of segwit.
    pub script_sig: Vec<u8>,
    pub witness: Vec<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TxOut {
    pub value: u64,
    pub script_pubkey: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tx {
    pub version: i32,
    pub inputs: Vec<TxIn>,
    pub outputs: Vec<TxOut>,
    pub locktime: u32,
}

impl Tx {
    /// Serialization without the witness.
    ///
    /// This is what the txid is computed over - which is why segwit fixed
    /// transaction malleability: a signature sits outside the bytes that
    /// determine the transaction's identity.
    pub fn serialize_legacy(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&self.version.to_le_bytes());
        varint::encode(&mut out, self.inputs.len() as u64);
        for i in &self.inputs {
            out.extend_from_slice(&i.prev.txid);
            out.extend_from_slice(&i.prev.vout.to_le_bytes());
            varint::encode_bytes(&mut out, &i.script_sig);
            out.extend_from_slice(&i.sequence.to_le_bytes());
        }
        varint::encode(&mut out, self.outputs.len() as u64);
        for o in &self.outputs {
            out.extend_from_slice(&o.value.to_le_bytes());
            varint::encode_bytes(&mut out, &o.script_pubkey);
        }
        out.extend_from_slice(&self.locktime.to_le_bytes());
        out
    }

    /// Serialization with the witness, which is what gets broadcast.
    pub fn serialize(&self) -> Vec<u8> {
        if self.inputs.iter().all(|i| i.witness.is_empty()) {
            return self.serialize_legacy();
        }
        let mut out = Vec::new();
        out.extend_from_slice(&self.version.to_le_bytes());
        // The marker and flag are what tell a parser this is a segwit
        // transaction. A legacy parser reads the marker as "zero inputs".
        out.push(0x00);
        out.push(0x01);
        varint::encode(&mut out, self.inputs.len() as u64);
        for i in &self.inputs {
            out.extend_from_slice(&i.prev.txid);
            out.extend_from_slice(&i.prev.vout.to_le_bytes());
            varint::encode_bytes(&mut out, &i.script_sig);
            out.extend_from_slice(&i.sequence.to_le_bytes());
        }
        varint::encode(&mut out, self.outputs.len() as u64);
        for o in &self.outputs {
            out.extend_from_slice(&o.value.to_le_bytes());
            varint::encode_bytes(&mut out, &o.script_pubkey);
        }
        for i in &self.inputs {
            varint::encode(&mut out, i.witness.len() as u64);
            for item in &i.witness {
                varint::encode_bytes(&mut out, item);
            }
        }
        out.extend_from_slice(&self.locktime.to_le_bytes());
        out
    }

    /// The identifier, in the reversed hex everyone prints.
    pub fn txid(&self) -> String {
        let h = double_sha256(&self.serialize_legacy());
        let mut r = h;
        r.reverse();
        hex::encode(r)
    }

    /// Virtual size, the unit fees are quoted in.
    ///
    /// Witness bytes count a quarter, which is the discount segwit exists to
    /// give. Rounding up matches how a node computes it, so a fee rate
    /// multiplied by this is never a satoshi short of what the network wants.
    pub fn vsize(&self) -> usize {
        let base = self.serialize_legacy().len();
        let total = self.serialize().len();
        let weight = base * 3 + total;
        weight.div_ceil(4)
    }

    /// What this transaction pays in fees, given what it spends.
    ///
    /// `None` when the inputs do not cover the outputs, which is a transaction
    /// that cannot exist rather than one with a negative fee.
    pub fn fee(&self, input_total: u64) -> Option<u64> {
        let out: u64 = self.outputs.iter().map(|o| o.value).sum();
        input_total.checked_sub(out)
    }
}

// ── BIP-143 ────────────────────────────────────────────────────────────────

/// The hash a segwit v0 signature is taken over.
///
/// Unlike the legacy algorithm this commits to the *value* being spent, which
/// is why a hardware wallet can be told what a transaction costs and be sure.
/// It is also why an input's amount is required here: get it wrong and the
/// signature is simply invalid, rather than valid for something unintended.
pub fn sighash_p2wpkh(
    tx: &Tx,
    index: usize,
    key_hash: &[u8; 20],
    value: u64,
    sighash_type: u32,
) -> [u8; 32] {
    let mut prevouts = Vec::new();
    let mut sequences = Vec::new();
    for i in &tx.inputs {
        prevouts.extend_from_slice(&i.prev.txid);
        prevouts.extend_from_slice(&i.prev.vout.to_le_bytes());
        sequences.extend_from_slice(&i.sequence.to_le_bytes());
    }
    let mut outputs = Vec::new();
    for o in &tx.outputs {
        outputs.extend_from_slice(&o.value.to_le_bytes());
        varint::encode_bytes(&mut outputs, &o.script_pubkey);
    }

    let mut pre = Vec::with_capacity(156);
    pre.extend_from_slice(&tx.version.to_le_bytes());
    pre.extend_from_slice(&double_sha256(&prevouts));
    pre.extend_from_slice(&double_sha256(&sequences));
    pre.extend_from_slice(&tx.inputs[index].prev.txid);
    pre.extend_from_slice(&tx.inputs[index].prev.vout.to_le_bytes());
    // The scriptCode for P2WPKH is the *P2PKH* script for the same key hash.
    // Not the witness program - that substitution is specified, and using the
    // witness program instead produces a signature no node will accept.
    pre.extend_from_slice(&[0x19, 0x76, 0xa9, 0x14]);
    pre.extend_from_slice(key_hash);
    pre.extend_from_slice(&[0x88, 0xac]);
    pre.extend_from_slice(&value.to_le_bytes());
    pre.extend_from_slice(&tx.inputs[index].sequence.to_le_bytes());
    pre.extend_from_slice(&double_sha256(&outputs));
    pre.extend_from_slice(&tx.locktime.to_le_bytes());
    pre.extend_from_slice(&sighash_type.to_le_bytes());

    double_sha256(&pre)
}

/// Sign every input, all of which must be ours and P2WPKH.
///
/// The signature is checked against the public key before it is attached: a
/// transaction with one bad signature is rejected by the network *after* it has
/// been broadcast, and by then the UTXOs look spent to anything watching.
pub fn sign_p2wpkh(
    tx: &mut Tx,
    spending: &[Utxo],
    key: &Zeroizing<[u8; 32]>,
) -> Result<(), BtcError> {
    use k256::ecdsa::signature::hazmat::PrehashVerifier;

    let pubkey = neko_hd::bitcoin::compressed_public_key(key)?;
    let ours = BtcAddress::p2wpkh_from_public_key(&pubkey)?;
    let key_hash = ours.witness_key_hash().ok_or(BtcError::UnsignableInput)?;
    let signing = k256::ecdsa::SigningKey::from_slice(key.as_slice())
        .map_err(|_| BtcError::UnsignableInput)?;
    let verifying = *signing.verifying_key();

    for (i, utxo) in spending.iter().enumerate() {
        // Every input has to be one we can sign for. Anything else would be
        // signed with the wrong key and rejected, having already told the
        // network which coins we intended to move.
        if utxo.script_pubkey != ours.script_pubkey() {
            return Err(BtcError::UnsignableInput);
        }
        let hash = sighash_p2wpkh(tx, i, &key_hash, utxo.value, SIGHASH_ALL);

        let (sig, _) = signing
            .sign_prehash_recoverable(&hash)
            .map_err(|_| BtcError::UnsignableInput)?;
        // Low-S. Bitcoin rejects the high-S half of each signature pair as
        // non-standard, and k256 does not normalise on its own.
        let sig = sig.normalize_s().unwrap_or(sig);
        verifying
            .verify_prehash(&hash, &sig)
            .map_err(|_| BtcError::UnsignableInput)?;

        let mut der = sig.to_der().as_bytes().to_vec();
        der.push(SIGHASH_ALL as u8);
        tx.inputs[i].witness = vec![der, pubkey.to_vec()];
    }
    Ok(())
}

/// An output paying `to`.
pub fn output(to: &BtcAddress, value: u64) -> TxOut {
    TxOut {
        value,
        script_pubkey: to.script_pubkey(),
    }
}

/// An unsigned input spending `utxo`.
pub fn input(utxo: &Utxo) -> TxIn {
    TxIn {
        prev: utxo.outpoint,
        sequence: SEQUENCE_FINAL,
        script_sig: Vec::new(),
        witness: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(s: &str) -> Vec<u8> {
        hex::decode(s).unwrap()
    }
    fn h32(s: &str) -> [u8; 32] {
        h(s).try_into().unwrap()
    }
    fn h20(s: &str) -> [u8; 20] {
        h(s).try_into().unwrap()
    }

    /// BIP-143's own "Native P2WPKH" vector.
    ///
    /// This is the test that matters. A sighash computed slightly wrong does
    /// not produce an error - it produces a signature the network rejects,
    /// after the transaction has been broadcast and the coins look spent to
    /// anything watching. Matching a published 32-byte digest cannot happen by
    /// accident.
    #[test]
    fn matches_the_bip143_vector() {
        let tx = Tx {
            version: 1,
            inputs: vec![
                TxIn {
                    prev: OutPoint {
                        txid: h32(
                            "fff7f7881a8099afa6940d42d1e7f6362bec38171ea3edf433541db4e4ad969f",
                        ),
                        vout: 0,
                    },
                    sequence: 0xffff_ffee,
                    script_sig: Vec::new(),
                    witness: Vec::new(),
                },
                TxIn {
                    prev: OutPoint {
                        txid: h32(
                            "ef51e1b804cc89d182d279655c3aa89e815b1b309fe287d9b2b55d57b90ec68a",
                        ),
                        vout: 1,
                    },
                    sequence: 0xffff_ffff,
                    script_sig: Vec::new(),
                    witness: Vec::new(),
                },
            ],
            outputs: vec![
                TxOut {
                    value: 112_340_000, // 1.1234 BTC
                    script_pubkey: h("76a9148280b37df378db99f66f85c95a783a76ac7a6d5988ac"),
                },
                TxOut {
                    value: 223_450_000, // 2.2345 BTC
                    script_pubkey: h("76a9143bde42dbee7e4dbe6a21b2d50ce2f0167faa815988ac"),
                },
            ],
            locktime: 0x0000_0011,
        };

        // The unsigned transaction the BIP prints.
        assert_eq!(
            hex::encode(tx.serialize_legacy()),
            "0100000002fff7f7881a8099afa6940d42d1e7f6362bec38171ea3edf433541db4e4ad969f0000000000eeffffffef51e1b804cc89d182d279655c3aa89e815b1b309fe287d9b2b55d57b90ec68a0100000000ffffffff02202cb206000000001976a9148280b37df378db99f66f85c95a783a76ac7a6d5988ac9093510d000000001976a9143bde42dbee7e4dbe6a21b2d50ce2f0167faa815988ac11000000"
        );

        // The second input is the P2WPKH one: 6 BTC behind
        // 00141d0f172a0ecb48aee1be1f2687d2963ae33f71a1.
        let got = sighash_p2wpkh(
            &tx,
            1,
            &h20("1d0f172a0ecb48aee1be1f2687d2963ae33f71a1"),
            600_000_000,
            SIGHASH_ALL,
        );
        assert_eq!(
            hex::encode(got),
            "c37af31116d1b27caf68aae9e3ac82f1477929014d5b917657d0eb49478cb670",
            "the BIP-143 sighash does not match"
        );
    }

    /// A signature has to verify against the key that made it, and the
    /// transaction has to carry the witness that proves it.
    #[test]
    fn a_signed_input_verifies() {
        let key = Zeroizing::new([0x42u8; 32]);
        let pubkey = neko_hd::bitcoin::compressed_public_key(&key).unwrap();
        let mine = BtcAddress::p2wpkh_from_public_key(&pubkey).unwrap();
        let to = BtcAddress::parse("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").unwrap();

        let utxo = Utxo {
            outpoint: OutPoint {
                txid: [7u8; 32],
                vout: 0,
            },
            value: 100_000,
            script_pubkey: mine.script_pubkey(),
            block_height: Some(800_000),
        };
        let mut t = Tx {
            version: VERSION,
            inputs: vec![input(&utxo)],
            outputs: vec![output(&to, 90_000)],
            locktime: 0,
        };
        sign_p2wpkh(&mut t, std::slice::from_ref(&utxo), &key).unwrap();

        assert_eq!(
            t.inputs[0].witness.len(),
            2,
            "witness is signature + pubkey"
        );
        assert_eq!(t.inputs[0].witness[1], pubkey, "the wrong public key");
        assert!(
            t.inputs[0].script_sig.is_empty(),
            "segwit puts nothing here"
        );

        // The signature must verify against the sighash it claims to cover.
        use k256::ecdsa::signature::hazmat::PrehashVerifier;
        let hash = sighash_p2wpkh(
            &t,
            0,
            &mine.witness_key_hash().unwrap(),
            utxo.value,
            SIGHASH_ALL,
        );
        let der = &t.inputs[0].witness[0];
        let sig = k256::ecdsa::Signature::from_der(&der[..der.len() - 1]).unwrap();
        let vk = k256::ecdsa::SigningKey::from_slice(key.as_slice())
            .unwrap()
            .verifying_key()
            .to_owned();
        vk.verify_prehash(&hash, &sig)
            .expect("signature does not verify");
        assert_eq!(*der.last().unwrap(), SIGHASH_ALL as u8);
    }

    /// Signatures must be low-S. Bitcoin treats the high half as non-standard
    /// and nodes will not relay it, so a transaction with one simply never
    /// propagates - a failure with no error message anywhere.
    #[test]
    fn signatures_are_low_s() {
        let to = BtcAddress::parse("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").unwrap();
        // Enough different keys and amounts that a high-S would come up.
        for n in 0..40u8 {
            let key = Zeroizing::new([n.wrapping_add(1); 32]);
            let pubkey = neko_hd::bitcoin::compressed_public_key(&key).unwrap();
            let mine = BtcAddress::p2wpkh_from_public_key(&pubkey).unwrap();
            let utxo = Utxo {
                outpoint: OutPoint {
                    txid: [n; 32],
                    vout: n as u32,
                },
                value: 100_000,
                script_pubkey: mine.script_pubkey(),
                block_height: None,
            };
            let mut t = Tx {
                version: VERSION,
                inputs: vec![input(&utxo)],
                outputs: vec![output(&to, 90_000)],
                locktime: 0,
            };
            sign_p2wpkh(&mut t, &[utxo], &key).unwrap();
            let der = &t.inputs[0].witness[0];
            let sig = k256::ecdsa::Signature::from_der(&der[..der.len() - 1]).unwrap();
            assert!(
                sig.normalize_s().is_none(),
                "key {n} produced a high-S signature"
            );
        }
    }

    /// An input this wallet does not own must be refused before signing, not
    /// signed with the wrong key and discovered by the network.
    #[test]
    fn an_input_we_do_not_own_is_refused() {
        let key = Zeroizing::new([9u8; 32]);
        let to = BtcAddress::parse("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").unwrap();
        let theirs = Utxo {
            outpoint: OutPoint {
                txid: [1u8; 32],
                vout: 0,
            },
            value: 100_000,
            // Somebody else's script.
            script_pubkey: to.script_pubkey(),
            block_height: None,
        };
        let mut t = Tx {
            version: VERSION,
            inputs: vec![input(&theirs)],
            outputs: vec![output(&to, 90_000)],
            locktime: 0,
        };
        assert!(matches!(
            sign_p2wpkh(&mut t, &[theirs], &key),
            Err(BtcError::UnsignableInput)
        ));
    }

    /// The txid is computed over the transaction *without* its witness, which
    /// is why signing does not change it. A wallet that showed a different id
    /// after signing would be telling the user to look for the wrong thing.
    #[test]
    fn signing_does_not_change_the_txid() {
        let key = Zeroizing::new([11u8; 32]);
        let pubkey = neko_hd::bitcoin::compressed_public_key(&key).unwrap();
        let mine = BtcAddress::p2wpkh_from_public_key(&pubkey).unwrap();
        let to = BtcAddress::parse("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").unwrap();
        let utxo = Utxo {
            outpoint: OutPoint {
                txid: [5u8; 32],
                vout: 1,
            },
            value: 250_000,
            script_pubkey: mine.script_pubkey(),
            block_height: Some(1),
        };
        let mut t = Tx {
            version: VERSION,
            inputs: vec![input(&utxo)],
            outputs: vec![output(&to, 200_000)],
            locktime: 0,
        };
        let before = t.txid();
        sign_p2wpkh(&mut t, &[utxo], &key).unwrap();
        assert_eq!(t.txid(), before);
        // ...but the broadcast bytes do grow, and are no longer the legacy form.
        assert!(t.serialize().len() > t.serialize_legacy().len());
        assert_eq!(t.serialize()[4], 0x00, "segwit marker");
        assert_eq!(t.serialize()[5], 0x01, "segwit flag");
    }

    /// Weight, not bytes. A one-in one-out P2WPKH spend is about 110 vB, and
    /// the fee is that times a rate - so an error here is an error in what
    /// every transfer costs.
    #[test]
    fn vsize_gives_the_witness_its_discount() {
        let key = Zeroizing::new([13u8; 32]);
        let pubkey = neko_hd::bitcoin::compressed_public_key(&key).unwrap();
        let mine = BtcAddress::p2wpkh_from_public_key(&pubkey).unwrap();
        let to = BtcAddress::parse("bc1qw508d6qejxtdg4y5r3zarvary0c5xw7kv8f3t4").unwrap();
        let utxo = Utxo {
            outpoint: OutPoint {
                txid: [2u8; 32],
                vout: 0,
            },
            value: 100_000,
            script_pubkey: mine.script_pubkey(),
            block_height: None,
        };
        let mut t = Tx {
            version: VERSION,
            inputs: vec![input(&utxo)],
            outputs: vec![output(&to, 90_000)],
            locktime: 0,
        };
        sign_p2wpkh(&mut t, &[utxo], &key).unwrap();

        let v = t.vsize();
        assert!(
            (109..=112).contains(&v),
            "one-in one-out came out at {v} vB"
        );
        // The discount is real: the raw byte count is much larger.
        assert!(t.serialize().len() > v + 30);
        assert_eq!(t.fee(100_000), Some(10_000));
        assert_eq!(t.fee(50_000), None, "inputs below outputs is not a fee");
    }
}

/// Two confirmed mainnet transactions, reconstructed from their own data.
///
/// Bitcoin has no `simulateTransaction` to check bytes against, so this is the
/// next best thing: take transactions the network already accepted, rebuild
/// them from their fields, and require the serialization to come out
/// byte-for-byte identical - witness layout, segwit marker, txid and virtual
/// size included. Anything wrong in the encoding changes the hex.
#[cfg(test)]
mod mainnet {
    use super::*;

    fn h(s: &str) -> Vec<u8> {
        hex::decode(s).unwrap()
    }
    fn prev(txid: &str, vout: u32) -> OutPoint {
        OutPoint::from_display_txid(txid, vout).unwrap()
    }
    fn witness_in(txid: &str, vout: u32, sig: &str, pk: &str) -> TxIn {
        TxIn {
            prev: prev(txid, vout),
            sequence: SEQUENCE_FINAL,
            script_sig: Vec::new(),
            witness: vec![h(sig), h(pk)],
        }
    }

    /// One P2WPKH input, one P2PKH output. Exactly the shape this wallet builds
    /// when paying somebody with an older address.
    #[test]
    fn one_in_one_out() {
        let t = Tx {
            version: 2,
            inputs: vec![witness_in(
                "dbf6eb40a0ec62dec137a5815eae24a026355f30f833814ef19937bb97b51662",
                2,
                "304402203cb3a7644dcb2e47c6f130878bbf5aa5528f3b856ea4701704977f99ba64b9400220111454ceaa1720b95c2b89523e4ddff388dbbb38c5e9a60d5edddeaa9a6816ae01",
                "02275b6bb943ffababecb6883357b987831288cbbdbbb107771d059c169f986bcc",
            )],
            outputs: vec![TxOut {
                value: 2_460_000,
                script_pubkey: h("76a914013b9c29b21d073977918487deaba146fd6071f788ac"),
            }],
            locktime: 0,
        };
        assert_eq!(
            hex::encode(t.serialize()),
            "020000000001016216b597bb3799f14e8133f8305f3526a024ae5e81a537c1de62eca040ebf6db0200000000ffffffff0160892500000000001976a914013b9c29b21d073977918487deaba146fd6071f788ac0247304402203cb3a7644dcb2e47c6f130878bbf5aa5528f3b856ea4701704977f99ba64b9400220111454ceaa1720b95c2b89523e4ddff388dbbb38c5e9a60d5edddeaa9a6816ae012102275b6bb943ffababecb6883357b987831288cbbdbbb107771d059c169f986bcc00000000"
        );
        assert_eq!(
            t.txid(),
            "53a78836f5c72cfe41d74011f9017d9d9594d9070040345de35f42258a12aa5a"
        );
        // The network recorded 194 bytes and weight 449. Virtual size is the
        // weight rounded *up*: Bitcoin Core computes (weight + 3) / 4, so 449
        // is 113 rather than 112. Rounding down would price every transfer a
        // shade under what the network charges for it.
        assert_eq!(t.serialize().len(), 194);
        assert_eq!(t.vsize(), 113);
        assert_eq!(t.vsize() * 4, 452, "weight 449 rounds up to 452");
        assert_eq!(t.fee(2_460_731), Some(731));
    }

    /// Two inputs, one output, version 1. Pins the input count varint and the
    /// witness stack repeating per input, which a single-input case cannot.
    #[test]
    fn two_in_one_out() {
        let pk = "03cfe6464db9e428906f4bfc2ef7f964b64721b42810f45c9060339757592459d4";
        let t = Tx {
            version: 1,
            inputs: vec![
                witness_in(
                    "e9734f3fe767a0d7e264b6fa0033711062e6001a4d3beb385e53089ee1e36093",
                    0,
                    "304402202f0798dbb7fb58bdc3ee36b02864014cd800af9343a90beb9c61b437ed3b1e5a02206ae591d456e60440490d36d0da134647078c1f9a9a4f9fc1cd35452c4409e9da01",
                    pk,
                ),
                witness_in(
                    "6ca1e2033abb12a8297267b8d5a3df7a14c4bde50d10154653c8f3c94ed38e1f",
                    0,
                    "304402200f0917da674f234a1477d5ce6dafc3b3b0ec12ee90a3ee00bd1fb004933e7f53022026a0620361068e4e3e70212ff14fc8e49d3d380cd3c9b98a4c2987f164e19c7a01",
                    pk,
                ),
            ],
            outputs: vec![TxOut {
                value: 51_912,
                script_pubkey: h("0014f16289533bfab85b8500de8913491b4f35a2988f"),
            }],
            locktime: 0,
        };
        assert_eq!(
            hex::encode(t.serialize()),
            "010000000001029360e3e19e08535e38eb3b4d1a00e66210713300fab664e2d7a067e73f4f73e90000000000ffffffff1f8ed34ec9f3c8534615100de5bdc4147adfa3d5b8677229a812bb3a03e2a16c0000000000ffffffff01c8ca000000000000160014f16289533bfab85b8500de8913491b4f35a2988f0247304402202f0798dbb7fb58bdc3ee36b02864014cd800af9343a90beb9c61b437ed3b1e5a02206ae591d456e60440490d36d0da134647078c1f9a9a4f9fc1cd35452c4409e9da012103cfe6464db9e428906f4bfc2ef7f964b64721b42810f45c9060339757592459d40247304402200f0917da674f234a1477d5ce6dafc3b3b0ec12ee90a3ee00bd1fb004933e7f53022026a0620361068e4e3e70212ff14fc8e49d3d380cd3c9b98a4c2987f164e19c7a012103cfe6464db9e428906f4bfc2ef7f964b64721b42810f45c9060339757592459d400000000"
        );
        assert_eq!(
            t.txid(),
            "1d990bda8e8d1a88bc43839a53c5934d91375b8dde55b4c66bc330d95a70d9b2"
        );
        // Weight 708 divides evenly, so this one is 177 either way - which is
        // why the case above is the one that pins the rounding.
        assert_eq!(t.serialize().len(), 339);
        assert_eq!(t.vsize(), 177);
        assert_eq!(t.fee(43_007 + 12_281), Some(3_376));
    }

    /// The estimate that decides the fee has to match what the transaction
    /// actually weighs, or every transfer is priced against a size it does not
    /// have.
    #[test]
    fn the_size_estimate_matches_the_real_thing() {
        // The two-in one-out transaction above measured 177 vB; the estimate
        // says 178. One byte high, which is the safe direction.
        let p2wpkh = h("0014f16289533bfab85b8500de8913491b4f35a2988f");
        assert_eq!(crate::coins::estimate_vbytes(2, &[&p2wpkh]), 178);
        // The one-in one-out to a P2PKH measured 113 vB, and the estimate is
        // exact.
        let p2pkh = h("76a914013b9c29b21d073977918487deaba146fd6071f788ac");
        assert_eq!(crate::coins::estimate_vbytes(1, &[&p2pkh]), 113);
        // Estimating a byte high is the safe direction: the fee is paid at a
        // rate slightly above what was asked, never below what will relay.
    }
}
