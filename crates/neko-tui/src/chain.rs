//! The async side of chain access.
//!
//! These functions run on the tokio runtime and return plain data. The session
//! and the private keys stay on the main thread; only bytes cross over.
//!
//! Two chains, two clients, and deliberately no shared abstraction over the
//! parts that genuinely differ. TRON charges bandwidth and energy and burns TRX
//! only for the shortfall; BNB Chain charges gas at a price the node quotes.
//! Flattening those into one "fee" would produce a number that is wrong on both.

use neko_core::{Asset, ChainAddress, ChainId, TransferRequest};
use neko_tron::TronGrid;

use crate::event::Quote;

/// Decimal places shown wherever a balance is scanned rather than signed.
/// Eight is finer than any figure a person reads at a glance and still shows a
/// real dust amount as something other than zero; transfers use the exact
/// value, never this.
pub const BALANCE_FRAC: u8 = 8;

/// Tag plus length prefix for `Transaction.raw_data`.
const RAW_DATA_FIELD_OVERHEAD: usize = 4;
/// Tag, length prefix and the 65-byte signature itself.
const SIGNATURE_FIELD_SIZE: usize = 67;
/// java-tron adds a flat allowance to every transaction's bandwidth charge
/// (`Constant.MAX_RESULT_SIZE_IN_TX`). Leaving it out understates the fee.
const MAX_RESULT_SIZE_IN_TX: usize = 64;

/// A connection to one chain.
pub enum Client {
    Tron(Box<TronGrid>),
    /// Both EVM chains. Which one it is comes from the client, because the
    /// chain id goes into every signature and carrying it separately is how
    /// the two get confused.
    Evm {
        rpc: Box<neko_evm::client::Rpc>,
        /// Only history needs this. Balances, fees and transfers all work
        /// from the plain RPC, so a missing key costs one screen rather than
        /// the chain.
        history_key: Option<String>,
    },
    Solana(Box<neko_solana::client::Rpc>),
    /// TON. The one client here that must ask a contract rather than the node:
    /// a wallet's sequence number and a jetton balance both live inside code,
    /// and are read by running it.
    Ton(Box<neko_ton::client::Toncenter>),
    Bitcoin {
        esplora: Box<neko_btc::client::Esplora>,
        /// Only the price uses this.
        ///
        /// Bitcoin has no exchange on it, so there is no pool on its own chain
        /// to ask what a coin is worth. Rather than add a price service - a new
        /// destination that would learn which addresses this wallet cares
        /// about - BTC is quoted from the BTCB pool on a chain we already talk
        /// to. Balances, fees and transfers never touch this.
        bsc: Box<neko_evm::client::Rpc>,
    },
}

impl Client {
    pub fn for_chain(chain: ChainId, url: Option<&str>, api_key: Option<String>) -> Self {
        match chain {
            ChainId::Tron => Client::Tron(Box::new(TronGrid::new(url, api_key))),
            // The TronGrid key is not a BscScan key; passing it here would only
            // be misleading. BNB Chain's public RPC needs no key at all.
            ChainId::Bsc | ChainId::Ethereum => Client::Evm {
                rpc: Box::new(neko_evm::client::Rpc::new(
                    chain.evm().expect("both EVM chains have parameters"),
                    url,
                )),
                history_key: api_key.filter(|k| !k.is_empty()),
            },
            ChainId::Solana => Client::Solana(Box::new(neko_solana::client::Rpc::new(url))),
            // toncenter's public endpoint rate-limits hard enough to matter,
            // and takes a key to raise that - so unlike Solana's, this url and
            // key travel together.
            ChainId::Ton => Client::Ton(Box::new(neko_ton::client::Toncenter::new(
                url,
                api_key.filter(|k| !k.is_empty()),
            ))),
            ChainId::Bitcoin => Client::Bitcoin {
                esplora: Box::new(neko_btc::client::Esplora::new(url)),
                bsc: Box::new(neko_evm::client::Rpc::new(neko_evm::BSC, None)),
            },
        }
    }

    /// The endpoint this client will use. For the same reason as
    /// `neko_solana::client::Rpc::url`: a setting that is saved but not passed
    /// on is indistinguishable from one that works, until the day it matters.
    pub fn endpoint(&self) -> Option<&str> {
        match self {
            Client::Solana(rpc) => Some(rpc.url()),
            Client::Bitcoin { esplora, .. } => Some(esplora.endpoint()),
            Client::Ton(api) => Some(api.endpoint()),
            _ => None,
        }
    }

