//! Building and signing EVM transactions.
//!
//! Two formats, chosen per chain rather than per taste.
//!
//! **Type 0 (legacy)** commits to a single gas price and carries the chain id
//! inside `v`, EIP-155 style. BNB Chain uses it: gas there is cheap and stable
//! enough that a priority fee buys nothing, and it is the format every node and
//! explorer has understood since the beginning.
//!
//! **Type 2 (EIP-1559)** names a ceiling and a tip. Only the base fee plus the
//! tip is charged; the rest of the ceiling is refunded. Ethereum uses it,
//! because there the base fee moves between blocks and a single price is a
//! choice between never confirming and overpaying. Its signature is a bare
//! parity bit rather than an EIP-155 `v`, and the chain id is a field of its
//! own - so the two formats cannot be confused for one another on the wire.
//!
//! Like the TRON side, nothing is fetched pre-built: the node supplies a
//! nonce, a gas price and an estimate, and the bytes that get signed are
//! assembled here. A node cannot hand back a transaction paying itself and
//! have it signed.

use k256::ecdsa::{RecoveryId, SigningKey};
use neko_hd::EvmAddress;

use crate::error::EvmError;
use crate::rlp;

/// The envelope byte that marks a typed transaction as EIP-1559.
pub const TYPE_EIP1559: u8 = 0x02;

/// How a transaction pays for its gas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fees {
    /// Type 0. One price, paid in full whatever the chain charges.
    Legacy { gas_price: u128 },
    /// Type 2. `max_fee_per_gas` is a ceiling, not a price: what is charged is
    /// the base fee plus the tip, and the difference is refunded.
    Eip1559 {
        max_fee_per_gas: u128,
        max_priority_fee_per_gas: u128,
        /// What the chain reported when this was quoted. Not part of the
        /// transaction - it is here so the screen can show what the transfer
        /// will *actually* cost rather than the ceiling, which is a much
        /// larger and much less useful number.
        base_fee: u128,
    },
}

impl Fees {
    /// The most this can cost per unit of gas. What a balance must cover.
    pub fn max_per_gas(&self) -> u128 {
        match self {
            Fees::Legacy { gas_price } => *gas_price,
            Fees::Eip1559 {
                max_fee_per_gas, ..
            } => *max_fee_per_gas,
        }
    }

    /// What it is expected to cost per unit of gas.
    ///
    /// For a legacy transaction those are the same number. For a type 2 one
    /// they are not, and showing the ceiling as though it were the price would
    /// tell somebody a transfer costs twice what it does.
    pub fn expected_per_gas(&self) -> u128 {
        match self {
            Fees::Legacy { gas_price } => *gas_price,
            Fees::Eip1559 {
                max_fee_per_gas,
                max_priority_fee_per_gas,
                base_fee,
            } => base_fee
                .saturating_add(*max_priority_fee_per_gas)
                .min(*max_fee_per_gas),
        }
    }

    pub fn is_eip1559(&self) -> bool {
        matches!(self, Fees::Eip1559 { .. })
    }
}

/// What the chain must tell us before a transaction can be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxParams {
    pub nonce: u64,
    pub gas_limit: u64,
    pub chain_id: u64,
    pub fees: Fees,
}

/// An unsigned transaction.
#[derive(Debug, Clone)]
pub struct Tx {
    pub to: EvmAddress,
    /// Wei for a plain transfer; zero for a token call, whose amount lives in
    /// the calldata instead.
    pub value: u128,
    pub data: Vec<u8>,
    pub params: TxParams,
}

/// A signed transaction, ready to broadcast.
#[derive(Debug, Clone)]
pub struct SignedTx {
    pub raw: Vec<u8>,
    /// `keccak256` of the signed bytes - what an explorer calls the hash.
    pub hash: [u8; 32],
}

impl SignedTx {
    pub fn hash_hex(&self) -> String {
        format!("0x{}", hex::encode(self.hash))
    }
    pub fn raw_hex(&self) -> String {
        format!("0x{}", hex::encode(&self.raw))
    }
}

