//! USDC on every chain that carries it, end to end, against mainnet.
//!
//! For each chain: read a real holder's balances through the same path the
//! wallet list uses, read its history, then quote a 0.01 USDC transfer from
//! that holder - which asks the contract for its name and precision before
//! anything could be signed. Nothing is signed or sent.
//!
//! Run: cargo run -p neko-tui --example usdc_live -- [chain=address ...]
//! `USDC_AMOUNT` changes the quoted amount from 0.01.

use neko_core::{ChainAddress, ChainId, Stable, TransferRequest};

/// Addresses that held USDC when this was written, found from recent transfer
/// logs. Any argument `chain=address` replaces the one for that chain.
const HOLDERS: &[(ChainId, &str)] = &[
    (ChainId::Bsc, "0xf035430a506e198db618e5546c30d464acdb6040"),
    (
        ChainId::Ethereum,
        "0x6872b6630a3afcd3117191a8403c2002e13df7de",
    ),
    (
        ChainId::Polygon,
        "0xe7804c37c13166ff0b37f5ae0bb07a3aebb6e245",
    ),
    (
        ChainId::Arbitrum,
        "0x0d9d44b6d55f0413c9fd168e78e9e2aab794bfbd",
    ),
    (
        ChainId::Optimism,
        "0x3c4cd4cfaa1e505213b1f07da182832cd80730eb",
    ),
    (
        ChainId::Avalanche,
        "0x864d4e5ee7318e97483db7eb0912e09f161516ea",
    ),
    (
        ChainId::HyperEvm,
        "0x613bc619741354a6171692b04228c9640373e54b",
    ),
    (
        ChainId::Scroll,
        "0x7160570bb153edd0ea1775ec2b2ac9b65f1ab61b",
    ),
    (
        ChainId::Solana,
        "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM",
    ),
    (
        ChainId::Aptos,
        "0x836e0558f907fe74488d455d729bd8b6752fb7d218665852dba7d2aa9f9cfd0a",
    ),
];

#[tokio::main]
async fn main() {
    let mut holders: Vec<(ChainId, String)> = HOLDERS
        .iter()
        .map(|(c, a)| (*c, (*a).to_string()))
        .collect();
    for arg in std::env::args().skip(1) {
        let Some((slug, addr)) = arg.split_once('=') else {
            continue;
        };
        let Some(chain) = ChainId::from_slug(slug) else {
            eprintln!("unknown chain {slug}");
            continue;
        };
        holders.retain(|(c, _)| *c != chain);
        holders.push((chain, addr.to_string()));
    }

    let mut failures = 0;
    for (chain, address) in holders {
        println!("{}", chain.label());
        let Some(usdc) = chain
            .stables()
            .into_iter()
            .find(|a| a.stable() == Some(Stable::Usdc))
        else {
            println!("  no USDC on this chain\n");
            continue;
        };
        let from = ChainAddress::parse(chain, &address).unwrap();
        let client = neko_tui::chain::Client::for_chain(chain, None, None);

        match neko_tui::chain::wallet_assets(&client, from).await {
            Ok(rows) => {
                let want: Vec<&str> = chain.assets().iter().map(|a| a.symbol()).collect();
                let got: Vec<&str> = rows.iter().map(|(s, _, _)| s.as_str()).collect();
                if want != got {
                    failures += 1;
                    println!("  ! rows {got:?} are not the assets {want:?}");
                }
                for (sym, dec, amt) in rows {
                    println!(
                        "  {sym:<5} {:>24}   ({dec} decimals)",
                        neko_core::Amount::new(amt, dec)
                            .to_display_string_trim(neko_tui::chain::BALANCE_FRAC)
                    );
                }
            }
            Err(e) => {
                failures += 1;
                println!("  ! balances failed: {e}");
            }
        }

        // History is informational here: an index may have no USDC row for
        // this address in its latest page, but it must not fail.
        match neko_tui::chain::history(&client, from, 50).await {
            Ok(rows) => {
                let count = |s: &str| rows.iter().filter(|r| r.symbol == s).count();
                println!(
                    "  history: {} rows - USDC {}, USDT {}, {} {}",
                    rows.len(),
                    count("USDC"),
                    count("USDT"),
                    chain.native_symbol(),
                    count(chain.native_symbol())
                );
            }
            Err(e) => println!("  history unavailable: {e}"),
        }

        let amount = std::env::var("USDC_AMOUNT").unwrap_or_else(|_| "0.01".into());
        let req = TransferRequest::parse(1, from, &address, &amount, usdc).unwrap();
        match neko_tui::chain::quote(&client, &req).await {
            Ok(_) => println!(
                "  quote ok: the contract answered USDC, {} decimals\n",
                usdc.decimals()
            ),
            Err(e) => {
                failures += 1;
                println!("  ! quote failed: {e}\n");
            }
        }
    }
    println!("{failures} failure(s)");
    std::process::exit(i32::from(failures > 0));
}
