# One-click build script: compile release build and optionally launch it.
# First run downloads all crates and compiles (5-15 min); later runs are incremental.
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

Write-Host "==============================================" -ForegroundColor Cyan
Write-Host "  File Toolbox - Release Build" -ForegroundColor Cyan
Write-Host "==============================================" -ForegroundColor Cyan

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "ERROR: cargo not found. Install Rust first: https://rustup.rs" -ForegroundColor Red
    exit 1
}

# Workaround for "spurious network error ... HTTP/2 framing layer":
# some proxies/firewalls break HTTP/2 multiplexing; forcing HTTP/1.1 makes downloads stable.
$env:CARGO_HTTP_MULTIPLEXING = "false"
$env:CARGO_NET_RETRY = "10"

# Privacy: use a neutral cargo home so compiled dependency paths never contain
# the username of the machine that built the binary (audit with scripts\audit-exe.js).
$neutralHome = Join-Path $PSScriptRoot ".cargo-neutral"
if (Test-Path (Join-Path $neutralHome "registry\index")) {
    $env:CARGO_HOME = $neutralHome
}

Write-Host "Building (first build takes 5-15 min, please wait)..."
cargo build --release --manifest-path src-tauri/Cargo.toml
if ($LASTEXITCODE -ne 0) {
    Write-Host "Build FAILED. Please copy the error messages above and send them to the developer." -ForegroundColor Red
    exit 1
}

$exe = Join-Path $PSScriptRoot "src-tauri\target\release\file-toolbox.exe"
if (-not (Test-Path $exe)) {
    Write-Host "Executable not found: $exe" -ForegroundColor Red
    exit 1
}

Write-Host ""
Write-Host "[OK] Build succeeded!" -ForegroundColor Green
Write-Host "Executable: $exe" -ForegroundColor Yellow
Write-Host "It is a portable single file - double-click to run, no install needed." -ForegroundColor Gray
Write-Host ""
Write-Host "Launch now? (y/n)"
$ans = Read-Host
if ($ans -eq "y") {
    Start-Process $exe
    Write-Host "Launched."
}