impl Tx {
    /// The bytes that are hashed and signed.
    ///
    /// The chain id and the two empty fields at the end are EIP-155. Without
    /// them the signature is valid on *every* EVM chain, so anyone who sees a
    /// BNB Chain transaction could replay it on another chain where the same
    /// address holds funds.
    pub fn signing_payload(&self) -> Vec<u8> {
        match self.params.fees {
            Fees::Legacy { gas_price } => {
                let mut body = Vec::new();
                rlp::uint(&mut body, self.params.nonce as u128);
                rlp::uint(&mut body, gas_price);
                rlp::uint(&mut body, self.params.gas_limit as u128);
                rlp::bytes(&mut body, self.to.as_bytes());
                rlp::uint(&mut body, self.value);
                rlp::bytes(&mut body, &self.data);
                rlp::uint(&mut body, self.params.chain_id as u128);
                rlp::uint(&mut body, 0);
                rlp::uint(&mut body, 0);

                let mut out = Vec::new();
                rlp::list(&mut out, &body);
                out
            }
            Fees::Eip1559 {
                max_fee_per_gas,
                max_priority_fee_per_gas,
                ..
            } => {
                let mut out = vec![TYPE_EIP1559];
                let mut body = Vec::new();
                rlp::uint(&mut body, self.params.chain_id as u128);
                rlp::uint(&mut body, self.params.nonce as u128);
                rlp::uint(&mut body, max_priority_fee_per_gas);
                rlp::uint(&mut body, max_fee_per_gas);
                rlp::uint(&mut body, self.params.gas_limit as u128);
                rlp::bytes(&mut body, self.to.as_bytes());
                rlp::uint(&mut body, self.value);
                rlp::bytes(&mut body, &self.data);
                // An empty access list. Present because the field is part of
                // the format, and omitting it shifts the signature onto a
                // different transaction.
                rlp::list(&mut body, &[]);
                rlp::list(&mut out, &body);
                out
            }
        }
    }

    /// Sign, then check the signature recovers to the address that is paying.
    ///
    /// The self-check costs almost nothing and catches the one failure that is
    /// otherwise silent and total: a recovery id computed wrongly produces a
    /// signature belonging to a different address, which the network accepts
    /// as a perfectly valid transaction from an account with no funds - or,
    /// worse, from one that has them.
    pub fn sign(&self, key: &[u8; 32]) -> Result<SignedTx, EvmError> {
        let signing = SigningKey::from_bytes(key.into()).map_err(|_| EvmError::Signing)?;
        let hash = keccak(&self.signing_payload());
        let (sig, recid) = signing
            .sign_prehash_recoverable(&hash)
            .map_err(|_| EvmError::Signing)?;

        let expected = neko_hd::derive::evm_address_from_private_key(key)?;
        verify_recovers(&hash, &sig, recid, expected)?;

        // r and s are trimmed of leading zeros in both formats: RLP integers
        // are minimal, and a fixed 32-byte encoding would hash to something the
        // network rejects.
        let r = trim(&sig.r().to_bytes());
        let sv = trim(&sig.s().to_bytes());

        let raw = self.encode_signed(&r, &sv, recid.to_byte());
        let hash = keccak(&raw);
        Ok(SignedTx { raw, hash })
    }
}

