//! Building and signing a Solana transaction.
//!
//! The wire format has no field names and no lengths beyond compact-u16
//! prefixes, so a byte in the wrong place does not fail to parse - it parses as
//! something else. Two places where that matters more than usual:
//!
//! * **Account order is part of the meaning.** Keys are sorted into four
//!   groups - writable signers, readonly signers, writable non-signers,
//!   readonly non-signers - and the header only says how many are in each. Sort
//!   them differently and every instruction's account indices point somewhere
//!   else, at accounts that exist.
//! * **The fee payer is whatever ends up first.** Not a flag; a position.
//!
//! The signature covers the serialized message, so both of the above are signed
//! over. That is the reason to get them right here rather than to hope the
//! cluster complains.

use neko_hd::SolanaAddress;
use zeroize::Zeroizing;

use crate::chain_consts;
use crate::error::SolanaError;
use crate::pda::associated_token_address;
use crate::shortvec;

/// A transaction must fit one network packet: 1280 bytes of MTU less 48 for
/// the IPv6 and fragment headers.
pub const MAX_TX_BYTES: usize = 1232;

/// Account indices are `u8`, so this is a hard structural limit.
pub const MAX_ACCOUNTS: usize = 256;

pub const SIGNATURE_BYTES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountMeta {
    pub pubkey: SolanaAddress,
    pub is_signer: bool,
    pub is_writable: bool,
}

