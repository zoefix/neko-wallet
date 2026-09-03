#!/bin/sh
# neko-wallet installer and updater for macOS, Linux and WSL.
#
#   curl -fsSL https://raw.githubusercontent.com/zoefix/neko-wallet/main/install.sh | sh
#
# Downloads the release build for this machine, checks it against the published
# checksums, puts it on PATH, and stops. It does not create a wallet, ask for
# anything, or touch an existing vault.
#
# This script is also how you update. neko-wallet has no self-update: a wallet
# that can replace its own executable is a wallet with a remote code path into
# the machine holding the keys, and no amount of signing makes that path
# smaller than not having it. Run this again and the binary is replaced; your
# vault file is never opened, moved, or read.
#
# Overrides:
#   NEKO_WALLET_VERSION      tag to install (default: the latest release)
#   NEKO_WALLET_INSTALL_DIR  where to put the binary (default: ~/.local/bin)
#   NEKO_WALLET_NO_PATH=1    install, but leave shell startup files alone
#
# POSIX sh on purpose: this runs under whatever `sh` is, before neko-wallet
# exists to tell anyone what it needs.

set -eu

REPO="zoefix/neko-wallet"
BIN="neko-wallet"
INSTALL_DIR="${NEKO_WALLET_INSTALL_DIR:-$HOME/.local/bin}"

# Ed25519 release-signing public key, 64 hex characters.
#
# Embedded here rather than downloaded alongside the release: fetching the key
# from the same place as the signature proves nothing, because anyone able to
# forge one could forge the other. This file arrives over TLS from the
# repository itself, so a key baked into it is anchored to the repository
# rather than to a release asset.
#
# Empty until release signing is set up, in which case only the checksum is
# verified and the script says so.
SIGNING_KEY_HEX=""

red() { printf '\033[31m%s\033[0m\n' "$1" >&2; }
dim() { printf '\033[2m%s\033[0m\n' "$1"; }
bold() { printf '\033[1m%s\033[0m\n' "$1"; }
die() { red "error: $1"; exit 1; }

# --- what are we running on -------------------------------------------------

detect_target() {
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$os" in
        Darwin) os_part="apple-darwin" ;;
        Linux)  os_part="unknown-linux-gnu" ;;
        *) die "unsupported operating system: $os. Build from source with 'cargo install --git https://github.com/$REPO'." ;;
    esac
    case "$arch" in
        x86_64 | amd64) arch_part="x86_64" ;;
        arm64 | aarch64) arch_part="aarch64" ;;
        *) die "unsupported architecture: $arch. Build from source with 'cargo install --git https://github.com/$REPO'." ;;
    esac
    printf '%s-%s' "$arch_part" "$os_part"
}

# --- fetching ---------------------------------------------------------------

if command -v curl >/dev/null 2>&1; then
    fetch() { curl -fsSL "$1" -o "$2"; }
    fetch_stdout() { curl -fsSL "$1"; }
elif command -v wget >/dev/null 2>&1; then
    fetch() { wget -qO "$2" "$1"; }
    fetch_stdout() { wget -qO- "$1"; }
else
    die "neither curl nor wget is available"
fi

latest_tag() {
    # Parsed with sed rather than a JSON tool, which is not guaranteed to be
    # present on a machine that has nothing installed yet.
    fetch_stdout "https://api.github.com/repos/$REPO/releases/latest" \
        | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' \
        | head -n 1
}

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | cut -d' ' -f1
    else
        die "no sha256 tool found (looked for sha256sum and shasum)"
    fi
}

# --- shell startup files ----------------------------------------------------

shell_rc() {
    case "$(basename "${SHELL:-}")" in
        zsh)  printf '%s' "${ZDOTDIR:-$HOME}/.zshrc" ;;
        bash) [ "$(uname -s)" = "Darwin" ] && printf '%s' "$HOME/.bash_profile" || printf '%s' "$HOME/.bashrc" ;;
        fish) printf '%s' "$HOME/.config/fish/config.fish" ;;
        *)    printf '%s' "$HOME/.profile" ;;
    esac
}

add_to_path() {
    rc="$(shell_rc)"
    marker="# added by the neko-wallet installer"

    if [ -f "$rc" ] && grep -Fq "$marker" "$rc"; then
        dim "PATH entry already present in $rc"
        return 0
    fi

    mkdir -p "$(dirname "$rc")"
    if [ "$(basename "$rc")" = "config.fish" ]; then
        printf '\n%s\nfish_add_path %s\n' "$marker" "$INSTALL_DIR" >> "$rc"
    else
        # $PATH is deliberately left unexpanded: the literal string has to
        # reach the startup file, not this installer's value of it.
        # shellcheck disable=SC2016
        printf '\n%s\nexport PATH="%s:$PATH"\n' "$marker" "$INSTALL_DIR" >> "$rc"
    fi
    printf '  added %s to PATH in %s\n' "$INSTALL_DIR" "$rc"
    NEEDS_RELOAD="$rc"
}

# --- go ---------------------------------------------------------------------

target="$(detect_target)"
version="${NEKO_WALLET_VERSION:-$(latest_tag)}"
[ -n "$version" ] || die "no published release found for $REPO yet. Build from source with 'cargo install --git https://github.com/$REPO'."

