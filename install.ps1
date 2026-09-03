# neko-wallet installer and updater for Windows.
#
#   irm https://raw.githubusercontent.com/zoefix/neko-wallet/main/install.ps1 | iex
#
# Downloads the release build for this machine, checks it against the published
# checksums, puts it on PATH, and stops. It does not create a wallet, ask for
# anything, or touch an existing vault.
#
# This script is also how you update. neko-wallet has no self-update: a wallet
# that can replace its own executable is a wallet with a remote code path into
# the machine holding the keys. Run this again and the binary is replaced; your
# vault file is never opened, moved, or read.
#
# Overrides:
#   $env:NEKO_WALLET_VERSION      tag to install (default: the latest release)
#   $env:NEKO_WALLET_INSTALL_DIR  where to put the binary
#   $env:NEKO_WALLET_NO_PATH=1    install, but leave PATH alone

$ErrorActionPreference = 'Stop'

$Repo = 'zoefix/neko-wallet'
$Bin  = 'neko-wallet'

$InstallDir = if ($env:NEKO_WALLET_INSTALL_DIR) {
    $env:NEKO_WALLET_INSTALL_DIR
} else {
    Join-Path $env:LOCALAPPDATA "Programs\$Bin"
}

function Write-Step($Message) { Write-Host "  $Message" }
function Write-Dim($Message)  { Write-Host "  $Message" -ForegroundColor DarkGray }
function Fail($Message) {
    Write-Host "error: $Message" -ForegroundColor Red
    exit 1
}

# Only x86_64 Windows is published; ARM64 machines run the x64 build under
# emulation, which works but is worth saying out loud.
$arch = $env:PROCESSOR_ARCHITECTURE
if ($arch -eq 'ARM64') {
    Write-Dim 'no native ARM64 build yet; installing the x64 build, which Windows will emulate'
}
$Target = 'x86_64-pc-windows-msvc'

# TLS 1.2 for Windows PowerShell 5, which still defaults to SSL3/TLS1.
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

$Version = $env:NEKO_WALLET_VERSION
if (-not $Version) {
    try {
        $release = Invoke-RestMethod "https://api.github.com/repos/$Repo/releases/latest" `
            -Headers @{ 'User-Agent' = 'neko-wallet-installer' }
        $Version = $release.tag_name
    } catch {
        Fail "no published release found for $Repo yet. Build from source with 'cargo install --git https://github.com/$Repo'."
    }
}

$Asset = "$Bin-$Target.tar.gz"
$Base  = "https://github.com/$Repo/releases/download/$Version"

# An existing vault sits next to the binary by default, so an update replaces a
# file in a directory that may hold the only copy of somebody's keys. Say what
# is about to happen to it: nothing.
$ExistingVault = Join-Path $InstallDir "$Bin.db"
$HadVault = Test-Path $ExistingVault

$dest = Join-Path $InstallDir "$Bin.exe"
if (Test-Path $dest) {
    Write-Host "Updating $Bin to $Version ($Target)" -ForegroundColor White
} else {
    Write-Host "Installing $Bin $Version ($Target)" -ForegroundColor White
}

$Tmp = Join-Path ([IO.Path]::GetTempPath()) ([IO.Path]::GetRandomFileName())
New-Item -ItemType Directory -Path $Tmp -Force | Out-Null

try {
    Write-Step "downloading $Asset"
    try {
        Invoke-WebRequest "$Base/$Asset" -OutFile (Join-Path $Tmp $Asset) -UseBasicParsing
    } catch {
        Fail "no build for $Target in $Version. See https://github.com/$Repo/releases"
    }

    # The checksum is what makes a truncated or substituted download fail
    # loudly instead of installing.
    Write-Step 'verifying checksum'
    Invoke-WebRequest "$Base/SHA256SUMS" -OutFile (Join-Path $Tmp 'SHA256SUMS') -UseBasicParsing
    $line = Get-Content (Join-Path $Tmp 'SHA256SUMS') |
        Where-Object { $_ -match "\s\*?$([regex]::Escape($Asset))$" } |
        Select-Object -First 1
    if (-not $line) { Fail "SHA256SUMS has no entry for $Asset" }
    $expected = ($line -split '\s+')[0].ToLower()
    $actual = (Get-FileHash (Join-Path $Tmp $Asset) -Algorithm SHA256).Hash.ToLower()
    if ($expected -ne $actual) {
        Fail "checksum mismatch for $Asset - do not use this download"
    }

    # tar has shipped with Windows 10 1803 and later, so the archive is
    # unpacked without adding a dependency.
    Write-Step "installing to $InstallDir"
    tar -xzf (Join-Path $Tmp $Asset) -C $Tmp
    if ($LASTEXITCODE -ne 0) { Fail 'cannot unpack the archive' }

    $exe = Get-ChildItem -Path $Tmp -Recurse -Filter "$Bin.exe" | Select-Object -First 1
    if (-not $exe) { Fail "the archive does not contain $Bin.exe" }

    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
    # Windows will not overwrite a running executable; move the old one aside
    # so an upgrade works even with a session open. Only the .exe is ever
    # touched - never the .db beside it.
    if (Test-Path $dest) {
        $old = "$dest.old"
        Remove-Item $old -Force -ErrorAction SilentlyContinue
        try { Move-Item $dest $old -Force } catch { }
    }
    Copy-Item $exe.FullName $dest -Force

    $needsNewShell = $false
    if ($env:NEKO_WALLET_NO_PATH -ne '1') {
        # The user's own PATH, not the machine's: no elevation, and nothing
        # that affects anyone else on this computer.
        $userPath = [Environment]::GetEnvironmentVariable('Path', 'User')
        if ($null -eq $userPath) { $userPath = '' }
        $already = $userPath -split ';' | Where-Object { $_ -eq $InstallDir }
        if ($already) {
            Write-Dim "$InstallDir is already on PATH"
        } else {
            $joined = if ($userPath.TrimEnd(';')) { "$($userPath.TrimEnd(';'));$InstallDir" } else { $InstallDir }
            [Environment]::SetEnvironmentVariable('Path', $joined, 'User')
            $env:Path = "$env:Path;$InstallDir"
            Write-Step "added $InstallDir to your PATH"
            $needsNewShell = $true
        }
    }

    Write-Host ''
    $reported = try { & $dest --machine-readable } catch { "$Bin $Version" }
    Write-Host "OK  $reported installed" -ForegroundColor Green
    if ($HadVault) {
        Write-Host "OK  your vault was not touched: $ExistingVault" -ForegroundColor Green
    }
    Write-Host ''
    if ($needsNewShell) {
        Write-Host 'Open a new terminal, then:' -ForegroundColor White
    } else {
        Write-Host 'Next:' -ForegroundColor White
    }
    Write-Host ''
    Write-Host '    neko-wallet                 # open it (first run sets up your vault)'
    Write-Host '    neko-wallet --where-db      # which vault file it opens'
    Write-Host '    neko-wallet set db <path>   # point it at a vault on a USB stick'
    Write-Host ''
    Write-Dim 'There is no recovery: forget the email or the password and the wallet is'
    Write-Dim 'gone. Copy the .db file somewhere safe once you have added a wallet - it'
    Write-Dim 'is encrypted, self-contained, and the backup to rely on.'
} finally {
    Remove-Item $Tmp -Recurse -Force -ErrorAction SilentlyContinue
}
