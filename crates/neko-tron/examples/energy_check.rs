//! What the chain will actually charge for a USDT transfer.
//!
//! The two figures the node returns are a total and a part of it, not two
//! addends. This prints both, and the fee that follows, so it can be compared
//! against a receipt.

use neko_tron::TronGrid;

#[tokio::main]
async fn main() {
    let c = TronGrid::new(None, std::env::var("TRONGRID_API_KEY").ok());
    let usdt = neko_tron::usdt_address();
    let from = neko_hd::Address::parse(
        &std::env::args()
            .nth(1)
            .unwrap_or_else(|| "TPZrDZTUWQqqUTVRxAmSdQyGXSSgAUyyk4".into()),
    )
    .unwrap();
    // An address that already holds USDT, so this is the ordinary case.
    let to = neko_hd::Address::parse("TNXoiAJ3dct8Fjg4M9fkLFh9S2v9TXc32G").unwrap();

    let calldata = neko_tron::tx::encode_trc20_transfer(to, 10_000_000).unwrap();
    let e = c
        .estimate_trc20_energy(usdt, from, &calldata)
        .await
        .unwrap();
    println!("from       {from}");
    println!("charged    {} energy   <- what the chain takes", e.total());
    println!("  of which {} is the dynamic-energy surcharge", e.penalty);
    println!("  leaving  {} for the call itself", e.base());
    println!("the sum of the two would be {}", e.total() + e.penalty);

    let p = c.prices().await.unwrap();
    let burn = e.total() * p.sun_per_energy;
    println!(
        "\nat {} sun/energy that is {} TRX",
        p.sun_per_energy,
        neko_core_amount(burn)
    );
    println!(
        "the old arithmetic would have said {} TRX",
        neko_core_amount((e.total() + e.penalty) * p.sun_per_energy)
    );
}

fn neko_core_amount(sun: i64) -> String {
    format!("{}.{:06}", sun / 1_000_000, (sun % 1_000_000).abs())
}
