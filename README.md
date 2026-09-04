<div align="center">

# Neko Wallet

---

A self-custody encrypted crypto wallet for the terminal. Your whole wallet is
one encrypted file — carry it on a USB stick, keep it on a network drive, copy
it anywhere. Unlocked by an email and a password that are stored nowhere.

Multi-chain by design: TRON, BNB Chain and Solana.

[English](README.md) | [简体中文](README.zh-CN.md) | [繁體中文](README.zh-TW.md) | [日本語](README.ja.md)

![Version](https://img.shields.io/badge/VERSION-v0.1.0-8A2BE2?style=for-the-badge&labelColor=444)
![Platform](https://img.shields.io/badge/PLATFORM-MACOS%20%7C%20LINUX%20%7C%20WINDOWS-00B5E2?style=for-the-badge&labelColor=444)
![Chains](https://img.shields.io/badge/CHAINS-TRON%20%7C%20BNB%20%7C%20SOLANA-1BC47D?style=for-the-badge&labelColor=444)
![Rust](https://img.shields.io/badge/RUST-1.86%2B-000000?style=for-the-badge&labelColor=444)
![Licence](https://img.shields.io/badge/LICENCE-MIT-F5A623?style=for-the-badge&labelColor=444)

</div>

## What this is

A crypto wallet that runs in your terminal. Keys live in a local, encrypted
SQLite vault; there is no account to sign in to, no server that holds anything,
and no sync. The only things this program contacts are the chain nodes you
point it at, and — only if you supply a key for it — one indexer for BNB Chain
history. No update check, no telemetry, no analytics, and **no price service**:
the portfolio figure in the wallet list is quoted from a swap pool on the chain
itself, so showing it costs nothing in privacy. It is denominated in USDT
rather than dollars, and says so, because that is what was actually quoted.

It is built as **wallet → chain → assets** from the storage schema up. TRON is
what works today; see [Chains](#chains).

The vault is **one self-contained file**. Copy it to a USB stick, drop it back
in later, and everything is there. Someone who takes the file gets ciphertext.

```
$ neko-wallet
Email: zoe@example.com
Password:

  ⠸ deriving key... this is slow on purpose

┌──────────────────────── wallets ────────────────────────┐
│ > Savings          TPZrDZ...  8.655007 TRX  7.00 USDT   │
│   Daily            TWx3kQ...  0.000000 TRX  0.00 USDT   │
└─────────────────────────────────────────────────────────┘
 n new   i import   Enter open   s settings   q quit
```

> [!WARNING]
> **There is no recovery.** No master phrase, no reset link, no support. Forget
> the email or the password and the wallet is gone — you are in exactly the
> same position as a thief who took the file. That is the design, not a gap in
> it. Keep a copy of the `.db` file — it is encrypted, so copies are cheap and
> safe. Read [Backing up](#backing-up) before you consider writing a recovery
> phrase on paper: it is not the harmless step it sounds like.

---

## Install

**macOS, Linux, WSL:**

```bash
curl -fsSL https://raw.githubusercontent.com/zoefix/neko-wallet/main/install.sh | sh
```

**Windows PowerShell:**

```powershell
irm https://raw.githubusercontent.com/zoefix/neko-wallet/main/install.ps1 | iex
```

**Windows CMD:**

```
curl -fsSL https://raw.githubusercontent.com/zoefix/neko-wallet/main/install.cmd -o install.cmd && install.cmd
```

The installer downloads the release build for your machine, checks it against
the published checksums, puts it on your PATH, and stops. It creates no wallet
and asks for nothing.

Piping a script into a shell runs whatever that URL serves, so read it first if
you would rather — [install.sh](install.sh) is about two hundred lines.
`NEKO_WALLET_INSTALL_DIR` changes where the binary goes and
`NEKO_WALLET_NO_PATH=1` leaves your shell startup files alone.

### From source

```bash
cargo install --git https://github.com/zoefix/neko-wallet
```

One binary, no runtime dependencies — SQLCipher is compiled in. On Windows,
building from source needs Perl and NASM, because OpenSSL is built from source
there.

---

## Getting started

```bash
neko-wallet
```

First run asks for an email and a password and creates the vault. Then:

| Key | Does |
|---|---|
| `n` | new wallet (generates a recovery phrase, does **not** show it) |
| `i` | import from a recovery phrase or a private key |
| `Enter` | open → chain → assets |
| `y` | copy the address |
| `s` | send |
| `t` | transaction history |
| `m` | reveal this wallet's recovery phrase |
| `<` `>` | switch language |
| `L` | lock now |

A new wallet does not show you its phrase. You ask for it deliberately, later,
with `m` — and that asks for your password again.

---

## Chains

A wallet's single recovery phrase covers every chain beneath it — that is what
BIP44 derivation paths are for. Adding a chain never means a new wallet, a new
phrase, or a new thing to back up.

| Chain | Status | Assets |
|---|---|---|
| TRON | working | TRX, USDT (TRC20) |
| BNB Chain | working | BNB, USDT (BEP20) |
| Solana | working | SOL, USDT (SPL) |
| Bitcoin | listed in the interface, not built yet | — |

The same phrase gives a different address on each — that is correct and
universal, not a bug. Solana has no agreed derivation path; this wallet uses
`m/44'/501'/{i}'/0'`, which is Phantom's and Backpack's default. Solflare,
Ledger Live and Trust Wallet default to `m/44'/501'/{i}'` and will show
different addresses for the same phrase.

Chain-specific code is confined to one crate. Key derivation, storage,
encryption, and the interface are shared and chain-agnostic, and the database
has carried a `chains` table with SLIP-44 coin types since the first migration.
The parts of this README about energy, bandwidth and fee estimation are
TRON-specific; everything else applies to any chain.

---

## Your wallet is a file

This is the part worth understanding, because it is how backup works and how
you move between machines.

```
neko-wallet.db     ← this is the wallet. All of it.
```

One file. No `-wal`, no `-shm`, no sidecar state — the database runs in
`journal_mode = DELETE` specifically so that copying the file copies
everything. It is encrypted end to end, so it is safe to put in places you
would never put a key:

```bash
cp ~/.local/bin/neko-wallet.db /Volumes/USB/            # USB stick
cp ~/.local/bin/neko-wallet.db ~/Dropbox/backup/        # cloud storage
```

To use a vault from somewhere else:

```bash
neko-wallet set db /Volumes/USB/neko-wallet.db   # remembered from now on
neko-wallet --db /Volumes/USB/neko-wallet.db     # just this once
neko-wallet --where-db                           # which one am I opening?
neko-wallet unset db                             # back to the default search
```

`set db` needs no email or password: it records a path and nothing else. It
refuses a path that does not exist unless you add `--new`, because the usual
reason a path does not exist is a typo, and the result would be a first-run
screen that reads as *my wallet is gone*.

Which file is opened, in order: `--db` → `$NEKO_WALLET_DB` → the saved setting
→ next to the executable → your OS data directory.

---

## Backing up

There are two backups, and they fail in **opposite** ways. It is worth knowing
which is which before you need either.

### The `.db` file — this is the one to rely on

Encrypted end to end. A copy on a USB stick or in cloud storage is ciphertext:
whoever finds it still needs your email and your password. So copy it freely,
and copy it often.

```bash
cp ~/.local/bin/neko-wallet.db /Volumes/USB/
```

What it does not survive is forgetting your password. At that point the file is
exactly as useless to you as it is to a thief.

### The recovery phrase — a last resort, not a routine backup

> [!CAUTION]
> **Writing the twelve words on paper is not recommended as your main backup.**
> A phrase on paper is a plaintext bearer secret: anyone who reads it moves your
> money immediately — a burglar, a houseguest, a photo taken over your shoulder,
> anyone who simply takes it from you. No password is asked for and nothing
> warns you. The `.db` file has none of that property, which is exactly why it
> is the backup to rely on.

If you do write it down, it is worth the same as the money in the wallet and
wants storing the same way:

- A safe or a bank deposit box. Not a drawer, not a notebook, not the back of a
  book.
- **Never** a photograph, a cloud note, a message to yourself, or anything typed
  into a device. Anything that syncs has left your control.
- Split across two locations, if the amount justifies it, so that one break-in
  is not enough.

The one thing that would make a stolen phrase useless on its own is a BIP39
passphrase — a "25th word" kept only in your head. neko-wallet accepts one when
you **import** a wallet, but does not yet offer to set one when generating a new
one.

---

## Sending

The confirmation is not a keypress. You retype the **last six characters of the
destination address**.

```
   To    TPZrDZ TUWQqqUTVRxAmSdQyGXSSg AUyyk4
         ^^^^^^                        ^^^^^^

   Type the last 6 characters to confirm:
         [ AUyyk_ ]  5/6
```

Clipboard-hijacking malware that swaps an address at paste time is the largest
real-world loss vector for command-line wallets, and pressing `y` does not stop
it — you are looking at what you *believe* you pasted. Retyping the tail is the
only step that forces your eyes onto the bytes about to be signed.

Then the password, in full, again. An unlocked terminal somebody walked away
from must not be enough to move money.

**Fees are shown broken down**, because TRON has no flat fee: a transfer spends
bandwidth, a contract call spends energy, and TRX is burned only for the part
your account cannot already cover. Energy is simulated against your exact
transfer, never guessed — the same USDT transfer costs about twice as much to
an address that has never held USDT.

### Address poisoning

Attackers send you dust from an address that looks like one you use, hoping you
copy the wrong one out of your history. Three defences:

- Dust is hidden by default.
- **No address is ever abbreviated.** A `TPZr…yyk4` rendering makes a lookalike
  indistinguishable from the real thing.
- Sending to an address that resembles — but is not — one from your history
  raises a warning.

---

## The recovery phrase

`m` shows a wallet's twelve words. Before that:

- Your password is re-derived **in full**, Argon2id and all, even though the
  vault is already open.
- Words are masked; arrow keys reveal **one at a time**. A screenshot or a
  `tmux capture-pane` leaks one word, not twelve.
- It hides itself after 60 seconds.
- **Copying is not refused — it does not exist.** There is no code path from
  that screen to any clipboard backend, so no future bug can invert a check and
  turn it on.

The screen also tells you what none of this stops: a camera, someone behind
you, `script`, a screen recorder, or malware already on the machine. A wallet
that claims a terminal can keep a secret on screen is lying.

Before you copy the words onto paper, read [Backing up](#backing-up) — a written
phrase is a bearer secret, and the `.db` file is the safer backup.

---

## Language

English, 简体中文, 繁體中文, 日本語. Detected from your system, changed any time
with `<` and `>` in settings, remembered per vault.

Every language is listed in its own script, so somebody who cannot read the
current one can still find their way out. Translations are checked at compile
time: a missing key, a mismatched `%{placeholder}`, or an ambiguous-width
character fails the build rather than reaching a user holding money.

---

## How it works

### Keys

```
email + password ─Argon2id─► stretched ─HKDF─┬─► file key ──► SQLCipher (whole database)
                                             └─► KEK ──► unwraps MK (32 random bytes)
                                                          │
                                                     HKDF ├─► k_data   field-level AEAD
                                                          └─► k_index  blind index
```

Each wallet has its own independent BIP39 phrase, encrypted with `k_data`.
`MK` is random and unrelated to any phrase — which is precisely why there is no
master recovery phrase. Changing your password re-wraps 32 bytes; it does not
re-encrypt anything else, and it is crash-safe (both wrappings are kept until
the switch completes).

### Two layers

The whole file is SQLCipher-encrypted (AES-256-CBC + HMAC-SHA512). On top of
that, the things whose leak costs you money get their own XChaCha20-Poly1305
envelope, keyed separately:

| Field | Second layer |
|---|---|
| recovery phrase entropy | yes |
| private keys | yes |
| wallet labels | yes — "company reserve" is intelligence by itself |
| TronGrid API key | yes |
| addresses, transactions, balances | no — public on-chain data, and needs indexing |

Every ciphertext's AAD binds it to its table, column, row id and key version, so
a row cannot be swapped for another and still decrypt.

### The salt problem

A self-contained file has to be decryptable with nothing but itself, but you
need the salt *before* you can decrypt. SQLCipher stores its 16-byte salt as
the plaintext first 16 bytes of the file — so we use that slot as our own
header: format version, KDF profile, and 14 random bytes unique to this vault.

Putting the profile in an unauthenticated header is safe: changing it produces
a different key, so the file simply refuses to open. A downgrade attack
disables itself.

### Cost

Argon2id, calibrated on your machine at setup — 128 MiB / t=4, 256 MiB / t=3,
or 1 GiB / t=4. The profile id lives in the header, so a vault created on a
fast machine still opens on a slow one.

Passwords must clear 70 bits of estimated entropy, measured as the *lower* of a
charset estimate and a pattern estimate, so `MyWallet2026!` is scored as what
it is.

### Transactions are built locally

`/wallet/createtransaction` is never called. The node supplies a block
reference; the bytes to be signed are assembled here, with a hand-written
protobuf encoder, and the signature is checked by recovering the public key
from it and confirming the address matches. A malicious node cannot hand you a
transaction that pays somebody else and have you sign it.

---

## Upgrading

Run the installer again.

```bash
curl -fsSL https://raw.githubusercontent.com/zoefix/neko-wallet/main/install.sh | sh
```

It replaces the binary and never touches your `.db` — it says so when it
finishes.

**There is deliberately no self-update.** A wallet that can replace its own
executable has a remote code path into the machine holding the keys, and no
amount of signing makes that path smaller than not having it. CI enforces this:
the build fails if a self-replacement crate reappears in the dependency tree,
or if the program grows a network destination outside a short list that has
to be edited deliberately: the three chain nodes, the optional BNB Chain
history indexer, and explorer links the wallet copies but never fetches.

---

## What this protects you from, and what it does not

**Protects:**

- The `.db` file being copied, leaked to a cloud sync, or recovered from a
  discarded disk — without the password it is ciphertext.
- Tampering with individual rows or columns of that ciphertext.
- Weakening the KDF parameters to make the file easier to crack.
- A malicious or compromised node trying to get you to sign a transfer to
  someone else.

**Does not:**

- **Forgetting your password.** Permanent, total loss. This is what "no
  recovery" means.
- A keylogger or remote-access trojan on the machine you run this on. It is not
  a hardware wallet: while the vault is open, keys are in this computer's
  memory, and nothing a terminal program does changes that.
- A memory dump of the unlocked process. Secrets are zeroized and kept out of
  `String` where possible; the operating system still wins.
- Someone reading the recovery phrase off your screen.
- A weak password. `Password123` with Argon2id is still `Password123`.

---

## Development

```bash
cargo test --workspace                              # 275 tests
cargo clippy --workspace --all-targets -- -D warnings
```

Cryptography, key derivation and TRON transaction encoding are pinned against
frozen cross-language vectors in `vectors/` — byte-exact, so a refactor that
changes a derived address or a transaction id fails immediately.

```
neko-crypto   Argon2id / XChaCha20-Poly1305 / HKDF. No IO, no SQL, no async.
neko-vault    key hierarchy, KDF profiles, password policy, normalization
neko-store    SQLCipher, migrations, field-level envelopes  (never derives keys)
neko-hd       BIP39 / BIP32 / BIP44 and SLIP-0010; TRON, EVM and Solana addresses
neko-tron     TRON only: protobuf, transaction building and signing, node client
neko-evm      BNB Chain: RLP, EIP-155 signing, ABI encoding, JSON-RPC
neko-solana   Solana: Ed25519, the wire format, token accounts, cluster RPC
neko-core     the facade the UI talks to
neko-i18n     compile-time-checked translation tables
neko-tui      ratatui interface
```

---

## Licence

MIT. See [LICENSE](LICENSE).