    pub fn chain(&self) -> ChainId {
        match self {
            Client::Tron(_) => ChainId::Tron,
            Client::Evm { rpc, .. } => {
                if rpc.chain().chain_id == neko_evm::ETHEREUM.chain_id {
                    ChainId::Ethereum
                } else {
                    ChainId::Bsc
                }
            }
            Client::Solana(_) => ChainId::Solana,
            Client::Bitcoin { .. } => ChainId::Bitcoin,
            Client::Ton(_) => ChainId::Ton,
        }
    }
}

pub fn client(url: Option<&str>, api_key: Option<String>) -> TronGrid {
    TronGrid::new(url, api_key)
}

/// Work out what a transfer will actually cost, and gather the parameters
/// needed to build it.
pub async fn quote(c: &Client, req: &TransferRequest) -> Result<Quote, String> {
    match c {
        Client::Tron(t) => tron_quote(t, req).await,
        Client::Evm { rpc, .. } => evm_quote(rpc, req).await,
        Client::Solana(rpc) => solana_quote(rpc, req).await,
        Client::Bitcoin { esplora, .. } => bitcoin_quote(esplora, req).await,
        Client::Ton(api) => ton_quote(api, req).await,
    }
}

/// What a Bitcoin transfer costs, and which coins it will spend.
///
/// This is the quote that does the most work of the four. On an account chain a
/// fee is a property of the transaction; here it is a property of the *choice*,
/// because each coin selected adds about 68 virtual bytes to the fee it is
/// helping to pay. Selection and fee estimation are therefore one calculation,
/// and its result is carried into signing unchanged - substituting a coin later
/// would change the fee without anything saying so.
async fn bitcoin_quote(
    esplora: &neko_btc::client::Esplora,
    req: &TransferRequest,
) -> Result<Quote, String> {
    let from = req.from.as_bitcoin().map_err(|e| e.to_string())?;
    let to = req.to.as_bitcoin().map_err(|e| e.to_string())?;
    let amount = u64::try_from(req.amount.raw).map_err(|_| "amount is too large".to_string())?;

    let utxos = esplora.utxos(from).await.map_err(|e| e.to_string())?;
    let fee_rate = esplora
        .fee_rate(neko_btc::TARGET_BLOCKS)
        .await
        .map_err(|e| e.to_string())?;

    // Change returns to the same address the payment came from. That is what
    // the one-address-per-chain model already implies - this wallet shows one
    // receiving address per chain, so nothing is being newly linked.
    let selection =
        neko_btc::coins::select(&utxos, &to, amount, &from, fee_rate).map_err(|e| e.to_string())?;

    Ok(Quote::Bitcoin {
        fee_rate,
        balance: utxos.iter().map(|u| u.value).sum(),
        utxo_count: utxos.len(),
        selection: Box::new(selection),
        change_to: from,
    })
}

