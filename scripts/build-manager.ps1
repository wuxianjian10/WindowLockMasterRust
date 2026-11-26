param([switch]$Test)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$env:RUSTUP_HOME = "$root/toolchain/rustup"
$env:CARGO_HOME = "$root/toolchain/cargo"
$env:NAUI_WINAPPSDK_PACKAGES = "$root/toolchain/nuget"
Set-Location $root
$cargo = "$env:CARGO_HOME/bin/cargo.exe"
if ($Test) {
    & $cargo test --lib
    if ($LASTEXITCODE -ne 0) { throw "Rust regression tests failed" }
}
& $cargo build --release --features portable --bin WindowLockMasterRust
if ($LASTEXITCODE -ne 0) { throw "Rust manager build failed" }
