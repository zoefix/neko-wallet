// Associated token accounts that exist on mainnet.
//
// `(owner, mint, associated token account)`. Each was derived independently
// and then confirmed against the cluster with `getAccountInfo`: every one of
// these accounts exists, is owned by the address in the first column and holds
// the mint in the second. Derivation that is subtly wrong produces a valid
// address nobody will ever look at, so the check has to be against reality
// rather than against the algorithm restated.
//
// The bump seeds differ (255, 254 and 253 are all represented here), which is
// what exercises the downward search rather than assuming the first try wins.
#[allow(dead_code)]
const KNOWN_ATAS: &[(&str, &str, &str)] = &[
    // Binance, USDT. bump 254.
    (
        "5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9",
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
        "CyBjGpte4Npi5zNkdtWumPxVW4kpMR8BuFSbA587xZES",
    ),
    // Binance, USDC. bump 254.
    (
        "5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9",
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        "FzbcyEZ9m8xjtergWgWDq7mfPoHEbboBF791B6cTpzbq",
    ),
    // bump 253 - the case where two candidates in a row are on the curve.
    (
        "2ojv9BAiHUrvsm9gxDe7fJSzbNZSJcxZvf8dqmWGHG8S",
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
        "GCByP7MsZ6CCerH7qsxCJh4uXe1RdmqLtoj7Nv1qvtjn",
    ),
    (
        "2ojv9BAiHUrvsm9gxDe7fJSzbNZSJcxZvf8dqmWGHG8S",
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        "9KaA7vEBUdRCcBWxfuMjxYwKfvu8Us3Cg5gkhVFt2LNk",
    ),
    // bump 255, and a 43-character address: base58 length varies with the
    // leading byte, so this is the short-encoding case.
    (
        "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM",
        "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB",
        "TB5FCqbNsnuLQgEjUuPaT9qtVPTT4U1A8rvi7qzEj2M",
    ),
];
