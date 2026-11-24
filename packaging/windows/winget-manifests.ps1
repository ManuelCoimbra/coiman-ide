# Writes the winget manifests of one release (the setup program), for the first submission to
# microsoft/winget-pkgs:
#   winget-manifests.ps1 -Version 0.7.0 -Out winget            (hash from the GitHub release)
#   winget-manifests.ps1 -Version 0.7.0 -Setup dist\...setup.exe -Out winget
# Check them with `winget validate --manifest winget`, then submit them with
# `wingetcreate submit winget` (see docs/PUBLISHING.md). Later releases are sent by the
# release workflow with `wingetcreate update`, which needs only the new version and address.
param(
    [Parameter(Mandatory)][string]$Version,
    [string]$Setup,
    [Parameter(Mandatory)][string]$Out
)
$ErrorActionPreference = 'Stop'
$id = 'ManuelCoimbra.CoimanIDE'
$repo = 'https://github.com/ManuelCoimbra/coiman-ide'
$file = "coiman-ide-$Version-windows-x64-setup.exe"
if ($Setup) {
    $hash = (Get-FileHash $Setup -Algorithm SHA256).Hash
} else {
    # GitHub keeps each file's SHA-256: nothing is downloaded.
    $digest = gh api "repos/ManuelCoimbra/coiman-ide/releases/tags/v$Version" --jq ".assets[] | select(.name == `"$file`") | .digest"
    if (-not $digest) { throw "release v$Version has no $file" }
    $hash = $digest -replace '^sha256:', ''
}
$hash = $hash.ToUpper()
New-Item -ItemType Directory -Force $Out | Out-Null

function Write-Manifest([string]$Name, [string]$Text) {
    $path = Join-Path (Resolve-Path $Out) $Name
    [IO.File]::WriteAllText($path, $Text.Replace("`r`n", "`n"), [Text.UTF8Encoding]::new($false))
}

Write-Manifest "$id.yaml" @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.version.1.6.0.schema.json
PackageIdentifier: $id
PackageVersion: $Version
DefaultLocale: en-US
ManifestType: version
ManifestVersion: 1.6.0
"@

# The setup installs for the current user unless asked otherwise; winget asks for the scope
# it wants. The product code is Inno Setup's: the AppId in coiman-ide.iss plus "_is1".
Write-Manifest "$id.installer.yaml" @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.installer.1.6.0.schema.json
PackageIdentifier: $id
PackageVersion: $Version
InstallerType: inno
UpgradeBehavior: install
ProductCode: '{173E221E-EB7A-4B40-8017-BF2081138B47}_is1'
Installers:
- Architecture: x64
  Scope: user
  InstallerUrl: $repo/releases/download/v$Version/$file
  InstallerSha256: $hash
  InstallerSwitches:
    Custom: /CURRENTUSER
- Architecture: x64
  Scope: machine
  InstallerUrl: $repo/releases/download/v$Version/$file
  InstallerSha256: $hash
  InstallerSwitches:
    Custom: /ALLUSERS
ManifestType: installer
ManifestVersion: 1.6.0
"@

Write-Manifest "$id.locale.en-US.yaml" @"
# yaml-language-server: `$schema=https://aka.ms/winget-manifest.defaultLocale.1.6.0.schema.json
PackageIdentifier: $id
PackageVersion: $Version
PackageLocale: en-US
Publisher: Manuel Pereira
PublisherUrl: https://github.com/ManuelCoimbra
PublisherSupportUrl: $repo/issues
PackageName: Coiman IDE
PackageUrl: $repo
License: MIT
LicenseUrl: $repo/blob/main/LICENSE
ShortDescription: Code editor and IDE written in Rust
Description: Coiman IDE edits, runs, tests and debugs code, and works with Git. It has no network connections of its own.
Moniker: coiman-ide
Tags:
- editor
- ide
- rust
- code-editor
ReleaseNotesUrl: $repo/releases/tag/v$Version
ManifestType: defaultLocale
ManifestVersion: 1.6.0
"@
Write-Host "made winget manifests for $Version in $Out (sha256 $hash)"