impl AccountMeta {
    pub fn signer_writable(pubkey: SolanaAddress) -> Self {
        Self {
            pubkey,
            is_signer: true,
            is_writable: true,
        }
    }
    pub fn signer_readonly(pubkey: SolanaAddress) -> Self {
        Self {
            pubkey,
            is_signer: true,
            is_writable: false,
        }
    }
    pub fn writable(pubkey: SolanaAddress) -> Self {
        Self {
            pubkey,
            is_signer: false,
            is_writable: true,
        }
    }
    pub fn readonly(pubkey: SolanaAddress) -> Self {
        Self {
            pubkey,
            is_signer: false,
            is_writable: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Instruction {
    pub program_id: SolanaAddress,
    pub accounts: Vec<AccountMeta>,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledInstruction {
    pub program_id_index: u8,
    pub accounts: Vec<u8>,
    pub data: Vec<u8>,
}

/// Everything the cluster has to tell us before a transaction can be signed.
///
/// `recent_blockhash` is the reason this type exists separately from the fee
/// quote. It is good for about a minute, so it is fetched immediately before
/// signing - long after the screen that showed the fee. Carrying one from the
/// quote, the way TRON's block reference is carried, would produce transactions
/// that are simply dropped by the cluster while looking perfectly valid here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxParams {
    pub recent_blockhash: [u8; 32],
    pub compute_unit_limit: u32,
    /// Micro-lamports per compute unit. Zero is legitimate on a quiet cluster
    /// and fatal on a busy one, which is why it comes from the chain.
    pub compute_unit_price: u64,
    /// Whether this transfer has to open the recipient's token account, and so
    /// pay its rent. Decided by asking the cluster, never by assuming.
    pub create_recipient_account: bool,
}

/// A legacy (unversioned) message. Address lookup tables buy nothing for a
/// wallet that sends to one recipient, and every cluster still accepts this.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub num_required_signatures: u8,
    pub num_readonly_signed: u8,
    pub num_readonly_unsigned: u8,
    pub account_keys: Vec<SolanaAddress>,
    pub recent_blockhash: [u8; 32],
    pub instructions: Vec<CompiledInstruction>,
}

impl Message {
    /// Gather every account an instruction touches, order them the way the
    /// runtime expects, and rewrite the instructions to point at positions.
    ///
    /// `fee_payer` is seen first and forced to be a writable signer, which is
    /// what puts it at index 0 - the only way to say who pays.
    pub fn compile(
        fee_payer: &SolanaAddress,
        instructions: &[Instruction],
        recent_blockhash: [u8; 32],
    ) -> Result<Self, SolanaError> {
        // (key, is_signer, is_writable), in first-seen order. Duplicates merge
        // by taking the union of the flags: an account that is writable in one
        // instruction is writable for the transaction.
        let mut keys: Vec<(SolanaAddress, bool, bool)> = vec![(*fee_payer, true, true)];
        let see = |k: SolanaAddress,
                   signer: bool,
                   writable: bool,
                   keys: &mut Vec<(SolanaAddress, bool, bool)>| {
            if let Some(e) = keys.iter_mut().find(|(p, _, _)| *p == k) {
                e.1 |= signer;
                e.2 |= writable;
            } else {
                keys.push((k, signer, writable));
            }
        };
        for ix in instructions {
            for m in &ix.accounts {
                see(m.pubkey, m.is_signer, m.is_writable, &mut keys);
            }
        }
        // Programs come last so they cannot displace an account somebody is
        // paying from. A program that is also an account keeps both roles.
        for ix in instructions {
            see(ix.program_id, false, false, &mut keys);
        }

        if keys.len() > MAX_ACCOUNTS {
            return Err(SolanaError::TooManyAccounts(keys.len()));
        }

        // Stable, so first-seen order survives inside each group.
        keys.sort_by_key(|(_, signer, writable)| match (signer, writable) {
            (true, true) => 0,
            (true, false) => 1,
            (false, true) => 2,
            (false, false) => 3,
        });
        debug_assert_eq!(
            keys.first().map(|(k, _, _)| k),
            Some(fee_payer),
            "the fee payer must be account 0"
        );

        let num_required_signatures = keys.iter().filter(|(_, s, _)| *s).count() as u8;
        let num_readonly_signed = keys.iter().filter(|(_, s, w)| *s && !*w).count() as u8;
        let num_readonly_unsigned = keys.iter().filter(|(_, s, w)| !*s && !*w).count() as u8;

        let account_keys: Vec<SolanaAddress> = keys.iter().map(|(k, _, _)| *k).collect();
        let index_of = |k: &SolanaAddress| -> u8 {
            account_keys
                .iter()
                .position(|p| p == k)
                .expect("every key was collected above") as u8
        };

        let compiled = instructions
            .iter()
            .map(|ix| CompiledInstruction {
                program_id_index: index_of(&ix.program_id),
                accounts: ix.accounts.iter().map(|m| index_of(&m.pubkey)).collect(),
                data: ix.data.clone(),
            })
            .collect();

        Ok(Self {
            num_required_signatures,
            num_readonly_signed,
            num_readonly_unsigned,
            account_keys,
            recent_blockhash,
            instructions: compiled,
        })
    }

    /// The bytes a signature covers.
    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.account_keys.len() * 32);
        out.push(self.num_required_signatures);
        out.push(self.num_readonly_signed);
        out.push(self.num_readonly_unsigned);

        shortvec::encode(&mut out, self.account_keys.len());
        for k in &self.account_keys {
            out.extend_from_slice(k.as_bytes());
        }
        out.extend_from_slice(&self.recent_blockhash);

        shortvec::encode(&mut out, self.instructions.len());
        for ix in &self.instructions {
            out.push(ix.program_id_index);
            shortvec::encode(&mut out, ix.accounts.len());
            out.extend_from_slice(&ix.accounts);
            shortvec::encode(&mut out, ix.data.len());
            out.extend_from_slice(&ix.data);
        }
        out
    }
}

/// A signed transaction, ready to broadcast.
pub struct Transaction {
    pub signatures: Vec<[u8; SIGNATURE_BYTES]>,
    pub message: Message,
}

impl Transaction {
    /// Sign with one key. Every transaction this wallet builds has exactly one
    /// signer, so a second key would mean a bug rather than a use case.
    pub fn sign(message: Message, key: &Zeroizing<[u8; 32]>) -> Result<Self, SolanaError> {
        let bytes = message.serialize();
        let sig = neko_hd::solana::sign(key, &bytes);

        // Proof that the key that signed is the account being debited. On the
        // other chains this is a public-key recovery check; here the address is
        // the public key, so it is a direct comparison.
        let signer = neko_hd::solana::address_from_private_key(key)?;
        if message.account_keys.first() != Some(&signer) {
            return Err(SolanaError::BadReply(
                "the signing key is not this transaction's fee payer".into(),
            ));
        }

        Ok(Self {
            signatures: vec![sig],
            message,
        })
    }

