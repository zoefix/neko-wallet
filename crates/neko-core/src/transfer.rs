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
    Solana(neko_solana::tx::TxParams),
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
            // Solana carries no calldata: the amount lives in the
            // instruction, which is built by the chain crate rather than
            // encoded here.
            Asset::Trx | Asset::Bnb | Asset::Sol | Asset::SplToken { .. } => Ok(None),
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
            (Asset::Sol, ChainTxParams::Solana(p)) => {
                let from = req.from.as_solana()?;
                let lamports = u64::try_from(req.amount.raw)
                    .map_err(|_| neko_solana::SolanaError::AmountTooLarge)?;
                let mut ixs = compute_budget(p);
                ixs.push(neko_solana::tx::transfer_sol(
                    from,
                    req.to.as_solana()?,
                    lamports,
                ));
                sign_solana(from, ixs, p, &key)
            }
            (Asset::SplToken { mint, decimals }, ChainTxParams::Solana(p)) => {
                let from = req.from.as_solana()?;
                let to = req.to.as_solana()?;
                let amount = u64::try_from(req.amount.raw)
                    .map_err(|_| neko_solana::SolanaError::AmountTooLarge)?;

                // Tokens do not go to the recipient's address. They go to an
                // account derived from it and the mint, which may not exist -
                // and if it does not, this transfer creates it and pays its
                // rent.
                let source = neko_solana::associated_token_address(&from, &mint)?;
                let destination = neko_solana::associated_token_address(&to, &mint)?;

                let mut ixs = compute_budget(p);
                if p.create_recipient_account {
                    ixs.push(neko_solana::tx::create_associated_token_account(
                        from, to, mint,
                    )?);
                }
                // `decimals` is passed to the program, which checks it against
                // the mint on chain. A wrong value fails there rather than
                // moving a millionth or a million times the amount.
                ixs.push(neko_solana::tx::transfer_token_checked(
                    source,
                    mint,
                    destination,
                    from,
                    amount,
                    decimals,
                ));
                sign_solana(from, ixs, p, &key)
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
        Ok(match chain {
            // SLIP-0010 over Ed25519. Deriving this with the BIP32 machinery
            // the other chains use would produce a perfectly valid key for an
            // address that holds nothing.
            ChainId::Solana => neko_hd::solana::private_key_at(&seed, index)?,
            ChainId::Tron | ChainId::Bsc => {
                neko_hd::derive::private_key_at_coin(&seed, chain.coin_type(), 0, index)?
            }
        })
    }
}

/// The compute-budget instructions, if the cluster gave us a reason for them.
///
/// Both are omitted when they would be no-ops, because every instruction costs
/// bytes and a transaction has one packet to fit in.
fn compute_budget(p: &neko_solana::tx::TxParams) -> Vec<neko_solana::tx::Instruction> {
    let mut out = Vec::with_capacity(2);
    if p.compute_unit_limit > 0 {
        out.push(neko_solana::tx::set_compute_unit_limit(
            p.compute_unit_limit,
        ));
    }
    if p.compute_unit_price > 0 {
        out.push(neko_solana::tx::set_compute_unit_price(
            p.compute_unit_price,
        ));
    }
    out
}

/// Compile, sign, and hand back the bytes with the id they will be found by.
///
/// `Transaction::sign` refuses a key that is not the fee payer, which is this
/// chain's equivalent of the public-key recovery check the other two do: proof
/// that the signature belongs to the account being debited.
fn sign_solana(
    from: neko_hd::SolanaAddress,
    ixs: Vec<neko_solana::tx::Instruction>,
    p: &neko_solana::tx::TxParams,
    key: &Zeroizing<[u8; 32]>,
) -> Result<SignedTransfer, CoreError> {
    let msg = neko_solana::tx::Message::compile(&from, &ixs, p.recent_blockhash)?;
    let signed = neko_solana::tx::Transaction::sign(msg, key)?;
    Ok(SignedTransfer {
        id: signed.id(),
        raw: signed.serialize()?,
    })
}
