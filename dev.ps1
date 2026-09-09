# Dev mode: install tauri-cli (one-time) then run "tauri dev" (auto-reload on UI changes).
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    Write-Host "ERROR: cargo not found. Install Rust first: https://rustup.rs" -ForegroundColor Red
    exit 1
}

# Privacy: neutral cargo home (see build.ps1)
$neutralHome = Join-Path $PSScriptRoot ".cargo-neutral"
if (Test-Path (Join-Path $neutralHome "registry\index")) {
    $env:CARGO_HOME = $neutralHome
}

if (-not (Get-Command cargo-tauri -ErrorAction SilentlyContinue)) {
    Write-Host "First run: installing tauri-cli (one-time, 3-8 min)..." -ForegroundColor Cyan
    cargo install tauri-cli --version "^2" --locked
    if ($LASTEXITCODE -ne 0) {
        Write-Host "tauri-cli install FAILED" -ForegroundColor Red
        exit 1
    }
}

Write-Host "Starting dev mode (edit files under ui/ to auto-reload; Ctrl+C to exit)..." -ForegroundColor Cyan
cargo tauri dev