asset="$BIN-$target.tar.gz"
base="https://github.com/$REPO/releases/download/$version"

# An existing vault sits next to the binary by default, so an update is
# replacing a file in a directory that may hold the only copy of somebody's
# keys. Say what is about to happen to it: nothing.
existing_vault=""
[ -f "$INSTALL_DIR/$BIN.db" ] && existing_vault="$INSTALL_DIR/$BIN.db"

if [ -x "$INSTALL_DIR/$BIN" ]; then
    bold "Updating $BIN to $version ($target)"
else
    bold "Installing $BIN $version ($target)"
fi

tmp="$(mktemp -d)"
# Clean up on any exit, including an interrupt partway through.
trap 'rm -rf "$tmp"' EXIT INT TERM

printf '  downloading %s\n' "$asset"
fetch "$base/$asset" "$tmp/$asset" \
    || die "no build for $target in $version. See https://github.com/$REPO/releases"

# The checksum is what makes a truncated or substituted download fail loudly
# instead of installing.
printf '  verifying checksum\n'
fetch "$base/SHA256SUMS" "$tmp/SHA256SUMS" || die "cannot download SHA256SUMS"
# Compared as a whole field rather than by regex: the asset name contains
# dots, and matching it loosely could pick up a different line.
expected="$(awk -v want="$asset" '{ name = $2; sub(/^\*/, "", name); if (name == want) { print $1; exit } }' "$tmp/SHA256SUMS")"
[ -n "$expected" ] || die "SHA256SUMS has no entry for $asset"
actual="$(sha256_of "$tmp/$asset")"
[ "$expected" = "$actual" ] || die "checksum mismatch for $asset - do not use this download"

# The Ed25519 signature is the check that still holds if the GitHub account
# itself is compromised. OpenSSL 3 is needed for -rawin; LibreSSL, which macOS
# ships as `openssl`, is not, so this degrades to the checksum rather than
# refusing to install.
if [ -n "$SIGNING_KEY_HEX" ]; then
    if command -v openssl >/dev/null 2>&1 && openssl version 2>/dev/null | grep -q "^OpenSSL 3" \
       && command -v xxd >/dev/null 2>&1; then
        fetch "$base/SHA256SUMS.sig" "$tmp/SHA256SUMS.sig" || die "cannot download SHA256SUMS.sig"
        # A raw Ed25519 key becomes a PEM by prefixing the fixed SPKI header.
        printf '302a300506032b6570032100%s' "$SIGNING_KEY_HEX" | xxd -r -p > "$tmp/key.der"
        openssl pkey -pubin -inform DER -in "$tmp/key.der" -out "$tmp/key.pem" 2>/dev/null \
            || die "the signing key built into this installer is malformed"
        tr -d '\n\r ' < "$tmp/SHA256SUMS.sig" | xxd -r -p > "$tmp/sig.bin"
        openssl pkeyutl -verify -pubin -inkey "$tmp/key.pem" -rawin \
            -in "$tmp/SHA256SUMS" -sigfile "$tmp/sig.bin" >/dev/null 2>&1 \
            || die "the release signature did not verify - do not use this download"
        printf '  signature verified\n'
    else
        dim "  signature not checked (needs OpenSSL 3 and xxd); the checksum was verified"
    fi
else
    dim "  this release is not signed yet; the checksum was verified"
fi

printf '  installing to %s\n' "$INSTALL_DIR"
tar -xzf "$tmp/$asset" -C "$tmp"
found="$(find "$tmp" -type f -name "$BIN" 2>/dev/null | head -n 1)"
[ -n "$found" ] || die "the archive does not contain a $BIN executable"

mkdir -p "$INSTALL_DIR"
# Replaced by rename so that an installer run does not corrupt a copy that is
# currently executing. Only "$BIN" is ever written - never "$BIN.db".
cp "$found" "$INSTALL_DIR/$BIN.new"
chmod 755 "$INSTALL_DIR/$BIN.new"
mv -f "$INSTALL_DIR/$BIN.new" "$INSTALL_DIR/$BIN"

NEEDS_RELOAD=""
case ":$PATH:" in
    *":$INSTALL_DIR:"*) dim "  $INSTALL_DIR is already on PATH" ;;
    *) [ "${NEKO_WALLET_NO_PATH:-}" = "1" ] || add_to_path ;;
esac

printf '\n'
printf '\033[32m✓\033[0m %s\n' "$("$INSTALL_DIR/$BIN" --machine-readable 2>/dev/null || echo "$BIN $version") installed"
if [ -n "$existing_vault" ]; then
    printf '\033[32m✓\033[0m your vault was not touched: %s\n' "$existing_vault"
fi
printf '\n'
if [ -n "$NEEDS_RELOAD" ]; then
    bold "Open a new terminal (or run: . $NEEDS_RELOAD), then:"
else
    bold "Next:"
fi
printf '\n'
printf '    neko-wallet                 # open it (first run sets up your vault)\n'
printf '    neko-wallet --where-db      # which vault file it opens\n'
printf '    neko-wallet set db <path>   # point it at a vault on a USB stick\n'
printf '\n'
dim "There is no recovery: forget the email or the password and the wallet is"
dim "gone. Copy the .db file somewhere safe once you have added a wallet - it"
dim "is encrypted, self-contained, and the backup to rely on."
