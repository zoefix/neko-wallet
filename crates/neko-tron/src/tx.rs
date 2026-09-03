//! Building and signing TRON transactions.
//!
//! **Zero trust: we never ask a node to build a transaction for us.** The
//! wallet constructs the exact bytes it intends to sign; the node only supplies
//! a block reference. Signing an opaque blob handed over by a node means a
//! compromised or malicious node can return "pay the attacker" and have you
//! sign it. Verified byte-for-byte against `vectors/tx.json`.

use neko_hd::{derive, Address};
use sha2::{Digest, Sha256};
use sha3::Keccak256;

use crate::error::TxError;
use crate::pb::Writer;

/// `core.Transaction.Contract.ContractType`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum ContractType {
    Transfer = 1,
    TriggerSmartContract = 31,
    FreezeBalanceV2 = 54,
    UnfreezeBalanceV2 = 55,
    WithdrawExpireUnfreeze = 56,
    DelegateResource = 57,
    UnDelegateResource = 58,
}

impl ContractType {
    fn type_url(self) -> &'static str {
        match self {
            ContractType::Transfer => "type.googleapis.com/protocol.TransferContract",
            ContractType::TriggerSmartContract => {
                "type.googleapis.com/protocol.TriggerSmartContract"
            }
            ContractType::FreezeBalanceV2 => "type.googleapis.com/protocol.FreezeBalanceV2Contract",
            ContractType::UnfreezeBalanceV2 => {
                "type.googleapis.com/protocol.UnfreezeBalanceV2Contract"
            }
            ContractType::WithdrawExpireUnfreeze => {
                "type.googleapis.com/protocol.WithdrawExpireUnfreezeContract"
            }
            ContractType::DelegateResource => {
                "type.googleapis.com/protocol.DelegateResourceContract"
            }
            ContractType::UnDelegateResource => {
                "type.googleapis.com/protocol.UnDelegateResourceContract"
            }
        }
    }
}

/// `ResourceCode`: BANDWIDTH = 0, ENERGY = 1.
const RESOURCE_ENERGY: u64 = 1;

/// Everything the chain context contributes. Timestamps are milliseconds.
#[derive(Debug, Clone)]
pub struct TxParams {
    pub ref_block_num: u64,
    /// The 32-byte block id.
    pub ref_block_hash: [u8; 32],
    pub timestamp: i64,
    pub expiration: i64,
    /// Required (> 0) for contract calls, and must be 0 for a plain transfer.
    pub fee_limit: i64,
}

impl TxParams {
    /// `ref_block_bytes` is bytes 6..8 of the block number as a big-endian u64.
    fn ref_block_bytes(&self) -> [u8; 2] {
        let b = self.ref_block_num.to_be_bytes();
        [b[6], b[7]]
    }
    /// `ref_block_hash` is bytes 8..16 of the block id.
    fn ref_hash(&self) -> [u8; 8] {
        let mut out = [0u8; 8];
        out.copy_from_slice(&self.ref_block_hash[8..16]);
        out
    }
    fn validate(&self) -> Result<(), TxError> {
        if self.timestamp <= 0 {
            return Err(TxError::BadTimestamp);
        }
        if self.expiration <= self.timestamp {
            return Err(TxError::BadExpiration);
        }
        Ok(())
    }
}

fn wrap_contract(kind: ContractType, body: &Writer) -> Writer {
    // google.protobuf.Any: 1 = type_url (string), 2 = value (bytes)
    let mut any = Writer::new();
    any.string(1, kind.type_url()).bytes(2, body.as_slice());

    let mut contract = Writer::new();
    contract.uint64(1, kind as u64).message(2, &any);
    contract
}

/// `TransactionRaw`: 1 ref_block_bytes, 4 ref_block_hash, 8 expiration,
/// 11 contract, 14 timestamp, 18 fee_limit.
fn build_raw(p: &TxParams, contract: &Writer) -> Result<Vec<u8>, TxError> {
    p.validate()?;
    let mut w = Writer::new();
    w.bytes(1, &p.ref_block_bytes())
        .bytes(4, &p.ref_hash())
        .uint64(8, p.expiration as u64)
        .message(11, contract)
        .uint64(14, p.timestamp as u64);
    if p.fee_limit > 0 {
        w.uint64(18, p.fee_limit as u64);
    }
    Ok(w.finish())
}