/// What a Solana transfer costs, and what it has to be told before it is built.
///
/// Two costs the other chains do not have:
///
/// * **Rent for the recipient's token account.** Tokens do not go to an
///   address, they go to an account derived from the address and the mint. If
///   the recipient has never held this token, that account does not exist, and
///   the sender pays about 0.002 SOL to open it - roughly forty times the fee.
/// * **A priority fee.** Solana drops rather than queues, so a transaction that
///   bids too low during congestion does not arrive late; it never arrives, and
///   the blockhash expires. The cluster is asked what recent blocks accepted.
async fn solana_quote(
    rpc: &neko_solana::client::Rpc,
    req: &TransferRequest,
) -> Result<Quote, String> {
    let from = req.from.as_solana().map_err(|e| e.to_string())?;
    let to = req.to.as_solana().map_err(|e| e.to_string())?;

    let (compute_unit_limit, create_recipient_account, rent) = match req.asset {
        Asset::Sol => (neko_solana::COMPUTE_UNITS_SOL, false, 0),
        Asset::SplToken { mint, decimals } => {
            // Same reasoning as the other two chains': ask the chain what this
            // token is before trusting a built-in address. Here the number that
            // matters is the precision, because the program checks it and a
            // mismatch is a factor of a thousand or a million.
            let chain_decimals = rpc
                .mint_decimals(mint)
                .await
                .map_err(|e| format!("could not verify the mint {mint}: {e}"))?;
            if chain_decimals != decimals {
                return Err(format!(
                    "mint {mint} reports {chain_decimals} decimals, expected {decimals} - refusing to send"
                ));
            }
            let exists = rpc
                .has_token_account(to, mint)
                .await
                .map_err(|e| e.to_string())?;
            if exists {
                (neko_solana::COMPUTE_UNITS_TOKEN, false, 0)
            } else {
                // Asked rather than assumed: it is a cluster parameter.
                let rent = rpc
                    .token_account_rent()
                    .await
                    .unwrap_or(neko_solana::TOKEN_ACCOUNT_RENT);
                (neko_solana::COMPUTE_UNITS_TOKEN_WITH_ATA, true, rent)
            }
        }
        _ => return Err("that asset is not on Solana".into()),
    };

    let compute_unit_price = rpc.priority_fee(&[from]).await.unwrap_or(0);
    // Fetched so the quote is complete, but not the one that gets signed: see
    // `AppEvent::Blockhash`. By the time a password has been accepted this is
    // usually stale.
    let recent_blockhash = rpc
        .latest_blockhash()
        .await
        .map_err(|e| e.to_string())?
        .hash;
    let sol_balance = rpc.balance(from).await.ok();

    Ok(Quote::Solana {
        params: neko_solana::tx::TxParams {
            recent_blockhash,
            compute_unit_limit,
            compute_unit_price,
            create_recipient_account,
        },
        sol_balance,
        sending_native: matches!(req.asset, Asset::Sol),
        amount: u64::try_from(req.amount.raw).map_err(|_| "amount is too large".to_string())?,
        rent,
    })
}

/// What a TON transfer costs, and what it has to be told before it is built.
///
/// Three things here have no equivalent on the account chains:
///
/// * **The wallet may not exist.** An address can hold GRAM before its contract
///   is deployed, and the first message out has to carry that contract's code.
///   Getting this wrong in either direction is fatal to the message: code on a
///   deployed wallet is rejected, and no code on an undeployed one leaves
///   nothing that can run.
/// * **`seqno` instead of a nonce.** A stale one is not rejected as a double
///   spend; the message is ignored, which looks exactly like a transfer that
///   vanished.
/// * **A token transfer needs GRAM attached to it**, to pay for the hops
///   between two jetton wallet contracts. Most of it comes back, which is why
///   the quote keeps it apart from the fee rather than adding the two.
async fn ton_quote(
    api: &neko_ton::client::Toncenter,
    req: &TransferRequest,
) -> Result<Quote, String> {
    let from = req.from.as_ton().map_err(|e| e.to_string())?;
    let state = api.wallet_state(&from).await.map_err(|e| e.to_string())?;

    let (jetton_wallet, attached) = match req.asset {
        Asset::Gram => (None, 0),
        Asset::Jetton { master, decimals } => {
            // Same reasoning as the other four chains': ask the contract what
            // it is before trusting a built-in address. Only the precision can
            // be had here - see `neko_ton::jetton::decimals_from_content` for
            // why the symbol is not fetched.
            let chain_decimals = api
                .jetton_decimals(&master)
                .await
                .map_err(|e| format!("could not verify the jetton master {master}: {e}"))?;
            if chain_decimals != decimals {
                return Err(format!(
                    "jetton master {master} reports {chain_decimals} decimals, expected {decimals} - refusing to send"
                ));
            }
            // Where *our* balance of it lives. Asked of the master rather than
            // derived here, because the code that goes into that address
            // belongs to the token.
            let wallet = api
                .jetton_wallet(&from, &master)
                .await
                .map_err(|e| e.to_string())?;

            // And the contract at that address is asked whose it is. A wallet
            // that does not exist yet holds nothing, so there is nothing to
            // send and nothing to check; one that exists and belongs to
            // somebody else is a node answering with the wrong address, and
            // the transfer must not be built against it.
            if let Some(d) = api
                .jetton_wallet_data(&wallet)
                .await
                .map_err(|e| e.to_string())?
            {
                if d.owner != from || d.master != master {
                    return Err(format!(
                        "{wallet} says it holds {} for {}, not {master} for {from} - refusing to send",
                        d.master, d.owner
                    ));
                }
            }
            (Some((master, wallet)), neko_ton::JETTON_TRANSFER_ATTACHED)
        }
        _ => return Err("that asset is not on TON".into()),
    };

    Ok(Quote::Ton {
        params: Box::new(neko_core::TonTxParams {
            seqno: state.seqno,
            valid_until: valid_until(),
            deploy: !state.deployed,
            jetton_wallet,
        }),
        gram_balance: Some(state.balance),
        sending_native: matches!(req.asset, Asset::Gram),
        amount: u128::try_from(req.amount.raw).map_err(|_| "amount is too large".to_string())?,
        // Not quoted: TON's fees are small, fixed in shape, and the chain
        // charges what it charges. The figure is an upper bound used to check a
        // balance covers the transfer - a node estimate would be more precise
        // and would also expire, and being a fraction of a cent generous costs
        // less than a message that fails for being a nanoton short.
        fee: neko_ton::FEE_TRANSFER,
        attached,
    })
}

