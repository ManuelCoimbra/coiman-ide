# Runs the same checks CI should: the no-network rule, formatting, lints
# (warnings are errors) and tests.
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

# Privacy baseline (see PRIVACY.md): no networking crates in the dependency tree.
$forbidden = 'reqwest', 'hyper', 'ureq', 'curl', 'isahc', 'attohttpc', 'surf', 'h2', 'rustls',
    'native-tls', 'openssl', 'tokio', 'async-std', 'tungstenite', 'websocket'
$deps = cargo tree --workspace -e normal --prefix none | ForEach-Object { ($_ -split ' ')[0] } | Sort-Object -Unique
$network = $deps | Where-Object { $forbidden -contains $_ }
if ($network) { throw "Networking crates in the dependency tree (see PRIVACY.md): $($network -join ', ')" }

cargo fmt --all -- --check
if ($LASTEXITCODE -ne 0) { throw 'Formatting differs: run `cargo fmt --all`.' }
cargo lint
if ($LASTEXITCODE -ne 0) { throw 'Clippy found problems.' }
cargo test --workspace
if ($LASTEXITCODE -ne 0) { throw 'Tests failed.' }
Write-Host 'All checks passed.'
