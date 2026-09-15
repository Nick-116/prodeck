$ErrorActionPreference = "Stop"

function Require-Command([string]$Name, [string]$Help) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) {
        throw "$Name was not found. $Help"
    }
}

Require-Command "node" "Install Node.js 22 LTS, then open a new PowerShell window."
Require-Command "npm" "Install Node.js 22 LTS, then open a new PowerShell window."
Require-Command "rustc" "Install Rust with the stable MSVC toolchain, then open a new PowerShell window."
Require-Command "cargo" "Install Rust with the stable MSVC toolchain, then open a new PowerShell window."

$rustHost = rustc -vV | Select-String "host:"
if ($rustHost -notmatch "windows-msvc") {
    throw "The Rust MSVC toolchain is required. Run: rustup default stable-msvc"
}

Write-Host "Installing exact frontend dependencies..." -ForegroundColor Cyan
npm ci

Write-Host "Running frontend tests..." -ForegroundColor Cyan
npm test
npx tsc --noEmit

# Before the Rust tests: web.rs embeds dist/ via include_dir! at compile time,
# so cargo needs the frontend built first. Same reason as in the CI workflow.
Write-Host "Building the frontend..." -ForegroundColor Cyan
npm run build

Write-Host "Running Rust tests on Windows..." -ForegroundColor Cyan
Push-Location (Join-Path $PSScriptRoot "..\src-tauri")
try {
    cargo test --locked
}
finally {
    Pop-Location
}

Write-Host "Building the Windows installer..." -ForegroundColor Cyan
Push-Location (Join-Path $PSScriptRoot "..")
try {
    npm run tauri build -- --bundles nsis --config src-tauri/tauri.windows.conf.json
}
finally {
    Pop-Location
}

$bundle = Join-Path $PSScriptRoot "..\src-tauri\target\release\bundle\nsis"
Write-Host "Build complete: $bundle" -ForegroundColor Green
