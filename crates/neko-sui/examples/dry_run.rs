//! Build both transfers locally and have mainnet price them.
//!
//! A dry run that returns a cost has been decoded by the chain's own parser,
//! which is the strongest check available without spending anything.

use neko_sui::client::Rpc;
use neko_sui::tx::{self, GasData};
use neko_sui::SuiAddress;

#[tokio::main]
async fn main() {
    let rpc = Rpc::new(None);
    let who = std::env::args().nth(1).unwrap_or_else(|| {
        "0x77776760c06997206b13fa76f127aa016d24a645f04fce516be153ece0bddf23".into()
    });
    let sender = SuiAddress::parse(&who).unwrap();
    let to =
        SuiAddress::parse("0x0000000000000000000000000000000000000000000000000000000000000002")
            .unwrap();

    let price = rpc.reference_gas_price().await.unwrap();
    let sui_coins = rpc.coins(sender, neko_sui::SUI_TYPE).await.unwrap();
    println!("sender {sender}");
    println!("  gas price {price}  SUI coin objects: {}", sui_coins.len());
    if sui_coins.is_empty() {
        println!("  no SUI to pay gas with");
        return;
    }
    let gas = GasData {
        payment: vec![sui_coins[0].object],
        owner: sender,
        price,
        budget: neko_sui::GAS_BUDGET_TRANSFER,
    };

    let data = tx::pay_sui(sender, to, 1_000_000, gas.clone());
    println!("\nSUI transfer: {} bytes", data.to_bytes().len());
    match rpc.dry_run(&data.to_bytes()).await {
        Ok(g) => println!(
            "  ACCEPTED  computation {} + storage {} - rebate {} = {} MIST",
            g.computation, g.storage, g.rebate, g.net
        ),
        Err(e) => println!("  refused: {e}"),
    }

    let usdc = rpc.coins(sender, neko_sui::USDC_TYPE).await.unwrap();
    println!("\nUSDC coin objects: {}", usdc.len());
    if usdc.is_empty() {
        return;
    }
    let objs: Vec<_> = usdc.iter().map(|c| c.object).collect();
    let data = tx::pay_token(sender, to, 1_000, &objs, gas).unwrap();
    println!(
        "USDC transfer: {} bytes, {} inputs, {} commands",
        data.to_bytes().len(),
        data.inputs.len(),
        data.commands.len()
    );
    match rpc.dry_run(&data.to_bytes()).await {
        Ok(g) => println!(
            "  ACCEPTED  computation {} + storage {} - rebate {} = {} MIST",
            g.computation, g.storage, g.rebate, g.net
        ),
        Err(e) => println!("  refused: {e}"),
    }
}
