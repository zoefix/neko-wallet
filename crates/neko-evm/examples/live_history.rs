#[tokio::main]
async fn main() {
    let key = std::env::var("NR_KEY").expect("set NR_KEY");
    let c = neko_evm::history::Bsctrace::new(neko_evm::BSC, &key);
    let who = neko_hd::EvmAddress::parse("0x9858EfFD232B4033E47d90003D41EC34EcaEda94").unwrap();
    match c.transfers(who, neko_evm::BSC.usdt_address(), 6).await {
        Ok(rows) => {
            println!("{} 条:", rows.len());
            for r in rows {
                let amt = neko_core::Amount::new(r.amount, r.decimals);
                let dir = if r.to.eq_ignore_ascii_case(&who.to_string()) {
                    "in "
                } else {
                    "out"
                };
                println!(
                    "  {dir} {:>22} {:<5} {} {}",
                    amt.to_display_string(),
                    r.symbol,
                    if r.success { "ok  " } else { "FAIL" },
                    &r.hash[..18]
                );
            }
        }
        Err(e) => println!("失败: {e}"),
    }
}
