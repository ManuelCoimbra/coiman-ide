# Frees the disk space rebuilds leave behind. Cargo never deletes old builds: each change to
# this workspace's crates leaves another full copy of them in target/, so the folder grows
# by gigabytes a day of work. This removes the copies of our own crates and the incremental
# caches; dependencies stay built, so the next build only recompiles our crates.
#
#   powershell -File scripts/tidy.ps1        old copies of our crates, incremental caches
#   powershell -File scripts/tidy.ps1 -All   everything (cargo clean): the next build is a full one
#
# The release program (target/release/coiman-ide.exe) is kept, so shortcuts to it still work.
param([switch]$All)
$ErrorActionPreference = 'Stop'
Set-Location (Split-Path $PSScriptRoot -Parent)

function Size-Of([string]$Path) {
    if (-not (Test-Path $Path)) { return 0 }
    (Get-ChildItem $Path -Recurse -File -Force -ErrorAction SilentlyContinue | Measure-Object Length -Sum).Sum
}

$before = Size-Of 'target'
$exe = 'target/release/coiman-ide.exe'
$kept = $null
if (Test-Path $exe) {
    $kept = Join-Path ([IO.Path]::GetTempPath()) "coiman-ide-kept-$PID.exe"
    Copy-Item $exe $kept
}
try {
    if ($All) {
        cargo clean
        if ($LASTEXITCODE -ne 0) { throw 'cargo clean failed' }
    } else {
        foreach ($release in @($false, $true)) {
            $cargoArgs = @('clean', '-p', 'coiman-core', '-p', 'coiman-ide', '-p', 'rcplugin')
            if ($release) { $cargoArgs += '--release' }
            cargo @cargoArgs
            if ($LASTEXITCODE -ne 0) { throw "cargo $($cargoArgs -join ' ') failed" }
        }
        foreach ($dir in 'target/debug/incremental', 'target/release/incremental', 'target/debug/examples', 'target/flycheck0') {
            if (Test-Path $dir) { Remove-Item $dir -Recurse -Force }
        }
    }
} finally {
    if ($kept) {
        New-Item -ItemType Directory -Force (Split-Path $exe) | Out-Null
        Move-Item $kept $exe -Force
    }
}
$after = Size-Of 'target'
'target/ was {0:N1} GB, now {1:N1} GB: {2:N1} GB freed.' -f ($before / 1GB), ($after / 1GB), (($before - $after) / 1GB)