impl Tx {
    /// The broadcast bytes, given a signature.
    ///
    /// Split out from `sign` so it can be checked against transactions the
    /// network has already accepted: feed one its own `r`, `s` and parity and
    /// the hash that comes out has to be the hash it is known by.
    ///
    /// `r` and `s` are trimmed of leading zeros - RLP integers are minimal, and
    /// a fixed 32-byte encoding hashes to something the network rejects.
    pub fn encode_signed(&self, r: &[u8], s: &[u8], parity: u8) -> Vec<u8> {
        match self.params.fees {
            Fees::Legacy { gas_price } => {
                // EIP-155: v carries the chain id as well as the recovery bit.
                let v = parity as u128 + 35 + 2 * self.params.chain_id as u128;
                let mut body = Vec::new();
                rlp::uint(&mut body, self.params.nonce as u128);
                rlp::uint(&mut body, gas_price);
                rlp::uint(&mut body, self.params.gas_limit as u128);
                rlp::bytes(&mut body, self.to.as_bytes());
                rlp::uint(&mut body, self.value);
                rlp::bytes(&mut body, &self.data);
                rlp::uint(&mut body, v);
                rlp::bytes(&mut body, r);
                rlp::bytes(&mut body, s);
                let mut raw = Vec::new();
                rlp::list(&mut raw, &body);
                raw
            }
            Fees::Eip1559 {
                max_fee_per_gas,
                max_priority_fee_per_gas,
                ..
            } => {
                // Type 2 carries the chain id as a field, so the signature is a
                // bare parity bit. Writing an EIP-155 `v` here would be a number
                // no node can recover a key from.
                let mut body = Vec::new();
                rlp::uint(&mut body, self.params.chain_id as u128);
                rlp::uint(&mut body, self.params.nonce as u128);
                rlp::uint(&mut body, max_priority_fee_per_gas);
                rlp::uint(&mut body, max_fee_per_gas);
                rlp::uint(&mut body, self.params.gas_limit as u128);
                rlp::bytes(&mut body, self.to.as_bytes());
                rlp::uint(&mut body, self.value);
                rlp::bytes(&mut body, &self.data);
                rlp::list(&mut body, &[]);
                rlp::uint(&mut body, parity as u128);
                rlp::bytes(&mut body, r);
                rlp::bytes(&mut body, s);
                let mut raw = vec![TYPE_EIP1559];
                rlp::list(&mut raw, &body);
                raw
            }
        }
    }
}

fn verify_recovers(
    hash: &[u8; 32],
    sig: &k256::ecdsa::Signature,
    recid: RecoveryId,
    expected: EvmAddress,
) -> Result<(), EvmError> {
    let key = k256::ecdsa::VerifyingKey::recover_from_prehash(hash, sig, recid)
        .map_err(|_| EvmError::Signing)?;
    let point = k256::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(
        &k256::PublicKey::from(&key),
        false,
    );
    let got = EvmAddress::from_public_key(point.as_bytes())?;
    if got != expected {
        return Err(EvmError::Signing);
    }
    Ok(())
}

fn trim(b: &[u8]) -> Vec<u8> {
    let first = b.iter().position(|x| *x != 0).unwrap_or(b.len());
    b[first..].to_vec()
}