/// A plain TRX transfer. `amount` is in sun (1 TRX = 1e6 sun).
pub fn build_trx_transfer(
    from: Address,
    to: Address,
    amount_sun: i64,
    p: &TxParams,
) -> Result<Vec<u8>, TxError> {
    if amount_sun <= 0 {
        return Err(TxError::NonPositiveAmount);
    }
    let mut body = Writer::new();
    body.bytes(1, from.as_bytes())
        .bytes(2, to.as_bytes())
        .uint64(3, amount_sun as u64);
    build_raw(p, &wrap_contract(ContractType::Transfer, &body))
}

/// `transfer(address,uint256)` — the first 4 bytes of its keccak256 hash.
pub fn trc20_transfer_selector() -> [u8; 4] {
    let d = Keccak256::digest(b"transfer(address,uint256)");
    let mut out = [0u8; 4];
    out.copy_from_slice(&d[..4]);
    out
}

/// ABI-encode `transfer(address,uint256)`.
///
/// The address argument is the **20-byte** form with the `0x41` prefix dropped,
/// left-padded to 32 bytes. On-chain addresses carry the prefix; ABI arguments
/// do not. Getting this wrong sends tokens to a different address entirely.
pub fn encode_trc20_transfer(to: Address, amount: u128) -> Result<Vec<u8>, TxError> {
    if amount == 0 {
        return Err(TxError::NonPositiveAmount);
    }
    let mut out = vec![0u8; 4 + 64];
    out[..4].copy_from_slice(&trc20_transfer_selector());
    out[4 + 12..4 + 32].copy_from_slice(&to.to_evm_bytes());
    out[4 + 64 - 16..].copy_from_slice(&amount.to_be_bytes());
    Ok(out)
}

pub fn build_trc20_transfer(
    from: Address,
    contract: Address,
    to: Address,
    amount: u128,
    p: &TxParams,
) -> Result<Vec<u8>, TxError> {
    if p.fee_limit <= 0 {
        return Err(TxError::MissingFeeLimit);
    }
    let data = encode_trc20_transfer(to, amount)?;
    // TriggerSmartContract: 1 owner, 2 contract, 4 data. call_value is 0 and so
    // is omitted.
    let mut body = Writer::new();
    body.bytes(1, from.as_bytes())
        .bytes(2, contract.as_bytes())
        .bytes(4, &data);
    build_raw(p, &wrap_contract(ContractType::TriggerSmartContract, &body))
}

// ── Staking (TRON Stake 2.0) ───────────────────────────────────────────────
//
// Freezing is not spending: the TRX stays yours, just locked, and comes back
// after the unbonding period. That is a different thing from burning TRX for
// energy, where it is simply gone.

pub fn build_freeze_for_energy(
    owner: Address,
    amount_sun: i64,
    p: &TxParams,
) -> Result<Vec<u8>, TxError> {
    if amount_sun <= 0 {
        return Err(TxError::NonPositiveAmount);
    }
    let mut body = Writer::new();
    body.bytes(1, owner.as_bytes())
        .uint64(2, amount_sun as u64)
        .uint64(3, RESOURCE_ENERGY);
    build_raw(p, &wrap_contract(ContractType::FreezeBalanceV2, &body))
}

pub fn build_unfreeze_energy(
    owner: Address,
    amount_sun: i64,
    p: &TxParams,
) -> Result<Vec<u8>, TxError> {
    if amount_sun <= 0 {
        return Err(TxError::NonPositiveAmount);
    }
    let mut body = Writer::new();
    body.bytes(1, owner.as_bytes())
        .uint64(2, amount_sun as u64)
        .uint64(3, RESOURCE_ENERGY);
    build_raw(p, &wrap_contract(ContractType::UnfreezeBalanceV2, &body))
}

pub fn build_withdraw_expire_unfreeze(owner: Address, p: &TxParams) -> Result<Vec<u8>, TxError> {
    let mut body = Writer::new();
    body.bytes(1, owner.as_bytes());
    build_raw(
        p,
        &wrap_contract(ContractType::WithdrawExpireUnfreeze, &body),
    )
}

