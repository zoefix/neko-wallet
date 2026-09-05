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
    Ton(Box<TonTxParams>),
    Evm(neko_evm::tx::TxParams),
    Solana(neko_solana::tx::TxParams),
    Bitcoin(Box<BtcTxParams>),
}

/// What TON needs told before a message can be built.
///
/// `seqno` is the wallet contract's own counter and takes a nonce's place, but
/// unlike a nonce a stale one is not rejected - the message is ignored, which
/// looks exactly like a transfer that vanished. `valid_until` is signed over,
/// so a message that sat too long cannot be replayed later.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TonTxParams {
    pub seqno: u32,
    pub valid_until: u32,
    /// Whether the wallet contract still has to be deployed. Its code travels
    /// with the first message a wallet ever sends, and including it afterwards
    /// is rejected.
    pub deploy: bool,
    /// For a token transfer: the jetton master the quote verified, and our own
    /// jetton wallet under it. The message goes to that wallet, not to the
    /// recipient.
    ///
    /// The master travels with the address because the address alone cannot be
    /// checked offline - it is a contract's hash, derived by asking the master
    /// itself. Carrying both lets signing refuse a wallet that was derived from
    /// some *other* token, which is the one way a verified quote could still
    /// move the wrong thing.
    pub jetton_wallet: Option<(neko_ton::TonAddress, neko_ton::TonAddress)>,
}