    pub fn serialize(&self) -> Result<Vec<u8>, SolanaError> {
        let msg = self.message.serialize();
        let mut out = Vec::with_capacity(1 + self.signatures.len() * SIGNATURE_BYTES + msg.len());
        shortvec::encode(&mut out, self.signatures.len());
        for s in &self.signatures {
            out.extend_from_slice(s);
        }
        out.extend_from_slice(&msg);
        if out.len() > MAX_TX_BYTES {
            return Err(SolanaError::MessageTooLong(out.len(), MAX_TX_BYTES));
        }
        Ok(out)
    }

    /// What block explorers call the transaction id: its first signature, in
    /// base58. Known before broadcasting, unlike TRON's hash of the raw body.
    pub fn id(&self) -> String {
        bs58::encode(self.signatures[0]).into_string()
    }
}

// ── Instructions ───────────────────────────────────────────────────────────

/// Move lamports between system accounts.
pub fn transfer_sol(from: SolanaAddress, to: SolanaAddress, lamports: u64) -> Instruction {
    // System program instruction 2, as a little-endian u32 discriminant.
    let mut data = Vec::with_capacity(12);
    data.extend_from_slice(&2u32.to_le_bytes());
    data.extend_from_slice(&lamports.to_le_bytes());
    Instruction {
        program_id: chain_consts::system_program(),
        accounts: vec![
            AccountMeta::signer_writable(from),
            AccountMeta::writable(to),
        ],
        data,
    }
}

/// Move SPL tokens, with the mint and its precision checked by the program.
///
/// `TransferChecked` rather than the older `Transfer`: it makes the runtime
/// verify that the mint is what we think and that `decimals` matches, on chain,
/// for every transfer. That is the same check this wallet does before signing,
/// enforced somewhere a bug here cannot skip.
pub fn transfer_token_checked(
    source: SolanaAddress,
    mint: SolanaAddress,
    destination: SolanaAddress,
    owner: SolanaAddress,
    amount: u64,
    decimals: u8,
) -> Instruction {
    let mut data = Vec::with_capacity(10);
    data.push(12); // TransferChecked
    data.extend_from_slice(&amount.to_le_bytes());
    data.push(decimals);
    Instruction {
        program_id: chain_consts::token_program(),
        accounts: vec![
            AccountMeta::writable(source),
            AccountMeta::readonly(mint),
            AccountMeta::writable(destination),
            AccountMeta::signer_readonly(owner),
        ],
        data,
    }
}

/// Open the recipient's account for a token they have never held.
///
/// Idempotent, so a recipient who opened one between the quote and the
/// signature does not turn the transfer into a failure that still charges a
/// fee.
pub fn create_associated_token_account(
    funder: SolanaAddress,
    owner: SolanaAddress,
    mint: SolanaAddress,
) -> Result<Instruction, SolanaError> {
    let ata = associated_token_address(&owner, &mint)?;
    Ok(Instruction {
        program_id: chain_consts::associated_token_program(),
        accounts: vec![
            AccountMeta::signer_writable(funder),
            AccountMeta::writable(ata),
            AccountMeta::readonly(owner),
            AccountMeta::readonly(mint),
            AccountMeta::readonly(chain_consts::system_program()),
            AccountMeta::readonly(chain_consts::token_program()),
        ],
        data: vec![1], // CreateIdempotent
    })
}

pub fn set_compute_unit_limit(units: u32) -> Instruction {
    let mut data = Vec::with_capacity(5);
    data.push(2);
    data.extend_from_slice(&units.to_le_bytes());
    Instruction {
        program_id: chain_consts::compute_budget_program(),
        accounts: vec![],
        data,
    }
}

pub fn set_compute_unit_price(micro_lamports: u64) -> Instruction {
    let mut data = Vec::with_capacity(9);
    data.push(3);
    data.extend_from_slice(&micro_lamports.to_le_bytes());
    Instruction {
        program_id: chain_consts::compute_budget_program(),
        accounts: vec![],
        data,
    }
}

