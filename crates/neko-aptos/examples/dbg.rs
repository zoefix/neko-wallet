#[tokio::main]
async fn main() {
    let rest = neko_aptos::client::Rest::new(None);
    // The test-mnemonic account, whose public key we know.
    let a = neko_aptos::AptosAddress::parse(
        "0xeb663b681209e7087d681c5d3eed12aaa8e1915e7c87794542c3f96e94b3d3bf",
    )
    .unwrap();
    let pk: [u8; 32] =
        hex::decode("a686f0309ab80312979606cfccc10ea2740147ae6888351488d11c46f08fbf60")
            .unwrap()
            .try_into()
            .unwrap();
    assert_eq!(
        neko_aptos::AptosAddress::from_public_key(&pk),
        a,
        "pubkey matches the account"
    );
    let to = neko_aptos::AptosAddress::parse(
        "0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    )
    .unwrap();

    let seq = rest.sequence_number(a).await.unwrap();
    let price = rest.gas_unit_price().await.unwrap();
    let now = rest.ledger_time_secs().await.unwrap();
    for (label, payload) in [
        ("APT", neko_aptos::tx::transfer_apt(to, 1_000)),
        (
            "USDT",
            neko_aptos::tx::transfer_fungible_asset(neko_aptos::usdt_metadata(), to, 1_000),
        ),
    ] {
        let raw = neko_aptos::tx::RawTransaction {
            sender: a,
            payload,
            params: neko_aptos::tx::TxParams {
                sequence_number: seq,
                max_gas_amount: neko_aptos::MAX_GAS_TRANSFER,
                gas_unit_price: price,
                expiration_timestamp_secs: now + 600,
                chain_id: neko_aptos::CHAIN_ID,
            },
        };
        let bytes = neko_aptos::tx::simulation_bytes(&raw, &pk);
        println!("{label}: simulate -> {:?}", rest.simulate(&bytes).await);
    }
}