/// When a signed message stops being valid.
///
/// TON signs over this, which the account chains have no equivalent of: a
/// message that sat in somebody's hands cannot be replayed a day later. Two
/// minutes is the wallet standard, and is long enough for a password prompt.
fn valid_until() -> u32 {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    (now + neko_ton::message::VALID_FOR_SECS as u64) as u32
}

/// A blockhash for the signature about to be taken.
pub async fn latest_blockhash(c: &Client) -> Result<[u8; 32], String> {
    match c {
        Client::Solana(rpc) => rpc
            .latest_blockhash()
            .await
            .map(|b| b.hash)
            .map_err(|e| e.to_string()),
        // Nothing on the other chains expires this fast; their parameters are
        // taken once, at quote time, and stay good.
        _ => Err("that chain does not use blockhashes".into()),
    }
}

/// TRON has no flat fee. A transaction spends bandwidth, a contract call spends
/// energy, and only the part the account *cannot* cover gets paid for by
/// burning TRX. Both the requirement and the account's holdings come from the
/// chain: energy is simulated against this exact transfer, and the burn prices
/// are governance parameters that change.
async fn tron_quote(c: &TronGrid, req: &TransferRequest) -> Result<Quote, String> {
    let from = req.from.as_tron().map_err(|e| e.to_string())?;
    let to = req.to.as_tron().map_err(|e| e.to_string())?;
    let fee_limit = req
        .asset
        .tron_fee_limit()
        .ok_or_else(|| "that asset is not on TRON".to_string())?;
    let params = c.tx_params(fee_limit).await.map_err(|e| e.to_string())?;

    let (energy, recipient_is_new, raw_len) = match req.asset {
        Asset::Trx => {
            let raw = neko_tron::tx::build_trx_transfer(from, to, req.amount.raw as i64, &params)
                .map_err(|e| e.to_string())?;
            (neko_tron::EnergyEstimate::default(), false, raw.len())
        }
        Asset::Trc20 { contract, decimals } => {
            // Prove on-chain that this really is the token we think it is,
            // before signing anything.
            let (symbol, chain_decimals) = c
                .verify_usdt(contract)
                .await
                .map_err(|e| format!("could not verify the token contract {contract}: {e}"))?;
            if chain_decimals != decimals {
                return Err(format!(
                    "token contract {contract} reports {chain_decimals} decimals, expected {decimals} - refusing to send"
                ));
            }
            if symbol != "USDT" {
                return Err(format!(
                    "token contract {contract} reports symbol {symbol:?}, not USDT - refusing to send"
                ));
            }

            let calldata = req
                .calldata()
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            let energy = c
                .estimate_trc20_energy(contract, from, &calldata)
                .await
                .map_err(|e| e.to_string())?;
            let new = c
                .trc20_balance(contract, to)
                .await
                .map(|b| b == 0)
                .unwrap_or(false);
            let raw = neko_tron::tx::build_trc20_transfer(
                from,
                contract,
                to,
                req.amount.raw as u128,
                &params,
            )
            .map_err(|e| e.to_string())?;
            (energy, new, raw.len())
        }
        _ => return Err("that asset is not on TRON".into()),
    };

    // Bandwidth is charged per byte of the signed transaction, plus a flat
    // allowance the chain reserves for the result.
    let signed_len = raw_len + RAW_DATA_FIELD_OVERHEAD + SIGNATURE_FIELD_SIZE;
    let bandwidth_needed = (signed_len + MAX_RESULT_SIZE_IN_TX) as i64;

    // A failure here must not block the transfer, but it must not be laundered
    // into a number either: `None` travels to the UI as "unknown", which is
    // rendered differently from "zero".
    let resources = c.account_resources(from).await.ok();
    let prices = c.prices().await.ok();

    Ok(Quote::Tron {
        params: Box::new(params),
        energy,
        bandwidth_needed,
        resources,
        prices,
        recipient_is_new,
    })
}

