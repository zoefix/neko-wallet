//! Bitcoin constants.

/// Satoshis: 1 BTC = 1e8. Eight, where TRON and Solana's USDT are six and BNB
/// Chain's are eighteen - the number travels with the asset for this reason.
pub const BTC_DECIMALS: u8 = 8;

/// Esplora, the REST interface Blockstream and mempool.space both serve.
///
/// A plain Bitcoin node cannot answer "what does this address hold": that needs
/// an index over the whole chain, which a node does not keep unless told to.
/// Esplora is the standard way to ask, and this is configurable for the same
/// reason the Solana cluster is - whoever answers sees which addresses you ask
/// about.
pub const DEFAULT_API: &str = "https://blockstream.info/api";

/// How many blocks ahead the default fee rate aims for.
///
/// Three is roughly half an hour. Faster costs more for no benefit on a
/// transfer nobody is waiting on; slower risks a fee that stops being enough
/// if the mempool fills.
pub const TARGET_BLOCKS: u32 = 3;

// The relay floor lives on `coins::FeeRate::MIN`, next to the arithmetic that
// has to respect it.

/// What a block explorer shows.
pub const EXPLORER_TX: &str = "https://mempool.space/tx/";
