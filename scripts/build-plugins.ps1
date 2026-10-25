# Builds the example plugin and the test fixture plugin: each is compiled
# for wasm32-unknown-unknown and turned into a component (plugin.wasm next
# to its plugin.toml) with rcplugin.
# Needs the wasm32 target: rustup target add wasm32-unknown-unknown
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
cargo build -q -p rcplugin
if ($LASTEXITCODE -ne 0) { throw 'rcplugin failed to build' }
$tool = Join-Path $root 'target\debug\rcplugin.exe'
foreach ($dir in 'examples\plugins\word-count', 'core\tests\fixtures\test-plugin') {
    Push-Location (Join-Path $root $dir)
    try {
        cargo build -q --release --target wasm32-unknown-unknown
        if ($LASTEXITCODE -ne 0) { throw "$dir failed to build" }
        $name = (Get-Content Cargo.toml | Select-String '^name = "(.*)"').Matches[0].Groups[1].Value -replace '-', '_'
        & $tool componentize "target\wasm32-unknown-unknown\release\$name.wasm" plugin.wasm
        if ($LASTEXITCODE -ne 0) { throw "$dir failed to componentize" }
    } finally {
        Pop-Location
    }
}