/// BNB Chain charges gas: a quantity the node estimates against this exact
/// call, times a price it quotes. Unlike TRON there is no allowance to cover
/// part of it - the fee is always paid in BNB, so a wallet holding only USDT
/// cannot send that USDT. Saying so before the attempt is the useful part.
async fn evm_quote(rpc: &neko_evm::client::Rpc, req: &TransferRequest) -> Result<Quote, String> {
    let from = req.from.as_evm().map_err(|e| e.to_string())?;

    let chain = rpc.chain();
    let (to, value, data) = match req.asset {
        Asset::Bnb | Asset::Eth => (
            req.to.as_evm().map_err(|e| e.to_string())?,
            req.amount.raw as u128,
            Vec::new(),
        ),
        Asset::Bep20 { contract, decimals } | Asset::Erc20 { contract, decimals } => {
            // Same reasoning as TRON's: ask the contract what it is before
            // trusting a built-in address.
            let (symbol, chain_decimals) = rpc
                .verify_token(contract)
                .await
                .map_err(|e| format!("could not verify the token contract {contract}: {e}"))?;
            if chain_decimals != decimals {
                return Err(format!(
                    "token contract {contract} reports {chain_decimals} decimals, expected {decimals} - refusing to send"
                ));
            }
            if symbol != "USDT" {
                return Err(format!(
                    "token contract {contract} reports symbol {symbol:?}, not USDT - refusing to send"
                ));
            }
            let data = req
                .calldata()
                .map_err(|e| e.to_string())?
                .unwrap_or_default();
            (contract, 0u128, data)
        }
        _ => return Err(format!("that asset is not on chain {}", chain.chain_id)),
    };

    let params = rpc
        .tx_params(from, to, value, &data)
        .await
        .map_err(|e| e.to_string())?;
    // What pays the fee, which is the chain's own coin regardless of what is
    // being sent.
    let native_balance = rpc.balance(from).await.ok();

    Ok(Quote::Evm {
        chain,
        params,
        native_balance,
        sending_native: matches!(req.asset, Asset::Bnb | Asset::Eth),
        amount: req.amount.raw as u128,
    })
}

/// What one unit of the chain's native coin is worth, in USDT.
///
/// Quoted from a swap pool on the chain we are already connected to, so the
/// wallet still talks to the node you point it at and to nothing else. The
/// unit is USDT rather than dollars, and the interface says so: they track
/// each other closely but are not the same thing, and this is a pool's spot
/// price rather than a market average.
/// Returned at [`neko_core::PRICE_SCALE`], **not** in the units the pool
/// quoted.
///
/// The pools answer in their own chain's USDT, and that is six decimals on
/// TRON and eighteen on BNB Chain. Storing either figure directly would value
/// BNB a million million times too high - the same trap as the balances, and
/// the reason the scale is normalised here, once, rather than at each use.
pub async fn native_price(c: &Client) -> Result<i128, String> {
    let (raw, quoted_decimals) = match c {
        Client::Tron(t) => (
            t.trx_price_in_usdt().await.map_err(|e| e.to_string())?,
            neko_tron::USDT_DECIMALS,
        ),
        Client::Evm { rpc, .. } => (
            rpc.native_price_in_usdt()
                .await
                .map_err(|e| e.to_string())?,
            // Six on Ethereum, eighteen on BNB Chain: the pool answers in its
            // own chain's USDT, and storing either figure directly would value
            // one of them a million million times wrong.
            rpc.chain().usdt_decimals,
        ),
        // Already normalised by the pool reader, which is handed the scale it
        // should answer at rather than a chain's own USDT precision.
        Client::Solana(rpc) => (
            neko_solana::price::sol_in_usdt(rpc, neko_core::PRICE_SCALE)
                .await
                .map_err(|e| e.to_string())? as u128,
            neko_core::PRICE_SCALE,
        ),
        // Read on BNB Chain, because Bitcoin has no exchange on it. The figure
        // is BTCB's, and the interface labels it as such rather than passing it
        // off as a spot BTC price.
        Client::Bitcoin { bsc, .. } => (
            bsc.btcb_price_in_usdt().await.map_err(|e| e.to_string())?,
            neko_evm::BSC.usdt_decimals,
        ),
        // Already normalised by the pool reader, like Solana's.
        Client::Ton(api) => (
            neko_ton::price::gram_in_usdt(api, neko_core::PRICE_SCALE)
                .await
                .map_err(|e| e.to_string())? as u128,
            neko_core::PRICE_SCALE,
        ),
    };
    Ok(rescale(
        raw as i128,
        quoted_decimals,
        neko_core::PRICE_SCALE,
    ))
}

