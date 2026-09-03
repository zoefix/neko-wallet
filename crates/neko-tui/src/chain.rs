//! The async side of chain access.
//!
//! These functions run on the tokio runtime and return plain data. The session
//! and the private keys stay on the main thread; only bytes cross over.

use neko_core::{Asset, TransferRequest};
use neko_hd::Address;
use neko_tron::TronGrid;

use crate::event::Quote;

/// Tag plus length prefix for `Transaction.raw_data`.
const RAW_DATA_FIELD_OVERHEAD: usize = 4;
/// Tag, length prefix and the 65-byte signature itself.
const SIGNATURE_FIELD_SIZE: usize = 67;
/// java-tron adds a flat allowance to every transaction's bandwidth charge
/// (`Constant.MAX_RESULT_SIZE_IN_TX`). Leaving it out understates the fee.
const MAX_RESULT_SIZE_IN_TX: usize = 64;

pub fn client(url: Option<&str>, api_key: Option<String>) -> TronGrid {
    TronGrid::new(url, api_key)
}

/// Fetch a block reference and work out what the transfer will actually cost.
///
/// TRON has no flat fee. A transaction spends bandwidth, a contract call spends
/// energy, and only the part the account *cannot* cover gets paid for by
/// burning TRX. Both the requirement and the account's holdings come from the
/// chain: energy is simulated against this exact transfer (the same USDT
/// transfer costs wildly different amounts depending on whether the recipient
/// already holds the token), and the burn prices are governance parameters that
/// change.
pub async fn quote(c: &TronGrid, req: &TransferRequest) -> Result<Quote, String> {
    let params = c
        .tx_params(req.asset.fee_limit())
        .await
        .map_err(|e| e.to_string())?;

    let (energy, recipient_is_new, raw_len) = match req.asset {
        Asset::Trx => {
            let raw =
                neko_tron::tx::build_trx_transfer(req.from, req.to, req.amount.raw as i64, &params)
                    .map_err(|e| e.to_string())?;
            (neko_tron::EnergyEstimate::default(), false, raw.len())
        }
        Asset::Trc20 { contract, decimals } => {
            // Prove on-chain that this really is the token we think it is,
            // before signing anything. A custom node could otherwise point the
            // built-in contract address at something else, and funds sent to
            // the wrong contract cannot be recovered.
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
                .estimate_trc20_energy(contract, req.from, &calldata)
                .await
                .map_err(|e| e.to_string())?;
            // A zero balance means the transfer has to create a storage slot.
            let new = c
                .trc20_balance(contract, req.to)
                .await
                .map(|b| b == 0)
                .unwrap_or(false);
            let raw = neko_tron::tx::build_trc20_transfer(
                req.from,
                contract,
                req.to,
                req.amount.raw as u128,
                &params,
            )
            .map_err(|e| e.to_string())?;
            (energy, new, raw.len())
        }
    };

    // Bandwidth is charged per byte of the signed transaction, plus a flat
    // allowance the chain reserves for the result. Measured against a real
    // mainnet transfer: the estimate without that allowance came out at 282
    // while the chain actually charged 345 — the missing 64 is
    // `MAX_RESULT_SIZE_IN_TX`, which java-tron adds to every transaction.
    let signed_len = raw_len + RAW_DATA_FIELD_OVERHEAD + SIGNATURE_FIELD_SIZE;
    let bandwidth_needed = (signed_len + MAX_RESULT_SIZE_IN_TX) as i64;

    // A failure here must not block the transfer, but it must not be laundered
    // into a number either: `None` travels to the UI as "unknown", which is
    // rendered differently from "zero". Without an API key these calls hit the
    // public rate limit intermittently, so this path is taken in practice.
    let resources = c.account_resources(req.from).await.ok();
    let prices = c.prices().await.ok();

    Ok(Quote {
        params,
        energy,
        bandwidth_needed,
        resources,
        prices,
        recipient_is_new,
    })
}

pub async fn broadcast(c: &TronGrid, raw_tx: Vec<u8>) -> Result<String, String> {
    c.broadcast(&raw_tx).await.map_err(|e| e.to_string())
}

/// TRX and USDT balances, formatted for display.
pub async fn balances(
    c: &TronGrid,
    addr: Address,
    usdt: Address,
) -> Result<Vec<(String, String)>, String> {
    let trx = c.trx_balance(addr).await.map_err(|e| e.to_string())?;
    let usdt_bal = c.trc20_balance(usdt, addr).await.unwrap_or(0);
    Ok(vec![
        (
            "TRX".into(),
            neko_core::Amount::new(trx as i128, 6).to_display_string(),
        ),
        (
            "USDT".into(),
            neko_core::Amount::new(usdt_bal as i128, 6).to_display_string(),
        ),
    ])
}

/// Fetch both history feeds and merge them, newest first.
///
/// The v1 endpoints are an indexer, not part of the node protocol: a
/// self-hosted full node will not serve them, and the error says so plainly
/// rather than looking like an empty history.
pub async fn history(
    c: &TronGrid,
    addr: Address,
    usdt: Address,
    limit: u32,
) -> Result<Vec<neko_tron::HistoryEntry>, String> {
    let owned = [addr];
    let mut all = Vec::new();

    let trx = c
        .history_trx(addr, limit)
        .await
        .map_err(|e| e.to_string())?;
    all.extend(neko_tron::history::parse_trx(&trx, &owned));

    // A TRC20 failure must not discard the TRX half we already have.
    match c.history_trc20(addr, usdt, limit).await {
        Ok(v) => all.extend(neko_tron::history::parse_trc20(&v, &owned)),
        Err(e) => {
            if all.is_empty() {
                return Err(e.to_string());
            }
        }
    }
    Ok(neko_tron::history::merge(all))
}

/// Balances for one wallet, in the shape the cache stores.
pub async fn wallet_assets(
    c: &TronGrid,
    addr: Address,
    usdt: Address,
) -> Result<Vec<(String, u8, i128)>, String> {
    let trx = c.trx_balance(addr).await.map_err(|e| e.to_string())?;
    // A USDT lookup failing must not discard the TRX figure we already have.
    let usdt_bal = c.trc20_balance(usdt, addr).await.unwrap_or(0);
    Ok(vec![
        ("TRX".to_string(), 6, trx as i128),
        ("USDT".to_string(), 6, usdt_bal as i128),
    ])
}
