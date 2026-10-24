# Runs the same checks CI should: formatting, lints (warnings are errors) and tests.
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)
cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { throw 'Formatting differs: run `cargo fmt --all`.' }
cargo lint
if ($LASTEXITCODE -ne 0) { throw 'Clippy found problems.' }
cargo test --workspace
if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }
Write-Host 'All checks passed.'