/// Move a fixed-point figure between decimal scales.
fn rescale(v: i128, from: u8, to: u8) -> i128 {
    match to.cmp(&from) {
        std::cmp::Ordering::Equal => v,
        std::cmp::Ordering::Greater => v
            .checked_mul(10i128.pow((to - from) as u32))
            .unwrap_or(i128::MAX),
        std::cmp::Ordering::Less => v / 10i128.pow((from - to) as u32),
    }
}

/// Hand the signed bytes to the chain, and answer with what this transfer will
/// be called.
///
/// `local_id` is the identifier derived while signing. Five chains ignore it in
/// favour of the node's own answer; TON uses it, because there the node returns
/// an acknowledgement and the message hash is the only name the transfer has
/// until a contract has run.
pub async fn broadcast(c: &Client, raw: Vec<u8>, local_id: String) -> Result<String, String> {
    match c {
        Client::Tron(t) => t.broadcast(&raw).await.map_err(|e| e.to_string()),
        Client::Evm { rpc, .. } => rpc.send_raw(&raw).await.map_err(|e| e.to_string()),
        Client::Solana(rpc) => rpc.send(&raw).await.map_err(|e| e.to_string()),
        Client::Bitcoin { esplora, .. } => esplora.broadcast(&raw).await.map_err(|e| e.to_string()),
        Client::Ton(api) => api
            .send(&raw)
            .await
            .map(|_| local_id)
            .map_err(|e| e.to_string()),
    }
}

/// Balances in the shape the cache stores: (symbol, decimals, amount).
pub async fn wallet_assets(
    c: &Client,
    addr: ChainAddress,
) -> Result<Vec<(String, u8, i128)>, String> {
    match c {
        Client::Tron(t) => {
            let a = addr.as_tron().map_err(|e| e.to_string())?;
            let usdt = neko_tron::usdt_address();
            let trx = t.trx_balance(a).await.map_err(|e| e.to_string())?;
            // A token lookup failing must not discard the native figure we
            // already have.
            let usdt_bal = t.trc20_balance(usdt, a).await.unwrap_or(0);
            Ok(vec![
                ("TRX".to_string(), 6, trx as i128),
                ("USDT".to_string(), 6, usdt_bal as i128),
            ])
        }
        Client::Evm { rpc: r, .. } => {
            let chain = r.chain();
            let a = addr.as_evm().map_err(|e| e.to_string())?;
            let native = r.balance(a).await.map_err(|e| e.to_string())?;
            let usdt_bal = r.token_balance(chain.usdt_address(), a).await.unwrap_or(0);
            Ok(vec![
                (
                    chain.native_symbol.to_string(),
                    chain.native_decimals,
                    native as i128,
                ),
                // Eighteen decimals on BNB Chain, six on Ethereum. The number
                // travels with the balance for exactly this reason.
                ("USDT".to_string(), chain.usdt_decimals, usdt_bal as i128),
            ])
        }
        Client::Solana(rpc) => {
            let a = addr.as_solana().map_err(|e| e.to_string())?;
            let mint = neko_solana::usdt_mint();
            let sol = rpc.balance(a).await.map_err(|e| e.to_string())?;
            // An address that has never held this token has no account for it,
            // which reads as zero here - correct, and different from a failed
            // lookup, which also reads as zero rather than discarding the SOL
            // figure we already have.
            let usdt = rpc
                .token_balance(a, mint)
                .await
                .ok()
                .flatten()
                .map(|b| b.amount)
                .unwrap_or(0);
            Ok(vec![
                ("SOL".to_string(), neko_solana::SOL_DECIMALS, sol as i128),
                // Six here, six on TRON, eighteen on BNB Chain. One token name,
                // three precisions.
                ("USDT".to_string(), neko_solana::USDT_DECIMALS, usdt as i128),
            ])
        }
        Client::Ton(api) => {
            let a = addr.as_ton().map_err(|e| e.to_string())?;
            let master = neko_ton::usdt_master();
            let state = api.wallet_state(&a).await.map_err(|e| e.to_string())?;
            // A jetton balance lives in a contract of its own, one per holder
            // per token, and an address that has never held USDT has no such
            // contract. That reads as zero here - correct, and the same figure
            // a failed lookup gives, which must not discard the GRAM we have.
            let usdt = match api.jetton_wallet(&a, &master).await {
                Ok(w) => api.jetton_balance(&w).await.unwrap_or(0),
                Err(_) => 0,
            };
            Ok(vec![
                (
                    "GRAM".to_string(),
                    neko_ton::GRAM_DECIMALS,
                    state.balance as i128,
                ),
                // Six here, six on TRON, Ethereum and Solana, eighteen on BNB
                // Chain. One token name, two precisions.
                ("USDT".to_string(), neko_ton::USDT_DECIMALS, usdt as i128),
            ])
        }
        Client::Bitcoin { esplora, .. } => {
            // There is no balance to ask for. A wallet holds unspent outputs,
            // and the balance is their sum - which is also why an address that
            // has never been paid returns an empty list rather than a zero.
            let a = addr.as_bitcoin().map_err(|e| e.to_string())?;
            let utxos = esplora.utxos(a).await.map_err(|e| e.to_string())?;
            let total: u64 = utxos.iter().map(|u| u.value).sum();
            Ok(vec![(
                "BTC".to_string(),
                neko_btc::BTC_DECIMALS,
                total as i128,
            )])
        }
    }
}

