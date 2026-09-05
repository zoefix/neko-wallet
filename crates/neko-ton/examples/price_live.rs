//! Read the GRAM price off the pool, and check it against the pool's own
//! swap estimator - a different method, on the same contract, reaching the
//! same reserves by a different route. A reserve read backwards, or scaled
//! wrong, does not survive that.

use neko_ton::cell::CellBuilder;
use neko_ton::{client, client::Toncenter, price};

#[tokio::main]
async fn main() {
    let api = Toncenter::new(std::env::var("TON_API").ok().as_deref(), None);
    let pool = neko_ton::TonAddress::parse(price::GRAM_USDT_POOL).unwrap();

    let reserves = api
        .get_method_ints(&pool, "get_reserves", &[])
        .await
        .unwrap();
    let spot = price::gram_in_usdt(&api, 6).await.unwrap();
    println!("reserves  {reserves:?}");
    println!(
        "spot      {spot} (6dp) = {} USDT per GRAM",
        spot as f64 / 1e6
    );

    // Native GRAM as DeDust describes an asset: four bits of tag and nothing
    // else.
    let mut b = CellBuilder::new();
    b.store_uint(0, 4).unwrap();
    let native = client::slice_arg(&b.build_arc().unwrap()).unwrap();

    // One whole GRAM in. The stack comes back mixed - the asset paid out, then
    // the amount, then the fee.
    let out = api
        .get_method(
            &pool,
            "estimate_swap_out",
            &[native, serde_json::json!(["num", "1000000000"])],
        )
        .await
        .unwrap();
    let amount = client::stack_int(&out[1]).unwrap();
    let fee = client::stack_int(&out[2]).unwrap();
    println!(
        "swap 1 GRAM -> {amount} USDT-units = {} USDT (fee {fee})",
        amount as f64 / 1e6
    );

    let gap = (spot - amount as i128) * 10_000 / spot;
    println!("gap to spot: {gap} bp (DeDust charges 25)");
    println!(
        "\npin these into the test:\n  spot({}, 9, {}, 6, 6) == {spot}\n  swap = {amount}",
        reserves[0], reserves[1]
    );
}
