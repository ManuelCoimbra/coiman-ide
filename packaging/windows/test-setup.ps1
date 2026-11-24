# Checks the setup program: it installs silently for the current user, a second setup of a
# newer version replaces the first in place, and uninstalling removes the program but keeps
# the settings.
#   test-setup.ps1 -Setup dist\coiman-ide-0.7.0-windows-x64-setup.exe -Version 0.7.0 [-Older <setup> -OlderVersion 0.6.9]
# It installs into a folder of its own under %TEMP%. The Start menu entry it makes is removed
# again by the uninstaller; an entry named "Coiman IDE" that was there before is put back.
param(
    [Parameter(Mandatory)][string]$Setup,
    [Parameter(Mandatory)][string]$Version,
    [string]$Older,
    [string]$OlderVersion
)
$ErrorActionPreference = 'Stop'
$dir = Join-Path ([IO.Path]::GetTempPath()) "coiman-setup-test-$PID"
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{173E221E-EB7A-4B40-8017-BF2081138B47}_is1'
$link = Join-Path ([Environment]::GetFolderPath('Programs')) 'Coiman IDE.lnk'
$saved = $null
if (Test-Path $link) { $saved = "$link.before-test"; Copy-Item $link $saved -Force }
$settings = Join-Path $env:APPDATA 'coiman-ide'
$marker = Join-Path $settings "setup-test-$PID"

function Install-Setup([string]$Path) {
    $p = Start-Process $Path -ArgumentList '/VERYSILENT', '/CURRENTUSER', '/SUPPRESSMSGBOXES', '/NORESTART', "/DIR=$dir" -Wait -PassThru
    if ($p.ExitCode) { throw "$Path exited with $($p.ExitCode)" }
}
function Assert([bool]$Ok, [string]$What) {
    if (-not $Ok) { throw "FAILED: $What" }
    Write-Host "ok: $What"
}

try {
    New-Item -ItemType Directory -Force $settings | Out-Null
    Set-Content $marker 'kept'
    if ($Older) {
        Install-Setup $Older
        Assert ((Get-ItemProperty $key).DisplayVersion -eq $OlderVersion) "older version $OlderVersion installed"
        # A file only the older version had, which the newer one must not leave behind.
        New-Item -ItemType Directory -Force "$dir\extensions\gone-in-newer" | Out-Null
    }
    Install-Setup $Setup
    Assert (Test-Path "$dir\coiman-ide.exe") 'program installed'
    Assert (Test-Path "$dir\extensions") 'extensions installed beside it'
    Assert (Test-Path $link) 'Start menu entry made'
    $entry = Get-ItemProperty $key
    Assert ($entry.DisplayVersion -eq $Version) "Apps list shows version $Version"
    $entries = @(Get-ChildItem 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall' |
        Where-Object { (Get-ItemProperty $_.PSPath).DisplayName -eq 'Coiman IDE' })
    Assert ($entries.Count -eq 1) 'one Coiman IDE in the Apps list'
    if ($Older) { Assert (-not (Test-Path "$dir\extensions\gone-in-newer")) 'older extensions removed on update' }

    $p = Start-Process "$dir\unins000.exe" -ArgumentList '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART' -Wait -PassThru
    # The uninstaller copies itself to %TEMP% and finishes from there: wait for the folder to go.
    for ($i = 0; $i -lt 60 -and (Test-Path "$dir\coiman-ide.exe"); $i++) { Start-Sleep -Milliseconds 500 }
    Assert (-not (Test-Path "$dir\coiman-ide.exe")) 'uninstall removed the program'
    Assert (-not (Test-Path $key)) 'uninstall removed the Apps list entry'
    Assert (Test-Path $marker) 'uninstall kept the settings'
    Write-Host 'setup test passed'
} finally {
    Remove-Item $marker -ErrorAction SilentlyContinue
    if ($saved) { Move-Item $saved $link -Force }
    if (Test-Path $dir) { Remove-Item $dir -Recurse -Force -ErrorAction SilentlyContinue }
}