/// Fetch both history feeds and merge them, newest first.
pub async fn history(
    c: &Client,
    addr: ChainAddress,
    limit: u32,
) -> Result<Vec<neko_tron::HistoryEntry>, String> {
    match c {
        Client::Tron(t) => {
            let a = addr.as_tron().map_err(|e| e.to_string())?;
            let usdt = neko_tron::usdt_address();
            let owned = [a];
            let mut all = Vec::new();

            let trx = t.history_trx(a, limit).await.map_err(|e| e.to_string())?;
            all.extend(neko_tron::history::parse_trx(&trx, &owned));

            // A TRC20 failure must not discard the TRX half we already have.
            match t.history_trc20(a, usdt, limit).await {
                Ok(v) => all.extend(neko_tron::history::parse_trc20(&v, &owned)),
                Err(e) => {
                    if all.is_empty() {
                        return Err(e.to_string());
                    }
                }
            }
            Ok(neko_tron::history::merge(all))
        }
        Client::Evm {
            rpc, history_key, ..
        } => {
            // A node's RPC cannot answer "what has this address done"; that
            // needs an index. Without a key, say so - an empty list would
            // read as "you have never used this address".
            let Some(key) = history_key else {
                return Err(neko_i18n::t(neko_i18n::Key::History_NeedsIndexer).to_string());
            };
            let a = addr.as_evm().map_err(|e| e.to_string())?;
            let chain = rpc.chain();
            let rows = neko_evm::history::Bsctrace::new(chain, key)
                .transfers(a, chain.usdt_address(), limit as usize)
                .await
                .map_err(|e| e.to_string())?;

            let mine = a.to_string().to_ascii_lowercase();
            Ok(rows
                .into_iter()
                .map(|t| {
                    // Direction is decided here, from the address we asked
                    // about, rather than trusting a field in the reply.
                    let incoming = t.to.eq_ignore_ascii_case(&mine);
                    neko_tron::HistoryEntry {
                        txid: t.hash,
                        block_ts: t.block_ts,
                        symbol: t.symbol,
                        decimals: t.decimals,
                        amount: t.amount,
                        direction: if incoming {
                            neko_tron::Direction::In
                        } else {
                            neko_tron::Direction::Out
                        },
                        counterparty: if incoming { t.from } else { t.to },
                        status: if t.success {
                            neko_tron::TxStatus::Success
                        } else {
                            neko_tron::TxStatus::Failed
                        },
                    }
                })
                .collect())
        }
        Client::Solana(rpc) => {
            // No indexer needed here: the cluster records what every account
            // held before and after each transaction, so a transfer is a
            // difference rather than something to be parsed out of a program's
            // instructions.
            let a = addr.as_solana().map_err(|e| e.to_string())?;
            let rows = rpc
                .transfers(a, neko_solana::usdt_mint(), limit as usize)
                .await
                .map_err(|e| e.to_string())?;
            Ok(rows
                .into_iter()
                .map(|t| neko_tron::HistoryEntry {
                    txid: t.signature,
                    block_ts: t.block_time * 1000,
                    symbol: t.symbol,
                    decimals: t.decimals,
                    amount: t.amount,
                    direction: match t.direction {
                        neko_solana::history::Direction::In => neko_tron::Direction::In,
                        neko_solana::history::Direction::Out => neko_tron::Direction::Out,
                    },
                    counterparty: t.counterparty,
                    status: if t.failed {
                        neko_tron::TxStatus::Failed
                    } else {
                        neko_tron::TxStatus::Success
                    },
                })
                .collect())
        }
        Client::Ton(api) => {
            // No indexer needed, and no token rows either: a jetton movement is
            // an opcode inside a message between two contracts neither of which
            // is this address, so it does not appear in what this address did.
            // Left out rather than guessed at - the balance still shows what is
            // held.
            let a = addr.as_ton().map_err(|e| e.to_string())?;
            let raw = api
                .transactions(&a, limit)
                .await
                .map_err(|e| e.to_string())?;
            let rows = neko_ton::history::parse(&raw, &a, neko_ton::GRAM_DECIMALS, "GRAM")
                .map_err(|e| e.to_string())?;
            Ok(rows
                .into_iter()
                .map(|t| neko_tron::HistoryEntry {
                    txid: t.hash,
                    block_ts: t.block_time * 1000,
                    symbol: t.symbol,
                    decimals: t.decimals,
                    amount: t.amount,
                    direction: match t.direction {
                        neko_ton::history::Direction::In => neko_tron::Direction::In,
                        neko_ton::history::Direction::Out => neko_tron::Direction::Out,
                    },
                    counterparty: t.counterparty,
                    // A transaction in this list has already executed. There is
                    // no pending state to report: a message that has not been
                    // processed is not here at all.
                    status: neko_tron::TxStatus::Success,
                })
                .collect())
        }
        Client::Bitcoin { esplora, .. } => {
            let a = addr.as_bitcoin().map_err(|e| e.to_string())?;
            let raw = esplora.address_txs(a).await.map_err(|e| e.to_string())?;
            let rows = neko_btc::history::parse(&raw, &a).map_err(|e| e.to_string())?;
            Ok(rows
                .into_iter()
                .take(limit as usize)
                .map(|t| neko_tron::HistoryEntry {
                    txid: t.txid,
                    block_ts: t.block_time * 1000,
                    symbol: "BTC".into(),
                    decimals: neko_btc::BTC_DECIMALS,
                    amount: t.amount,
                    direction: match t.direction {
                        neko_btc::history::Direction::In => neko_tron::Direction::In,
                        neko_btc::history::Direction::Out => neko_tron::Direction::Out,
                    },
                    counterparty: t.counterparty,
                    // Unconfirmed is not failed - it is still in the mempool
                    // and can still be replaced, which the pending state says
                    // and a success would not.
                    status: if t.confirmed {
                        neko_tron::TxStatus::Success
                    } else {
                        neko_tron::TxStatus::Pending
                    },
                })
                .collect())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The failure this guards against valued BNB at a million million times
    /// its price, because PancakeSwap quotes in eighteen decimals and TRON's
    /// USDT has six.
    #[test]
    fn prices_are_normalised_to_one_scale() {
        // 1 BNB = 723.586483 USDT, as PancakeSwap states it.
        let pancake = 723_586_483_000_000_000_000i128;
        assert_eq!(rescale(pancake, 18, 6), 723_586_483);

        // SunSwap already speaks in six.
        assert_eq!(rescale(330_255, 6, 6), 330_255);

        // And the other direction, for completeness.
        assert_eq!(rescale(1_000_000, 6, 18), 10i128.pow(18));
        // Absurd input saturates rather than wrapping into a plausible price.
        assert_eq!(rescale(i128::MAX, 6, 18), i128::MAX);
    }
}