/// Delegate energy to another address.
///
/// `lock` is always false: TIP-542 makes a locked delegation unrecoverable for
/// three days, whereas an unlocked one can be reclaimed at will.
pub fn build_delegate_energy(
    owner: Address,
    receiver: Address,
    amount_sun: i64,
    p: &TxParams,
) -> Result<Vec<u8>, TxError> {
    if amount_sun <= 0 {
        return Err(TxError::NonPositiveAmount);
    }
    let mut body = Writer::new();
    body.bytes(1, owner.as_bytes())
        .uint64(2, RESOURCE_ENERGY)
        .uint64(3, amount_sun as u64)
        .bytes(4, receiver.as_bytes());
    // lock (field 5) is false, so proto3 omits it.
    build_raw(p, &wrap_contract(ContractType::DelegateResource, &body))
}

pub fn build_undelegate_energy(
    owner: Address,
    receiver: Address,
    amount_sun: i64,
    p: &TxParams,
) -> Result<Vec<u8>, TxError> {
    if amount_sun <= 0 {
        return Err(TxError::NonPositiveAmount);
    }
    let mut body = Writer::new();
    body.bytes(1, owner.as_bytes())
        .uint64(2, RESOURCE_ENERGY)
        .uint64(3, amount_sun as u64)
        .bytes(4, receiver.as_bytes());
    build_raw(p, &wrap_contract(ContractType::UnDelegateResource, &body))
}

// ── Identity and signing ───────────────────────────────────────────────────

/// The transaction id is a plain SHA-256 of `raw_data`. No keccak, and none of
/// Ethereum's `\x19` message prefix.
pub fn txid(raw_data: &[u8]) -> [u8; 32] {
    Sha256::digest(raw_data).into()
}

pub struct SignedTx {
    pub txid: [u8; 32],
    /// The complete `Transaction` protobuf, ready to broadcast.
    pub raw_tx: Vec<u8>,
    /// r(32) || s(32) || recovery_id(1)
    pub signature: [u8; 65],
}

/// Sign `raw_data`, then verify the signature recovers to `expect_from`.
///
/// The self-check costs almost nothing and catches a miscomputed recovery id —
/// an error whose consequence is a transaction the network rejects, or worse,
/// one attributed to an address we did not mean to spend from.
pub fn sign(
    raw_data: &[u8],
    private_key: &[u8; 32],
    expect_from: Address,
) -> Result<SignedTx, TxError> {
    use k256::ecdsa::{RecoveryId, Signature, SigningKey, VerifyingKey};

    let hash = txid(raw_data);
    let sk = SigningKey::from_slice(private_key).map_err(|_| TxError::Sign)?;
    let (sig, recid): (Signature, RecoveryId) = sk
        .sign_prehash_recoverable(&hash)
        .map_err(|_| TxError::Sign)?;

    // TRON wants r || s || recovery_id.
    let mut signature = [0u8; 65];
    signature[..64].copy_from_slice(&sig.to_bytes());
    signature[64] = recid.to_byte();

    let recovered =
        VerifyingKey::recover_from_prehash(&hash, &sig, recid).map_err(|_| TxError::Sign)?;
    let point = recovered.to_encoded_point(false);
    let got = Address::from_public_key(point.as_bytes())?;
    if got != expect_from {
        return Err(TxError::SelfCheck {
            got: got.to_string(),
            want: expect_from.to_string(),
        });
    }

    // Transaction: 1 = raw_data (message), 2 = signature (repeated bytes)
    let mut tx = Writer::new();
    tx.bytes(1, raw_data).bytes(2, &signature);

    Ok(SignedTx {
        txid: hash,
        raw_tx: tx.finish(),
        signature,
    })
}

/// Recover the signer's address from a signed transaction. Used to audit a
/// transaction we did not build ourselves.
pub fn recover_signer(raw_data: &[u8], signature: &[u8; 65]) -> Result<Address, TxError> {
    use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
    let hash = txid(raw_data);
    let sig = Signature::from_slice(&signature[..64]).map_err(|_| TxError::Sign)?;
    let recid = RecoveryId::from_byte(signature[64]).ok_or(TxError::Sign)?;
    let vk = VerifyingKey::recover_from_prehash(&hash, &sig, recid).map_err(|_| TxError::Sign)?;
    let point = vk.to_encoded_point(false);
    Ok(Address::from_public_key(point.as_bytes())?)
}

/// Re-exported so callers do not need `neko_hd` directly for the common path.
pub use derive::address_from_private_key;
