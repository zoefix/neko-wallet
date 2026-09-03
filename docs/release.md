# Releasing neko-wallet

The install scripts are the whole distribution channel. neko-wallet cannot
update itself, on purpose: a wallet able to replace its own executable is a
wallet with a remote code path into the machine holding the keys, and no amount
of signing makes that path smaller than not having it. So what users run is
whatever `install.sh` hands them, and protecting that is the entire job here.

## Cutting a release

1. **Bump the version** in the workspace `Cargo.toml`. CI refuses a tag that
   disagrees with it - if they drift, users install a binary that reports a
   different version from the one they asked for.

   ```bash
   git commit -am "Release v0.2.0" && git tag v0.2.0 && git push --tags
   ```

2. **CI builds and publishes.** Every target is compiled, packaged as
   `neko-wallet-<target>.tar.gz`, hashed into `SHA256SUMS`, and attached to a
   GitHub release. A missing platform fails the job rather than shipping a
   release those users are told does not exist for them.

3. **Check it from a clean machine.**

   ```bash
   curl -fsSL https://raw.githubusercontent.com/zoefix/neko-wallet/main/install.sh | sh
   neko-wallet --machine-readable
   ```

## Signing (optional, and worth doing)

Without a signing key the release still publishes; `install.sh` verifies the
checksum, says plainly that the release is unsigned, and continues. The
checksum proves the download arrived intact from whoever served it. The
signature is what still holds if the GitHub account itself is compromised.

Generate an Ed25519 key **on a machine that is not this one**:

```bash
openssl genpkey -algorithm ed25519 -out release.key
openssl pkey -in release.key -text -noout   # copy the 32-byte private and public halves
```

- Put the **private** half (64 hex chars) in the repository secret
  `RELEASE_SIGNING_KEY`.
- Put the **public** half in `install.sh` as `SIGNING_KEY_HEX`, and commit it.

The public key travels inside `install.sh`, which reaches the user over TLS
from the repository itself. Serving the key next to the signature would prove
nothing: anyone able to forge one could forge the other.

CI verifies its own signature against the key in `install.sh` before
publishing, so a release cannot go out signed with a key no user trusts.

### Rotating

Commit the new public key to `install.sh` and release **signed with the old
key**, so people running the current installer still verify it. Switch the
secret to the new key on the release after that.

### If the key is compromised

There is no revocation. Rotate, say so in the README and the release notes, and
treat every release the attacker could have signed as suspect. This is why the
key is generated off the build machine.

## What the installer checks

| Check | Stops |
|---|---|
| SHA256 against `SHA256SUMS` | a truncated or substituted download |
| Ed25519 over `SHA256SUMS` | a compromised GitHub account |
| write-then-rename | corrupting a copy that is currently running |
| only ever writes the binary | touching a `neko-wallet.db` in the same directory |

That last row matters more here than in most projects: the vault sits beside
the executable by default, so an update is writing into a directory that may
hold the only copy of somebody's keys. The installer never opens, moves, or
reads a `.db`, and says so when it finishes.
