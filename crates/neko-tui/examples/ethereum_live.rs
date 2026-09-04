//! The Ethereum path, end to end, against mainnet.
#[tokio::main]
async fn main() {
    let seed = neko_hd::derive::seed_from_mnemonic(
        "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
        "",
    )
    .unwrap();
    let addr = neko_hd::derive::evm_address_at(&seed, 0, 0).unwrap();
    println!("derived    {addr}   (m/44'/60'/0'/0/0)");
    println!("           the same address on BNB Chain - one coin type for every EVM chain");

    for chain in [neko_core::ChainId::Ethereum, neko_core::ChainId::Bsc] {
        let client = neko_tui::chain::Client::for_chain(chain, None, None);
        let mine = neko_core::ChainAddress::parse(chain, &addr.to_string()).unwrap();
        println!("\n--- {} ---", chain.label());
        match neko_tui::chain::wallet_assets(&client, mine).await {
            Ok(rows) => {
                for (sym, dec, amt) in rows {
                    println!(
                        "  {sym:<5} {:>22}   ({dec} decimals)",
                        neko_core::Amount::new(amt, dec)
                            .to_display_string_trim(neko_tui::chain::BALANCE_FRAC)
                    );
                }
            }
            Err(e) => println!("  balances failed: {e}"),
        }
        match neko_tui::chain::native_price(&client).await {
            Ok(p) => println!(
                "  1 {} = {} USDT",
                chain.native_symbol(),
                neko_core::Amount::new(p, neko_core::PRICE_SCALE).to_display_string_trim(2)
            ),
            Err(e) => println!("  price failed: {e}"),
        }
    }

    // What a transfer would cost right now, through the real quote path.
    let rpc = neko_evm::client::Rpc::new(neko_evm::ETHEREUM, None);
    // A plain account, not a contract: estimating a bare value transfer into
    // a contract runs its fallback, which most of them revert - correctly, and
    // for a reason that has nothing to do with the fee.
    let to = neko_hd::EvmAddress::parse("0x742d35Cc6634C0532925a3b844Bc454e4438f44e").unwrap();
    match rpc.tx_params(addr, to, 0, &[]).await {
        Ok(p) => {
            let expected = p.gas_limit as u128 * p.fees.expected_per_gas();
            let ceiling = p.gas_limit as u128 * p.fees.max_per_gas();
            println!("\nfee quote (plain transfer):");
            println!("  gas limit  {}", p.gas_limit);
            println!(
                "  type       {}",
                if p.fees.is_eip1559() {
                    "2 (EIP-1559)"
                } else {
                    "0 (legacy)"
                }
            );
            println!(
                "  expected   {} ETH",
                neko_core::Amount::new(expected as i128, 18).to_display_string_trim(10)
            );
            println!(
                "  ceiling    {} ETH",
                neko_core::Amount::new(ceiling as i128, 18).to_display_string_trim(10)
            );
        }
        Err(e) => println!("\nfee quote failed: {e}"),
    }
}
