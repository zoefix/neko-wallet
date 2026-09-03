use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HdError {
    #[error("public key must be 65 bytes (uncompressed), got {0}")]
    BadPublicKeyLen(usize),
    #[error("public key is not in uncompressed SEC1 form")]
    PublicKeyNotUncompressed,
    #[error("address must be 21 bytes, got {0}")]
    BadAddressLen(usize),
    #[error("address prefix must be 0x41, got 0x{0:02x}")]
    BadAddressPrefix(u8),
    #[error("address checksum does not match")]
    BadChecksum,
    #[error("not valid base58")]
    BadBase58,
    #[error("mnemonic is not valid BIP39")]
    BadMnemonic,
    #[error("entropy must be 16 or 32 bytes, got {0}")]
    BadEntropyLen(usize),
    #[error("derivation index {0} is out of range")]
    IndexOutOfRange(u32),
    #[error("key derivation failed")]
    Derive,
    #[error("private key is not a valid secp256k1 scalar")]
    BadPrivateKey,
}