pub fn keccak(data: &[u8]) -> [u8; 32] {
    use sha3::{Digest, Keccak256};
    let mut h = Keccak256::new();
    h.update(data);
    h.finalize().into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worked example from EIP-155 itself. Every byte of the signed
    /// transaction is specified there, so this checks the RLP layout, the
    /// minimal integer encoding, the signing hash, the deterministic nonce,
    /// and the `v = recid + 35 + 2 * chain_id` rule at once. If any one of
    /// them were wrong the output would differ and the network would reject
    /// the transaction without saying why.
    #[test]
    fn matches_the_eip155_example() {
        let key: [u8; 32] =
            hex::decode("4646464646464646464646464646464646464646464646464646464646464646")
                .unwrap()
                .try_into()
                .unwrap();
        let tx = Tx {
            to: EvmAddress::parse("0x3535353535353535353535353535353535353535").unwrap(),
            value: 1_000_000_000_000_000_000,
            data: Vec::new(),
            params: TxParams {
                nonce: 9,
                fees: Fees::Legacy {
                    gas_price: 20_000_000_000,
                },
                gas_limit: 21_000,
                chain_id: 1,
            },
        };

        // The EIP states the sender for this key. Checking it first means a
        // later mismatch cannot be blamed on key handling.
        assert_eq!(
            neko_hd::derive::evm_address_from_private_key(&key)
                .unwrap()
                .to_string(),
            "0x9d8A62f656a8d1615C1294fd71e9CFb3E4855A4F",
            "the private key does not derive the sender the EIP names"
        );
        assert_eq!(
            hex::encode(keccak(&tx.signing_payload())),
            "daf5a779ae972f972197303d7b574746c7ef83eadac0f2791ad23db92e4c8e53",
            "signing payload differs from the EIP-155 example"
        );

        let signed = tx.sign(&key).unwrap();
        assert_eq!(
            signed.raw_hex(),
            "0xf86c098504a817c800825208943535353535353535353535353535353535353535880de0b6b3a7640000\
             8025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f76\
             1aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83"
                .replace(['\n', ' '], "")
                .as_str(),
            "signed transaction differs from the EIP-155 example"
        );
    }

    /// Signing is deterministic (RFC 6979), so the same inputs must always
    /// produce the same bytes. A transaction that varies between runs cannot
    /// be reasoned about or reproduced from a bug report.
    #[test]
    fn signing_is_deterministic() {
        let key = [0x11u8; 32];
        let tx = Tx {
            to: EvmAddress::parse("0x3535353535353535353535353535353535353535").unwrap(),
            value: 1,
            data: vec![1, 2, 3],
            params: TxParams {
                nonce: 1,
                fees: Fees::Legacy {
                    gas_price: 3_000_000_000,
                },
                gas_limit: 21_000,
                chain_id: crate::BSC.chain_id,
            },
        };
        assert_eq!(tx.sign(&key).unwrap().raw, tx.sign(&key).unwrap().raw);
    }

    /// The chain id must reach the signature. Without it the same transaction
    /// is valid on every EVM chain, and anyone can replay it wherever that
    /// address also holds funds.
    #[test]
    fn the_chain_id_changes_the_signature() {
        let key = [0x22u8; 32];
        let base = Tx {
            to: EvmAddress::parse("0x3535353535353535353535353535353535353535").unwrap(),
            value: 5,
            data: Vec::new(),
            params: TxParams {
                nonce: 0,
                fees: Fees::Legacy {
                    gas_price: 1_000_000_000,
                },
                gas_limit: 21_000,
                chain_id: 56,
            },
        };
        let mut other = base.clone();
        other.params.chain_id = 1;
        assert_ne!(
            base.sign(&key).unwrap().raw,
            other.sign(&key).unwrap().raw,
            "the chain id did not reach the signature - this transaction is replayable"
        );
    }

    /// A zero value and empty calldata must encode as RLP's empty string, not
    /// as a zero byte. This is the encoding mistake that produces a valid
    /// signature over the wrong bytes.
    #[test]
    fn zero_fields_encode_as_empty() {
        let tx = Tx {
            to: EvmAddress::parse("0x0000000000000000000000000000000000000000").unwrap(),
            value: 0,
            data: Vec::new(),
            params: TxParams {
                nonce: 0,
                fees: Fees::Legacy { gas_price: 0 },
                gas_limit: 0,
                chain_id: 56,
            },
        };
        let p = tx.signing_payload();
        // nonce, gasPrice, gasLimit, value and data are all 0x80; the address
        // is 20 bytes so it keeps its 0x94 header.
        assert_eq!(p[1], 0x80, "nonce should be the empty string");
        assert_eq!(p[2], 0x80, "gas price should be the empty string");
        assert_eq!(p[3], 0x80, "gas limit should be the empty string");
        assert_eq!(p[4], 0x94, "address length header");
    }
}

/// A confirmed mainnet EIP-1559 transaction, rebuilt from its own fields.
///
/// The same check as the Bitcoin side's: take something the network has already
/// accepted, encode it here, and require the hash to come out identical. That
/// covers the type byte, the field order, the empty access list and the bare
/// parity bit - none of which produce an error when wrong, only a transaction
/// nobody will mine.
///
/// The signing payload is checked separately and more sharply: recovering a
/// public key from it has to yield the address that actually sent this
/// transaction. If a single byte of what gets signed were different, the
/// recovered address would be somebody else's.
#[cfg(test)]
mod mainnet_eip1559 {
    use super::*;

    /// `0xc9440ab9…5c87`: a USDC transfer on Ethereum, block 25,902,440.
    const HASH: &str = "c9440ab99b950380b7d21f02eef1ea76e2054eb624455530ba07bc02a88f5c87";
    /// As the node reports it: all lowercase, which carries no EIP-55
    /// capitalisation to check. Compared case-insensitively for that reason,
    /// and the checksummed form is verified separately by round-tripping it.
    const FROM: &str = "0x654de8a1b6f2b2f8de8c32b7b5b50d401d3fc897";
    const TO: &str = "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48";
    const R: &str = "da2abcd25cecdcc0da28768e88d8dee4cd1473ed1e19e688902bdd0a318c330f";
    const S: &str = "6c29d09b668b6f6eaf60d4107e225aadad72e8d21f9353b89be0f1ab4df81f6b";

