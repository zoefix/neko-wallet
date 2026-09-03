//! Building and signing outgoing transfers.
//!
//! The split here is deliberate. Fetching the block reference and broadcasting
//! are async and happen on the runtime; **signing is synchronous and happens on
//! the thread that owns the session**. Only bytes cross the boundary, never the
//! private key.
//!
//! And the wallet builds the transaction itself. It never signs something a
//! node handed it — a compromised node could otherwise return "pay the
//! attacker" and have the user approve it.

use zeroize::Zeroizing;

use crate::amount::Amount;
use crate::chain::{Asset, ChainAddress, ChainId};
use crate::error::CoreError;
use crate::session::Session;

/// What each chain needs from its node before a transaction can be built.
///
/// The two have nothing in common - a block reference and an expiry on one
/// side, a nonce and a gas price on the other - so they are kept apart rather
/// than flattened into a shape that fits neither.
#[derive(Debug, Clone)]
pub enum ChainTxParams {
    Tron(Box<neko_tron::tx::TxParams>),
    Evm(neko_evm::tx::TxParams),
}

/// A signed transaction, reduced to what every caller actually needs: the
/// bytes to broadcast, and the id to show and to poll with.
#[derive(Debug, Clone)]
pub struct SignedTransfer {
    pub raw: Vec<u8>,
    pub id: String,
}

/// Everything the user chose. Kept together so the confirmation screen and the
/// signer see exactly the same values.
#[derive(Debug, Clone)]
pub struct TransferRequest {
    pub wallet_id: i64,
    pub from: ChainAddress,
    pub to: ChainAddress,
    pub asset: Asset,
    pub amount: Amount,
}

impl TransferRequest {
    pub fn chain(&self) -> ChainId {
        self.asset.chain()
    }
}

impl TransferRequest {
    pub fn parse(
        wallet_id: i64,
        from: ChainAddress,
        to_input: &str,
        amount_input: &str,
        asset: Asset,
    ) -> Result<Self, CoreError> {
        // The destination is parsed *as an address on the asset's chain*, so a
        // TRON address in a BNB Chain transfer is refused here rather than
        // becoming a payment to an account nobody holds the key to.
        let to = ChainAddress::parse(asset.chain(), to_input)?;
        if from.chain() != asset.chain() {
            return Err(CoreError::WrongChain);
        }
        let amount = Amount::parse(amount_input, asset.decimals())?;
        Ok(Self {
            wallet_id,
            from,
            to,
            asset,
            amount,
        })
    }

    /// Token call data, needed both for the transaction and for asking the
    /// chain what it will cost.
    pub fn calldata(&self) -> Result<Option<Vec<u8>>, CoreError> {
        match self.asset {
            Asset::Trx | Asset::Bnb => Ok(None),
            Asset::Trc20 { .. } => Ok(Some(neko_tron::tx::encode_trc20_transfer(
                self.to.as_tron()?,
                self.amount.raw as u128,
            )?)),
            Asset::Bep20 { .. } => Ok(Some(neko_evm::abi::transfer(
                self.to.as_evm()?,
                self.amount.raw as u128,
            ))),
        }
    }
}

impl Session {
    /// Build and sign. Synchronous, and it borrows the key only for the call.
    ///
    /// The signature is self-checked by recovering the signer's address, so a
    /// miscomputed recovery id cannot produce a transaction attributed to some
    /// other address.
    pub fn sign_transfer(
        &self,
        req: &TransferRequest,
        params: &ChainTxParams,
    ) -> Result<SignedTransfer, CoreError> {
        // The key is derived for the chain being spent on. One phrase, two
        // coin types, two different keys - deriving with the wrong one signs
        // with a key that does not own the funds.
        let key = self.private_key_for(req.wallet_id, req.chain(), 0)?;

        match (req.asset, params) {
            (Asset::Trx, ChainTxParams::Tron(p)) => {
                let sun = i64::try_from(req.amount.raw)
                    .map_err(|_| neko_tron::TxError::AmountTooLarge)?;
                let raw = neko_tron::tx::build_trx_transfer(
                    req.from.as_tron()?,
                    req.to.as_tron()?,
                    sun,
                    p,
                )?;
                let signed = neko_tron::tx::sign(&raw, &key, req.from.as_tron()?)?;
                Ok(SignedTransfer {
                    raw: signed.raw_tx,
                    id: hex::encode(signed.txid),
                })
            }
            (Asset::Trc20 { contract, .. }, ChainTxParams::Tron(p)) => {
                let raw = neko_tron::tx::build_trc20_transfer(
                    req.from.as_tron()?,
                    contract,
                    req.to.as_tron()?,
                    req.amount.raw as u128,
                    p,
                )?;
                let signed = neko_tron::tx::sign(&raw, &key, req.from.as_tron()?)?;
                Ok(SignedTransfer {
                    raw: signed.raw_tx,
                    id: hex::encode(signed.txid),
                })
            }
            (Asset::Bnb, ChainTxParams::Evm(p)) => {
                let tx = neko_evm::tx::Tx {
                    to: req.to.as_evm()?,
                    value: req.amount.raw as u128,
                    data: Vec::new(),
                    params: *p,
                };
                let signed = tx.sign(&key)?;
                let id = signed.hash_hex();
                Ok(SignedTransfer {
                    raw: signed.raw,
                    id,
                })
            }
            (Asset::Bep20 { contract, .. }, ChainTxParams::Evm(p)) => {
                // The amount lives in the calldata; the transaction itself
                // moves no BNB.
                let tx = neko_evm::tx::Tx {
                    to: contract,
                    value: 0,
                    data: neko_evm::abi::transfer(req.to.as_evm()?, req.amount.raw as u128),
                    params: *p,
                };
                let signed = tx.sign(&key)?;
                let id = signed.hash_hex();
                Ok(SignedTransfer {
                    raw: signed.raw,
                    id,
                })
            }
            // Unreachable through the interface, because the asset and the
            // parameters are chosen together. Refused rather than assumed.
            _ => Err(CoreError::WrongChain),
        }
    }

    /// Borrow the signing key for one wallet/index.
    fn private_key_for(
        &self,
        wallet_id: i64,
        chain: ChainId,
        index: u32,
    ) -> Result<Zeroizing<[u8; 32]>, CoreError> {
        use neko_store::repo::wallets;
        let conn = self.conn()?;
        let dk = self.data_key();

        if let Some(pk) = wallets::privkey(conn, dk, wallet_id)? {
            if pk.len() != 32 {
                return Err(CoreError::BadPrivateKey);
            }
            let mut k = [0u8; 32];
            k.copy_from_slice(&pk);
            return Ok(Zeroizing::new(k));
        }
        let seed = self.seed_for(wallet_id)?;
        Ok(neko_hd::derive::private_key_at_coin(
            &seed,
            chain.coin_type(),
            0,
            index,
        )?)
    }
}
