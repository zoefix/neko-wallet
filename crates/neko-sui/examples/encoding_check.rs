//! Compare this wallet's BCS against a transaction Sui's own node built.
//!
//! The reference was produced by `unsafe_paySui` on mainnet and decoded byte
//! by byte - all 219 of them. Reproducing it exactly is what makes the builder
//! trustworthy: BCS is not self-describing, so a wrong encoding is not
//! malformed, it is a different transaction.

use neko_sui::tx::{self, GasData, ObjectRef};
use neko_sui::SuiAddress;

fn main() {
    let sender =
        SuiAddress::parse("0xffd4f043057226453aeba59732d41c6093516f54823ebc3a16d17f8a77d2f0ad")
            .unwrap();
    let to =
        SuiAddress::parse("0x0000000000000000000000000000000000000000000000000000000000000002")
            .unwrap();
    let gas_id: [u8; 32] =
        hex::decode("aa649b915f683af20595631a46826e99a2bb6e0b093b5dd4a4a6ccee89cdaf23")
            .unwrap()
            .try_into()
            .unwrap();
    let digest: [u8; 32] =
        hex::decode("90668da58c70bbde13fc25de770c787c489498250bcf74115759be7a4ab98473")
            .unwrap()
            .try_into()
            .unwrap();

    let data = tx::pay_sui(
        sender,
        to,
        1_000_000,
        GasData {
            payment: vec![ObjectRef {
                id: gas_id,
                version: 985_513_514,
                digest,
            }],
            owner: sender,
            price: 100,
            budget: 3_000_000,
        },
    );

    let ours = data.to_bytes();
    let theirs = hex::decode(
        "000002000840420f0000000000002000000000000000000000000000000000000000000000000000000000\
         0000000202020001010000010103000000000101 00ffd4f043057226453aeba59732d41c6093516f54823eb\
         c3a16d17f8a77d2f0ad01aa649b915f683af20595631a46826e99a2bb6e0b093b5dd4a4a6ccee89cdaf232ab\
         ebd3a000000002090668da58c70bbde13fc25de770c787c489498250bcf74115759be7a4ab98473ffd4f0430\
         57226453aeba59732d41c6093516f54823ebc3a16d17f8a77d2f0ad6400000000000000c0c62d0000000000\
         00"
            .replace(['\n', ' '], ""),
    )
    .unwrap();

    println!("ours  {} bytes", ours.len());
    println!("node  {} bytes", theirs.len());
    if ours == theirs {
        println!("IDENTICAL");
    } else {
        println!("DIFFER");
        println!("  ours: {}", hex::encode(&ours));
        println!("  node: {}", hex::encode(&theirs));
        let n = ours.iter().zip(&theirs).take_while(|(a, b)| a == b).count();
        println!("  first difference at byte {n}");
    }
}