/// Which coins a Bitcoin transfer spends, and what comes back.
///
/// Unlike the other three chains' parameters, this is not just context - it is
/// half the transaction. The coins were chosen at quote time by an algorithm
/// that also decided the fee, and signing has to use exactly those, because the
/// fee is inputs minus outputs and any substitution changes it silently.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtcTxParams {
    pub inputs: Vec<neko_btc::tx::Utxo>,
    /// What returns to us, when it is above dust. `None` means the remainder
    /// went to the fee because it was too small to create an output for.
    pub change: Option<u64>,
    pub change_to: neko_hd::BtcAddress,
    /// What was quoted. Re-derived from the transaction before signing, and a
    /// mismatch is refused - this is the check that a forgotten change output
    /// cannot get past.
    pub fee: u64,
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
            // encoded here. Bitcoin has no calldata at all - an amount is an
            // output, not an argument.
            Asset::Trx
            | Asset::Bnb
            | Asset::Eth
            | Asset::Sol
            | Asset::SplToken { .. }
            | Asset::Btc
            // TON has no calldata either: a body is a cell, built by the chain
            // crate rather than encoded here.
            | Asset::Gram
            // Polygon's coin, like the other EVM chains': the value goes in
            // the transaction, not in a call.
            | Asset::Pol
            | Asset::Jetton { .. } => Ok(None),
            Asset::Trc20 { .. } => Ok(Some(neko_tron::tx::encode_trc20_transfer(
                self.to.as_tron()?,
                self.amount.raw as u128,
            )?)),
            Asset::Bep20 { .. } | Asset::Erc20 { .. } | Asset::PolygonErc20 { .. } => Ok(Some(
                neko_evm::abi::transfer(self.to.as_evm()?, self.amount.raw as u128),
            )),
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
            // One arm for both EVM chains: the bytes are identical and the
            // difference - chain id and transaction format - already lives in
            // the parameters the quote produced.
            (Asset::Bnb | Asset::Eth | Asset::Pol, ChainTxParams::Evm(p)) => {
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
            (
                Asset::Bep20 { contract, .. }
                | Asset::Erc20 { contract, .. }
                | Asset::PolygonErc20 { contract, .. },
                ChainTxParams::Evm(p),
            ) => {
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
            (Asset::Gram, ChainTxParams::Ton(p)) => {
                let to = req.to.as_ton()?;
                let amount = u128::try_from(req.amount.raw)
                    .map_err(|_| neko_ton::TonError::AmountTooLarge)?;
                // The form the destination was written in decides whether a
                // failure returns the coins. Respecting what was pasted is the
                // predictable choice: a bounceable address sent to a wallet
                // that does not exist yet comes back rather than arriving.
                let inner = neko_ton::message::internal_message(&to, amount, to.bounceable, None)?;
                sign_ton(req, p, inner, &key)
            }
            (Asset::Jetton { master, .. }, ChainTxParams::Ton(p)) => {
                let from = req.from.as_ton()?;
                let to = req.to.as_ton()?;
                let amount = u128::try_from(req.amount.raw)
                    .map_err(|_| neko_ton::TonError::AmountTooLarge)?;
                let (quoted_master, jetton_wallet) = p
                    .jetton_wallet
                    .ok_or_else(|| neko_ton::TonError::NoJettonWallet("USDT".into()))?;
                // The quote asked *this* master where our balance lives. If the
                // asset being signed for is a different token, the address in
                // hand is the wrong contract and the transfer would move the
                // wrong balance - so it is refused rather than sent.
                if quoted_master != master {
                    return Err(CoreError::WrongToken {
                        quoted: quoted_master.to_string(),
                        asked: master.to_string(),
                    });
                }

                // The body goes to *our* jetton wallet, which messages the
                // recipient's. `to` is the recipient's own address - their
                // jetton wallet is a real address that would not credit them.
                let body = neko_ton::jetton::transfer_body(
                    amount,
                    &to,
                    &from,
                    neko_ton::JETTON_FORWARD_AMOUNT,
                )?;
                // Coin travels with it to pay for both hops, and whatever is
                // unused comes back to `from`. This is why sending USDT here
                // costs GRAM.
                let inner = neko_ton::message::internal_message(
                    &jetton_wallet,
                    neko_ton::JETTON_TRANSFER_ATTACHED,
                    true,
                    Some(body),
                )?;
                sign_ton(req, p, inner, &key)
            }
            (Asset::Btc, ChainTxParams::Bitcoin(p)) => {
                let to = req.to.as_bitcoin()?;
                let amount = u64::try_from(req.amount.raw)
                    .map_err(|_| neko_btc::BtcError::AmountTooLarge)?;

                let mut outputs = vec![neko_btc::tx::output(&to, amount)];
                if let Some(change) = p.change {
                    outputs.push(neko_btc::tx::output(&p.change_to, change));
                }
                let mut tx = neko_btc::tx::Tx {
                    version: neko_btc::tx::VERSION,
                    inputs: p.inputs.iter().map(neko_btc::tx::input).collect(),
                    outputs,
                    locktime: 0,
                };

                // The fee is inputs minus outputs and nothing declares it, so
                // it is derived here and checked against what was quoted. This
                // is the guard against the classic loss on this chain: a
                // change output that was dropped somewhere between the quote
                // and the signature hands its whole value to a miner, and
                // nothing else in the flow would notice.
                let input_total: u64 = p.inputs.iter().map(|u| u.value).sum();
                match tx.fee(input_total) {
                    Some(actual) if actual == p.fee => {}
                    Some(actual) => {
                        return Err(CoreError::FeeMismatch {
                            quoted: p.fee,
                            actual,
                        })
                    }
                    None => return Err(CoreError::WrongChain),
                }

                neko_btc::tx::sign_p2wpkh(&mut tx, &p.inputs, &key)?;
                Ok(SignedTransfer {
                    id: tx.txid(),
                    raw: tx.serialize(),
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
        Ok(match chain {
            // SLIP-0010 over Ed25519. Deriving this with the BIP32 machinery
            // the other chains use would produce a perfectly valid key for an
            // address that holds nothing.
            ChainId::Solana => neko_hd::solana::private_key_at(&seed, index)?,
            // SLIP-0010 at m/44'/607'/0'. TON's own wallets use a different
            // scheme entirely - see `neko_hd::ton`.
            ChainId::Ton => neko_hd::ton::private_key_at(&seed, index)?,
            // BIP84, and the purpose level *is* the script type: deriving under
            // 44' and building a segwit script produces an address that is
            // valid, empty, and unspendable by the key that made it.
            ChainId::Bitcoin => neko_hd::bitcoin::private_key_at(&seed, 0, 0, index)?,
            ChainId::Tron | ChainId::Bsc | ChainId::Ethereum | ChainId::Polygon => {
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

/// Wrap an internal message in a signed external one.
///
/// The signature covers the hash of the body cell, and the internal message is
/// a reference inside it - so the destination and the amount are signed over by
/// being part of the tree.
fn sign_ton(
    req: &TransferRequest,
    p: &TonTxParams,
    inner: std::sync::Arc<neko_ton::cell::Cell>,
    key: &Zeroizing<[u8; 32]>,
) -> Result<SignedTransfer, CoreError> {
    use neko_ton::{message, wallet};

    let from = req.from.as_ton()?;

    // A TON wallet's address *is* the hash of the contract holding this public
    // key, so the key can be checked against the address before anything is
    // signed. Nowhere else here can do this: on the other chains a mismatched
    // key produces a valid signature by somebody else, and the failure only
    // shows up as a message the contract silently ignores - which looks exactly
    // like a transfer that vanished.
    let pk = neko_hd::ton::public_key(key);
    let derived = wallet::address_for(&pk)?;
    if derived != from {
        return Err(CoreError::WrongSigningKey {
            expected: from.to_string(),
            derived: derived.to_string(),
        });
    }

    let body = message::signing_body(
        wallet::DEFAULT_SUBWALLET_ID,
        p.valid_until,
        p.seqno,
        message::MODE_ORDINARY,
        inner,
    )?;
    let signed = message::signed_body(body, key)?;

    // The contract's code travels with the first message a wallet ever sends,
    // and only that one.
    let init = if p.deploy {
        Some(wallet::state_init(
            wallet::code()?,
            wallet::initial_data(&pk, wallet::DEFAULT_SUBWALLET_ID)?,
        )?)
    } else {
        None
    };
    let ext = message::external_message(&from, init, signed)?;

    Ok(SignedTransfer {
        // The *message* hash. A transaction's own hash does not exist until the
        // contract has run, so there is nothing else to hand somebody yet - and
        // explorers find a message by this.
        id: hex::encode(ext.hash()),
        raw: neko_ton::boc::serialize(&ext)?,
    })
}
