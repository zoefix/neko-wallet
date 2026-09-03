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

use neko_hd::Address;
use neko_tron::tx::{self, SignedTx, TxParams};
use zeroize::Zeroizing;

use crate::amount::Amount;
use crate::error::CoreError;
use crate::session::Session;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Asset {
    Trx,
    /// TRC20, carrying its contract address so the network's own USDT is used.
    Trc20 {
        contract: Address,
        decimals: u8,
    },
}

impl Asset {
    pub fn decimals(self) -> u8 {
        match self {
            Asset::Trx => neko_tron::TRX_DECIMALS,
            Asset::Trc20 { decimals, .. } => decimals,
        }
    }
    pub fn fee_limit(self) -> i64 {
        match self {
            Asset::Trx => neko_tron::FEE_LIMIT_TRX,
            // A contract call without a fee limit fails for lack of energy.
            Asset::Trc20 { .. } => neko_tron::FEE_LIMIT_TRC20,
        }
    }
}

/// Everything the user chose. Kept together so the confirmation screen and the
/// signer see exactly the same values.
#[derive(Debug, Clone)]
pub struct TransferRequest {
    pub wallet_id: i64,
    pub from: Address,
    pub to: Address,
    pub asset: Asset,
    pub amount: Amount,
}

impl TransferRequest {
    pub fn parse(
        wallet_id: i64,
        from: Address,
        to_input: &str,
        amount_input: &str,
        asset: Asset,
    ) -> Result<Self, CoreError> {
        let to = Address::parse(to_input.trim()).map_err(|_| CoreError::BadAddress)?;
        let amount = Amount::parse(amount_input, asset.decimals())?;
        Ok(Self {
            wallet_id,
            from,
            to,
            asset,
            amount,
        })
    }

    /// The TRC20 call data, needed both for the transaction and for asking the
    /// chain to estimate energy.
    pub fn calldata(&self) -> Result<Option<Vec<u8>>, CoreError> {
        match self.asset {
            Asset::Trx => Ok(None),
            Asset::Trc20 { .. } => Ok(Some(tx::encode_trc20_transfer(
                self.to,
                self.amount.raw as u128,
            )?)),
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
        params: &TxParams,
    ) -> Result<SignedTx, CoreError> {
        let key = self.private_key_for(req.wallet_id, 0)?;

        let raw = match req.asset {
            Asset::Trx => {
                let sun = i64::try_from(req.amount.raw)
                    .map_err(|_| neko_tron::TxError::AmountTooLarge)?;
                tx::build_trx_transfer(req.from, req.to, sun, params)?
            }
            Asset::Trc20 { contract, .. } => tx::build_trc20_transfer(
                req.from,
                contract,
                req.to,
                req.amount.raw as u128,
                params,
            )?,
        };
        Ok(tx::sign(&raw, &key, req.from)?)
    }

    /// Borrow the signing key for one wallet/index.
    fn private_key_for(
        &self,
        wallet_id: i64,
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
        Ok(neko_hd::derive::private_key_at(&seed, 0, index)?)
    }
}