/// What a transaction will cost, in lamports.
///
/// The base fee is per signature and fixed. The priority fee is the compute
/// budget times the price per million compute units, and the runtime rounds it
/// *up* - so this does too, rather than quoting a figure a lamport below what
/// gets charged.
pub fn fee_lamports(signatures: u64, compute_units: u32, micro_lamports_per_cu: u64) -> u64 {
    let base = signatures.saturating_mul(chain_consts::LAMPORTS_PER_SIGNATURE);
    let priority = (compute_units as u64)
        .saturating_mul(micro_lamports_per_cu)
        .div_ceil(1_000_000);
    base.saturating_add(priority)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FROM: &str = "5tzFkiKscXHK5ZXCGbXZxdw7gTjjD1mBwuoFbhUvuAi9";
    const TO: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";

    fn addr(s: &str) -> SolanaAddress {
        SolanaAddress::parse(s).unwrap()
    }
    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    /// The exact bytes, pinned.
    ///
    /// Both messages below were built by this code, sent to mainnet's
    /// `simulateTransaction`, and executed there. The plain transfer and the
    /// token transfer both returned `err=null`; deliberately corrupted versions
    /// were rejected by the on-chain programs with the specific errors that
    /// prove each field is read where this code puts it - an oversized amount
    /// gave "insufficient funds", a wrong `decimals` gave "decimals different
    /// from the Mint decimals", and swapping the token accounts gave "owner
    /// does not match".
    ///
    /// Pinning the result keeps that verification without a network call. A
    /// change to the encoding that the cluster would reject fails here first.
    #[test]
    fn the_wire_bytes_are_what_mainnet_accepted() {
        let from = addr(FROM);
        let to = addr(TO);
        let blockhash = [0x11u8; 32];

        let sol = Message::compile(&from, &[transfer_sol(from, to, 1_000_000)], blockhash).unwrap();
        assert_eq!(
            hex(&sol.serialize()),
            concat!(
                // header: 1 signature, 0 readonly signed, 1 readonly unsigned
                "010001",
                // 3 account keys
                "03",
                "48c01b5059005455d9dcb0c6bcecdcb4fb5b2eabc1a9a82b57392baaa40f04e6",
                "7e8c088760bfde1dddcf32c17f209b8242ee52aaf131facd88d0ea2c6d0b06f2",
                "0000000000000000000000000000000000000000000000000000000000000000",
                // blockhash
                "1111111111111111111111111111111111111111111111111111111111111111",
                // 1 instruction: program 2, accounts [0,1], 12 bytes of data
                "01",
                "02",
                "02",
                "0001",
                // discriminant 2 (u32 LE), then 1_000_000 lamports (u64 LE)
                "0c",
                "02000000",
                "40420f0000000000",
            )
        );

        let mint = crate::chain_consts::usdt_mint();
        let src = associated_token_address(&from, &mint).unwrap();
        let dst = associated_token_address(&to, &mint).unwrap();
        let token = Message::compile(
            &from,
            &[transfer_token_checked(src, mint, dst, from, 1_000_000, 6)],
            blockhash,
        )
        .unwrap();
        assert_eq!(
            hex(&token.serialize()),
            concat!(
                "010002",
                "05",
                "48c01b5059005455d9dcb0c6bcecdcb4fb5b2eabc1a9a82b57392baaa40f04e6",
                "b1d5366f7f37a176aba38d17f2a891f19f6ac38c7e14e0c1bb7ad884556de763",
                "06b47da6c2d705c86b33226c4b67a8a273f153bc9c9043a5a8e14597c063d9ce",
                "ce010e60afedb22717bd63192f54145a3f965a33bb82d2c7029eb2ce1e208264",
                "06ddf6e1d765a193d9cbe146ceeb79ac1cb485ed5f5b37913a8cf5857eff00a9",
                "1111111111111111111111111111111111111111111111111111111111111111",
                // program 4, accounts [1,3,2,0], 10 bytes of data
                "01",
                "04",
                "04",
                "01030200",
                // TransferChecked, 1_000_000, 6 decimals
                "0a",
                "0c",
                "40420f0000000000",
                "06",
            )
        );
    }

    /// The fee payer is a position, not a flag, and everything else is ordered
    /// around it. Getting this wrong points every instruction at the wrong
    /// account - at accounts that exist.
    #[test]
    fn accounts_are_grouped_and_the_payer_comes_first() {
        let payer = addr(FROM);
        let mint = crate::chain_consts::usdt_mint();
        let other = addr(TO);
        let src = associated_token_address(&payer, &mint).unwrap();
        let dst = associated_token_address(&other, &mint).unwrap();

        let m = Message::compile(
            &payer,
            &[
                set_compute_unit_limit(40_000),
                create_associated_token_account(payer, other, mint).unwrap(),
                transfer_token_checked(src, mint, dst, payer, 1, 6),
            ],
            [0u8; 32],
        )
        .unwrap();

        assert_eq!(m.account_keys[0], payer, "the fee payer must be account 0");
        assert_eq!(m.num_required_signatures, 1);
        assert_eq!(m.num_readonly_signed, 0);

        // Writable non-signers sit between the signers and the readonly keys.
        let n = m.account_keys.len();
        let ro_start = n - m.num_readonly_unsigned as usize;
        let writable = &m.account_keys[1..ro_start];
        assert!(writable.contains(&src) && writable.contains(&dst));
        // Programs and the mint are readonly.
        let readonly = &m.account_keys[ro_start..];
        assert!(readonly.contains(&mint));
        assert!(readonly.contains(&crate::chain_consts::token_program()));
        assert!(readonly.contains(&crate::chain_consts::system_program()));
    }

    /// An account named twice is one account with the union of its roles. Two
    /// entries would shift every later index by one.
    #[test]
    fn a_repeated_account_appears_once() {
        let a = addr(FROM);
        let b = addr(TO);
        let m = Message::compile(
            &a,
            &[transfer_sol(a, b, 1), transfer_sol(a, b, 2)],
            [0u8; 32],
        )
        .unwrap();
        // a, b, system program. Nothing repeated.
        assert_eq!(m.account_keys.len(), 3);
        assert_eq!(m.instructions.len(), 2);
        assert_eq!(m.instructions[0].accounts, m.instructions[1].accounts);
    }

    /// Signing has to prove the key belongs to the account being debited.
    /// A signature from anyone else is a fee paid for a rejected transaction.
    #[test]
    fn signing_with_the_wrong_key_is_refused() {
        let seed = [3u8; 64];
        let mine = neko_hd::solana::private_key_at(&seed, 0).unwrap();
        let other = neko_hd::solana::private_key_at(&seed, 1).unwrap();
        let my_addr = neko_hd::solana::address_from_private_key(&mine).unwrap();

        let msg =
            Message::compile(&my_addr, &[transfer_sol(my_addr, addr(TO), 1)], [0u8; 32]).unwrap();
        assert!(Transaction::sign(msg.clone(), &other).is_err());

        let signed = Transaction::sign(msg, &mine).expect("the right key must work");
        // And that signature has to verify against the address on the account.
        use ed25519_dalek::Verifier;
        let vk = ed25519_dalek::VerifyingKey::from_bytes(my_addr.as_bytes()).unwrap();
        vk.verify(
            &signed.message.serialize(),
            &ed25519_dalek::Signature::from_bytes(&signed.signatures[0]),
        )
        .expect("the signature does not cover the message");
    }

    /// The base fee is per signature; the priority fee rounds up, because the
    /// runtime rounds up and a quote a lamport low is a quote that is wrong.
    #[test]
    fn fees_round_the_way_the_runtime_does() {
        // No priority fee at all.
        assert_eq!(fee_lamports(1, 500, 0), 5_000);
        // 500 units at 1000 micro-lamports = 0.5 lamports, charged as 1.
        assert_eq!(fee_lamports(1, 500, 1_000), 5_001);
        // Exactly one lamport, not two.
        assert_eq!(fee_lamports(1, 1_000, 1_000), 5_001);
        assert_eq!(fee_lamports(1, 1_001, 1_000), 5_002);
        // A realistic token transfer under load.
        assert_eq!(fee_lamports(1, 40_000, 50_000), 5_000 + 2_000);
    }

    /// A transaction that cannot fit a packet must be refused here rather than
    /// by a node, which would charge for the attempt.
    #[test]
    fn an_oversized_transaction_is_refused() {
        let a = addr(FROM);
        let seed = [4u8; 64];
        // Distinct recipients, so the account list itself is what overflows -
        // 32 bytes each, which is how a real batch would hit the limit.
        let mut ixs = Vec::new();
        for i in 0..40 {
            let to = neko_hd::solana::address_at(&seed, i).unwrap();
            ixs.push(transfer_sol(a, to, 1));
        }
        let m = Message::compile(&a, &ixs, [0u8; 32]).unwrap();
        let t = Transaction {
            signatures: vec![[0u8; 64]],
            message: m,
        };
        match t.serialize() {
            Err(SolanaError::MessageTooLong(got, max)) => {
                assert!(got > max, "{got} should be over {max}")
            }
            other => panic!(
                "expected a length refusal, got {:?}",
                other.map(|v| v.len())
            ),
        }
    }
}
