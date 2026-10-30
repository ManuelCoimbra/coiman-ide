# Builds the built-in extensions, the example plugin and the test fixture: each is compiled
# for wasm32-unknown-unknown and turned into a component (plugin.wasm next
# to its plugin.toml) with rcplugin.
# Needs the wasm32 target: rustup target add wasm32-unknown-unknown
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
Set-Location $root
cargo build -q -p rcplugin
if ($LASTEXITCODE -ne 0) { throw 'rcplugin failed to build' }
$tool = Join-Path $root 'target\debug\rcplugin.exe'
# One shared build folder, so guests don't each keep their own copy of wit-bindgen.
$env:CARGO_TARGET_DIR = Join-Path $root 'target\plugins'
$dirs = 'extensions\comment-highlights', 'extensions\trailing-spaces', 'extensions\color-preview', 'extensions\scs-sii',
    'examples\plugins\word-count', 'core\tests\fixtures\test-plugin'
foreach ($dir in $dirs) {
    Push-Location (Join-Path $root $dir)
    try {
        cargo build -q --release --target wasm32-unknown-unknown
        if ($LASTEXITCODE -ne 0) { throw "$dir failed to build" }
        $name = (Get-Content Cargo.toml | Select-String '^name = "(.*)"').Matches[0].Groups[1].Value -replace '-', '_'
        & $tool componentize "$env:CARGO_TARGET_DIR\wasm32-unknown-unknown\release\$name.wasm" plugin.wasm
        if ($LASTEXITCODE -ne 0) { throw "$dir failed to componentize" }
    } finally {
        Pop-Location
    }
}
