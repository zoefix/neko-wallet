//! Building and signing BNB Chain transactions.
//!
//! Legacy (type 0) transactions, not EIP-1559. BNB Chain accepts both, gas
//! there is cheap and stable enough that a priority fee buys nothing, and type
//! 0 is the format every node and explorer has understood since the beginning.
//! Fewer fields is fewer ways to sign something other than what was shown.
//!
//! Like the TRON side, nothing is fetched pre-built: the node supplies a
//! nonce, a gas price and an estimate, and the bytes that get signed are
//! assembled here. A node cannot hand back a transaction paying itself and
//! have it signed.

use k256::ecdsa::{RecoveryId, SigningKey};
use neko_hd::EvmAddress;

use crate::error::EvmError;
use crate::rlp;

/// What the chain must tell us before a transaction can be built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxParams {
    pub nonce: u64,
    pub gas_price: u128,
    pub gas_limit: u64,
    pub chain_id: u64,
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
        let mut body = Vec::new();
        rlp::uint(&mut body, self.params.nonce as u128);
        rlp::uint(&mut body, self.params.gas_price);
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

        // EIP-155: v carries the chain id as well as the recovery bit.
        let v = recid.to_byte() as u128 + 35 + 2 * self.params.chain_id as u128;

        let mut body = Vec::new();
        rlp::uint(&mut body, self.params.nonce as u128);
        rlp::uint(&mut body, self.params.gas_price);
        rlp::uint(&mut body, self.params.gas_limit as u128);
        rlp::bytes(&mut body, self.to.as_bytes());
        rlp::uint(&mut body, self.value);
        rlp::bytes(&mut body, &self.data);
        rlp::uint(&mut body, v);
        // r and s are trimmed of leading zeros: RLP integers are minimal, and
        // a fixed 32-byte encoding would hash to something the network rejects.
        rlp::bytes(&mut body, &trim(&sig.r().to_bytes()));
        rlp::bytes(&mut body, &trim(&sig.s().to_bytes()));

        let mut raw = Vec::new();
        rlp::list(&mut raw, &body);
        let hash = keccak(&raw);
        Ok(SignedTx { raw, hash })
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
                gas_price: 20_000_000_000,
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
                gas_price: 3_000_000_000,
                gas_limit: 21_000,
                chain_id: crate::CHAIN_ID,
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
                gas_price: 1_000_000_000,
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
                gas_price: 0,
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
