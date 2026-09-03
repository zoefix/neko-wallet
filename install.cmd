@echo off
REM neko-wallet installer for Windows CMD.
REM
REM   curl -fsSL https://raw.githubusercontent.com/zoefix/neko-wallet/main/install.cmd -o install.cmd ^&^& install.cmd
REM
REM CMD has no way to run a script straight off a pipe, so this file exists
REM only to hand the real work to install.ps1. Everything it does is
REM described there.

setlocal

set "NEKO_WALLET_PS1=https://raw.githubusercontent.com/zoefix/neko-wallet/main/install.ps1"

where powershell >nul 2>&1
if errorlevel 1 (
    echo error: PowerShell was not found on PATH.
    echo Install from https://github.com/zoefix/neko-wallet/releases instead.
    exit /b 1
)

REM -NoProfile so a user profile cannot change how the installer behaves;
REM -ExecutionPolicy Bypass applies to this one process only.
powershell -NoProfile -ExecutionPolicy Bypass -Command ^
  "[Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12; irm '%NEKO_WALLET_PS1%' | iex"

set "RESULT=%ERRORLEVEL%"

REM The one-liner leaves this file in the current directory; clean it up so
REM the install does not litter.
if exist "%~f0" (
    start "" /b cmd /c "timeout /t 1 >nul & del /q "%~f0"" >nul 2>&1
)

exit /b %RESULT%