    fn tx() -> Tx {
        Tx {
            to: EvmAddress::parse(TO).unwrap(),
            value: 0,
            data: hex::decode(
                "a9059cbb0000000000000000000000007f5050035f3eebe9e7b7b34cc954e79d4648f401\
                 000000000000000000000000000000000000000000000000000000059f01017b",
            )
            .unwrap(),
            params: TxParams {
                nonce: 36_650,
                gas_limit: 100_000,
                chain_id: 1,
                fees: Fees::Eip1559 {
                    max_fee_per_gas: 5_114_139_756,
                    max_priority_fee_per_gas: 5_000_000_000,
                    // Not part of the transaction; any value encodes the same.
                    base_fee: 0,
                },
            },
        }
    }

    #[test]
    fn the_encoding_reproduces_a_real_transaction_hash() {
        let raw = tx().encode_signed(&hex::decode(R).unwrap(), &hex::decode(S).unwrap(), 1);
        assert_eq!(raw[0], TYPE_EIP1559, "the envelope byte is missing");
        assert_eq!(hex::encode(keccak(&raw)), HASH);
    }

    /// What gets signed is the thing that matters, and this is the sharpest
    /// available check on it: the address that comes back out.
    #[test]
    fn the_signing_payload_recovers_the_real_sender() {
        let t = tx();
        let hash = keccak(&t.signing_payload());
        let sig = k256::ecdsa::Signature::from_scalars(
            <[u8; 32]>::try_from(hex::decode(R).unwrap()).unwrap(),
            <[u8; 32]>::try_from(hex::decode(S).unwrap()).unwrap(),
        )
        .unwrap();
        let key = k256::ecdsa::VerifyingKey::recover_from_prehash(
            &hash,
            &sig,
            RecoveryId::from_byte(1).unwrap(),
        )
        .expect("could not recover a key - the payload is not what was signed");
        let point = k256::elliptic_curve::sec1::ToEncodedPoint::to_encoded_point(
            &k256::PublicKey::from(&key),
            false,
        );
        let got = EvmAddress::from_public_key(point.as_bytes()).unwrap();
        assert!(
            got.to_string().eq_ignore_ascii_case(FROM),
            "recovered {got}, expected {FROM}"
        );
        // And what we print carries a valid EIP-55 checksum, which parsing it
        // back as mixed case proves.
        assert_eq!(EvmAddress::parse(&got.to_string()).unwrap(), got);
    }

    /// The two formats must not produce the same bytes for the same
    /// transaction, or one chain's signature would be valid on the other's
    /// format.
    #[test]
    fn the_two_formats_are_distinguishable() {
        let mut legacy = tx();
        legacy.params.fees = Fees::Legacy {
            gas_price: 5_114_139_756,
        };
        let a = tx().encode_signed(&[1], &[2], 1);
        let b = legacy.encode_signed(&[1], &[2], 1);
        assert_ne!(a, b);
        assert_eq!(a[0], TYPE_EIP1559);
        // A legacy transaction starts with an RLP list header, never 0x02.
        assert!(b[0] >= 0xc0, "legacy should be a bare RLP list");
    }

    /// A type-2 transaction pays the base fee plus the tip, not the ceiling.
    /// Showing the ceiling as the price says a transfer costs about twice what
    /// it does.
    #[test]
    fn the_expected_cost_is_not_the_ceiling() {
        let f = Fees::Eip1559 {
            max_fee_per_gas: 5_114_139_756,
            max_priority_fee_per_gas: 5_000_000_000,
            base_fee: 77_969_276,
        };
        assert_eq!(f.expected_per_gas(), 5_077_969_276);
        assert_eq!(f.max_per_gas(), 5_114_139_756);
        assert!(f.expected_per_gas() < f.max_per_gas());

        // A base fee that has risen past the ceiling is capped by it - that is
        // what the ceiling is for.
        let spiked = Fees::Eip1559 {
            max_fee_per_gas: 1_000,
            max_priority_fee_per_gas: 100,
            base_fee: 10_000,
        };
        assert_eq!(spiked.expected_per_gas(), 1_000);

        // Legacy has no such distinction.
        let l = Fees::Legacy { gas_price: 5_000 };
        assert_eq!(l.expected_per_gas(), l.max_per_gas());
        assert!(!l.is_eip1559());
    }
}
