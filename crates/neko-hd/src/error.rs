use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum HdError {
    #[error("public key must be 65 bytes (uncompressed), got {0}")]
    BadPublicKeyLen(usize),
    #[error("public key is not in uncompressed SEC1 form")]
    PublicKeyNotUncompressed,
    #[error("a TRON address must be 21 bytes, got {0}")]
    BadAddressLen(usize),
    #[error("a TRON address must start with 0x41, got 0x{0:02x}")]
    BadAddressPrefix(u8),

    // --- EVM chains. Separate variants because the shapes differ and a
    // message about 21 bytes and a 0x41 prefix would be actively misleading
    // to somebody who pasted an EVM address.
    #[error("an EVM address must be 20 bytes of hex, got {0} hex characters")]
    BadEvmAddressLen(usize),
    #[error("an EVM address must start with 0x")]
    MissingHexPrefix,
    #[error("not valid hexadecimal")]
    NotHex,
    #[error("the EIP-55 capitalisation does not match this address - check for a typo")]
    BadEip55Checksum,
    #[error("address checksum does not match")]
    BadChecksum,

    // --- Solana. Its address is a bare 32-byte key in plain base58: no
    // prefix, no checksum, nothing to say beyond the length.
    #[error("a Solana address must be 32 bytes of base58")]
    BadSolanaAddress,

    // --- Bitcoin. Five script types, three text encodings, and a testnet that
    // uses the same alphabet - so there is nothing useful to say beyond that
    // this is not a mainnet address this program can pay.
    #[error("not a Bitcoin mainnet address")]
    BadBtcAddress,
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
