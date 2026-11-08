# Prints one version's section of CHANGELOG.md, for a GitHub release's notes.
# Usage: scripts/release-notes.ps1 0.2.0
param([Parameter(Mandatory)][string]$Version)
$ErrorActionPreference = 'Stop'
$changelog = Join-Path (Split-Path $PSScriptRoot -Parent) 'CHANGELOG.md'
$lines = Get-Content $changelog -Encoding utf8
$start = -1
for ($i = 0; $i -lt $lines.Count; $i++) {
    if ($lines[$i] -match "^## \[$([regex]::Escape($Version))\]") { $start = $i + 1; break }
}
if ($start -lt 0) { throw "CHANGELOG.md has no section for $Version" }
$end = $lines.Count
for ($i = $start; $i -lt $lines.Count; $i++) {
    if ($lines[$i] -match '^## \[') { $end = $i; break }
}
($lines[$start..($end - 1)] -join "`n").Trim()
