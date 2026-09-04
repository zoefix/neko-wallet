//! What BTCB is quoted at right now, and how far it is from BNB's own quote.
#[tokio::main]
async fn main() {
    let rpc = neko_evm::client::Rpc::new(None);
    for (name, r) in [
        ("BNB ", rpc.bnb_price_in_usdt().await),
        ("BTCB", rpc.btcb_price_in_usdt().await),
    ] {
        match r {
            Ok(v) => println!(
                "1 {name} = {} USDT",
                neko_core::Amount::new(v as i128, neko_evm::USDT_DECIMALS)
                    .to_display_string_trim(2)
            ),
            Err(e) => println!("{name}: {e}"),
        }
    }
}
