# Writes the Scoop manifest of one release:
#   scoop-manifest.ps1 -Version 0.7.0 -Zip dist\coiman-ide-0.7.0-windows-x64.zip -Out dist\coiman-ide.json
# Each release attaches it as coiman-ide.json, so this address always gives the newest one:
#   https://github.com/ManuelCoimbra/coiman-ide/releases/latest/download/coiman-ide.json
# Scoop remembers the address a program was installed from, and `scoop update coiman-ide`
# reads it again: that is how Scoop users get new versions. Nothing is committed to the
# repository for it.
param(
    [Parameter(Mandatory)][string]$Version,
    [Parameter(Mandatory)][string]$Zip,
    [Parameter(Mandatory)][string]$Out
)
$ErrorActionPreference = 'Stop'
$repo = 'https://github.com/ManuelCoimbra/coiman-ide'
$hash = (Get-FileHash $Zip -Algorithm SHA256).Hash.ToLower()
$manifest = [ordered]@{
    version      = $Version
    description  = 'Code editor and IDE written in Rust'
    homepage     = $repo
    license      = 'MIT'
    notes        = 'Settings are kept in %APPDATA%\coiman-ide.'
    architecture = [ordered]@{
        '64bit' = [ordered]@{
            url  = "$repo/releases/download/v$Version/coiman-ide-$Version-windows-x64.zip"
            hash = $hash
        }
    }
    bin          = 'coiman-ide.exe'
    shortcuts    = @(, @('coiman-ide.exe', 'Coiman IDE'))
    checkver     = 'github'
    autoupdate   = [ordered]@{
        architecture = [ordered]@{
            '64bit' = [ordered]@{ url = "$repo/releases/download/v`$version/coiman-ide-`$version-windows-x64.zip" }
        }
    }
}
# UTF-8 without a byte order mark, which Scoop's JSON reader expects.
$json = $manifest | ConvertTo-Json -Depth 6
[IO.File]::WriteAllText((Join-Path (Resolve-Path (Split-Path $Out -Parent)) (Split-Path $Out -Leaf)), $json + "`n", [Text.UTF8Encoding]::new($false))
Write-Host "made $Out ($Version, sha256 $hash)"
