//! The two kinds of number that both get called a "chain id".
//!
//! One of them numbers the chains this wallet supports, in the order they were
//! added, and is what the database stores. The other is the EVM network id
//! that goes into a signature and decides which network that signature is
//! valid on. They are unrelated, and on every chain here they look unrelated -
//! Base is row 8 and chain 8453 - except one.
//!
//! Optimism is row 10 and EVM chain 10. That coincidence is harmless in
//! itself; what it could do is teach the next reader a rule that is not one,
//! and a chain registered under the wrong number of the two either signs for
//! the wrong network or reads another chain's balances.

/// Optimism's two tens are a coincidence, and no other chain has it.
#[test]
fn optimisms_row_id_matching_its_chain_id_is_a_coincidence() {
    let row = neko_store::repo::addresses::OPTIMISM_CHAIN_ID;
    assert_eq!(row, 10, "the row `migrations/0010_optimism.sql` inserts");
    assert_eq!(
        neko_evm::OPTIMISM.chain_id,
        10,
        "read from the chain: eth_chainId answers 0xa"
    );

    // Optimism is the only chain whose EVM chain id equals *its own* row id.
    let both: Vec<&str> = neko_core::CHAINS
        .iter()
        .filter(|c| c.evm().is_some_and(|e| e.chain_id == db_row(**c) as u64))
        .map(|c| c.slug())
        .collect();
    assert_eq!(both, ["optimism"], "only one chain has the two numbers equal");

    // The namespaces overlap in a second way, which is the real reason this
    // file exists: Ethereum's chain id is 1, and 1 is TRON's row id. So a
    // number in this range is not even enough to tell you *which* chain is
    // meant, let alone which of the two things is being named.
    assert_eq!(neko_evm::ETHEREUM.chain_id, 1);
    assert_eq!(db_row(neko_core::ChainId::Tron), 1);
    assert_eq!(
        db_row(neko_core::ChainId::Ethereum),
        5,
        "Ethereum's own row is 5, not the 1 in its chain id"
    );
}

/// Every chain's row id is distinct, and every EVM chain id is distinct.
///
/// Said separately because they are separate namespaces: two chains may not
/// share a row (they would read each other's balances) and may not share an
/// EVM chain id (a signature for one would be valid on the other).
#[test]
fn neither_numbering_has_a_collision() {
    let rows: Vec<i64> = neko_core::CHAINS.iter().map(|c| db_row(*c)).collect();
    for (i, a) in rows.iter().enumerate() {
        for b in &rows[i + 1..] {
            assert_ne!(a, b, "two chains share a database row id");
        }
    }
    assert_eq!(
        rows.len(),
        neko_core::CHAINS.len(),
        "every chain has a row id"
    );

    let ids: Vec<u64> = neko_core::CHAINS
        .iter()
        .filter_map(|c| c.evm().map(|e| e.chain_id))
        .collect();
    for (i, a) in ids.iter().enumerate() {
        for b in &ids[i + 1..] {
            assert_ne!(a, b, "two chains share an EVM chain id");
        }
    }
    assert_eq!(ids.len(), 12, "twelve EVM chains");
}

/// The row id each chain is stored under. Mirrors `wallets::db_chain_id`,
/// which is private - and being a second copy is the point: if the two ever
/// disagree, the chain this names is not the chain whose balances it reads.
fn db_row(c: neko_core::ChainId) -> i64 {
    use neko_store::repo::addresses as a;
    match c {
        neko_core::ChainId::Tron => a::TRON_CHAIN_ID,
        neko_core::ChainId::Bsc => a::BSC_CHAIN_ID,
        neko_core::ChainId::Solana => a::SOLANA_CHAIN_ID,
        neko_core::ChainId::Bitcoin => a::BITCOIN_CHAIN_ID,
        neko_core::ChainId::Ethereum => a::ETHEREUM_CHAIN_ID,
        neko_core::ChainId::Ton => a::TON_CHAIN_ID,
        neko_core::ChainId::Polygon => a::POLYGON_CHAIN_ID,
        neko_core::ChainId::Base => a::BASE_CHAIN_ID,
        neko_core::ChainId::Arbitrum => a::ARBITRUM_CHAIN_ID,
        neko_core::ChainId::Optimism => a::OPTIMISM_CHAIN_ID,
        neko_core::ChainId::Avalanche => a::AVALANCHE_CHAIN_ID,
        neko_core::ChainId::HyperEvm => a::HYPEREVM_CHAIN_ID,
        neko_core::ChainId::Mantle => a::MANTLE_CHAIN_ID,
        neko_core::ChainId::Linea => a::LINEA_CHAIN_ID,
        neko_core::ChainId::ZkSyncEra => a::ZKSYNC_ERA_CHAIN_ID,
        neko_core::ChainId::Scroll => a::SCROLL_CHAIN_ID,
        neko_core::ChainId::Aptos => a::APTOS_CHAIN_ID,
        neko_core::ChainId::Sui => a::SUI_CHAIN_ID,
    }
}
