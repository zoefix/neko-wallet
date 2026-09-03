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

/// Decimal places shown on the assets screen. Eight is finer than any figure
/// a person reads at a glance and still shows a real dust amount as something
/// other than zero; transfers use the exact value, never this.
const BALANCE_FRAC: u8 = 8;

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
    Bsc {
        rpc: Box<neko_evm::client::Rpc>,
        /// Only history needs this. Balances, fees and transfers all work
        /// from the plain RPC, so a missing key costs one screen rather than
        /// the chain.
        history_key: Option<String>,
    },
}

impl Client {
    pub fn for_chain(chain: ChainId, url: Option<&str>, api_key: Option<String>) -> Self {
        match chain {
            ChainId::Tron => Client::Tron(Box::new(TronGrid::new(url, api_key))),
            // The TronGrid key is not a BscScan key; passing it here would only
            // be misleading. BNB Chain's public RPC needs no key at all.
            ChainId::Bsc => Client::Bsc {
                rpc: Box::new(neko_evm::client::Rpc::new(url)),
                history_key: api_key.filter(|k| !k.is_empty()),
            },
        }
    }

    pub fn chain(&self) -> ChainId {
        match self {
            Client::Tron(_) => ChainId::Tron,
            Client::Bsc { .. } => ChainId::Bsc,
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
        Client::Bsc { rpc, .. } => bsc_quote(rpc, req).await,
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
async fn bsc_quote(rpc: &neko_evm::client::Rpc, req: &TransferRequest) -> Result<Quote, String> {
    let from = req.from.as_evm().map_err(|e| e.to_string())?;

    let (to, value, data) = match req.asset {
        Asset::Bnb => (
            req.to.as_evm().map_err(|e| e.to_string())?,
            req.amount.raw as u128,
            Vec::new(),
        ),
        Asset::Bep20 { contract, decimals } => {
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
        _ => return Err("that asset is not on BNB Chain".into()),
    };

    let params = rpc
        .tx_params(from, to, value, &data)
        .await
        .map_err(|e| e.to_string())?;
    // What pays the fee, which is BNB regardless of what is being sent.
    let bnb_balance = rpc.balance(from).await.ok();

    Ok(Quote::Bsc {
        params,
        bnb_balance,
        sending_native: matches!(req.asset, Asset::Bnb),
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
        Client::Bsc { rpc, .. } => (
            rpc.bnb_price_in_usdt().await.map_err(|e| e.to_string())?,
            neko_evm::USDT_DECIMALS,
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

pub async fn broadcast(c: &Client, raw: Vec<u8>) -> Result<String, String> {
    match c {
        Client::Tron(t) => t.broadcast(&raw).await.map_err(|e| e.to_string()),
        Client::Bsc { rpc, .. } => rpc.send_raw(&raw).await.map_err(|e| e.to_string()),
    }
}

/// Native and USDT balances, formatted for display.
pub async fn balances(c: &Client, addr: ChainAddress) -> Result<Vec<(String, String)>, String> {
    let rows = wallet_assets(c, addr).await?;
    Ok(rows
        .into_iter()
        .map(|(sym, dec, amt)| {
            // Capped: eighteen decimals is a row of zeros nobody reads, and it
            // pushed the column off the screen. The cap never renders a
            // non-empty balance as zero - it shows `<0.00000001` instead - and
            // the send screen still works from the exact figure.
            (
                sym,
                neko_core::Amount::new(amt, dec).to_display_string_max(BALANCE_FRAC),
            )
        })
        .collect())
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
        Client::Bsc { rpc: r, .. } => {
            let a = addr.as_evm().map_err(|e| e.to_string())?;
            let usdt = neko_evm::usdt_address();
            let bnb = r.balance(a).await.map_err(|e| e.to_string())?;
            let usdt_bal = r.token_balance(usdt, a).await.unwrap_or(0);
            Ok(vec![
                ("BNB".to_string(), 18, bnb as i128),
                // Eighteen decimals here, six on TRON. The number travels with
                // the balance for exactly this reason.
                ("USDT".to_string(), 18, usdt_bal as i128),
            ])
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
        Client::Bsc { history_key, .. } => {
            // A node's RPC cannot answer "what has this address done"; that
            // needs an index. Without a key, say so - an empty list would
            // read as "you have never used this address".
            let Some(key) = history_key else {
                return Err(neko_i18n::t(neko_i18n::Key::History_NeedsIndexer).to_string());
            };
            let a = addr.as_evm().map_err(|e| e.to_string())?;
            let rows = neko_evm::history::Bsctrace::new(key)
                .transfers(a, neko_evm::usdt_address(), limit as usize)
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
