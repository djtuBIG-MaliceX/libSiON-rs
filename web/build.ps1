param([switch]$SkipInstall)

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath (Join-Path $PSScriptRoot '..')

rustup target add wasm32-unknown-unknown

if (-not $SkipInstall) {
    $haveCli = Get-Command wasm-bindgen -ErrorAction SilentlyContinue
    if (-not $haveCli) {
        $listed = cargo install --list | Select-String -SimpleMatch -Quiet 'wasm-bindgen-cli v'
        if (-not $listed) {
            cargo install wasm-bindgen-cli --locked
        }
    }
}

cargo build --release --target wasm32-unknown-unknown --no-default-features --features wasm

$wasm = Join-Path 'target/wasm32-unknown-unknown/release' 'libsion.wasm'
if (-not (Test-Path -LiteralPath $wasm)) {
    $found = Get-ChildItem target/wasm32-unknown-unknown/release -Filter *.wasm | Select-Object -First 1
    if ($found) { $wasm = $found.FullName } else { throw "no wasm artifact under target/wasm32-unknown-unknown/release" }
}

$bindgen = (Get-Command wasm-bindgen -ErrorAction SilentlyContinue)?.Source
if (-not $bindgen) { $bindgen = Join-Path $env:USERPROFILE '.cargo\bin\wasm-bindgen.exe' }
if (-not (Test-Path -LiteralPath $bindgen)) { throw 'wasm-bindgen not found: run without -SkipInstall' }

New-Item -ItemType Directory -Force -Path web/pkg | Out-Null
& $bindgen --target web --out-name libsion_wasm --out-dir web/pkg $wasm
if ($LASTEXITCODE -ne 0) { throw "wasm-bindgen failed with exit code $LASTEXITCODE" }

Write-Host ''
Write-Host "done ($wasm -> web/pkg/libsion_wasm.js). serve with:  cd web; python -m http.server 8000"
