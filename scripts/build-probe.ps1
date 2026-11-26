param([switch]$Portable)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$env:RUSTUP_HOME = "$root/toolchain/rustup"
$env:CARGO_HOME = "$root/toolchain/cargo"
$env:TMP = "$root/toolchain/downloads"
$env:TEMP = $env:TMP
$env:NAUI_WINAPPSDK_PACKAGES = "$root/toolchain/nuget"
Set-Location $root
$argsBuild = @('build','--release','--bin','WindowLockMasterRustProbe')
if ($Portable) { $argsBuild += @('--features','portable') } else { $argsBuild += @('--target-dir','target/framework') }
& "$env:CARGO_HOME/bin/cargo.exe" @argsBuild
if ($LASTEXITCODE -ne 0) { throw "cargo build failed: $LASTEXITCODE" }
