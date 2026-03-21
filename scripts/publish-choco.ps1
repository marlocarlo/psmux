<#
.SYNOPSIS
    Build and publish the psmux Chocolatey package with the correct SHA256 checksum.

.DESCRIPTION
    This script mirrors what the GitHub Actions release workflow does:
    1. Downloads the release zip from GitHub Releases
    2. Computes the SHA256 checksum
    3. Generates chocolateyinstall.ps1 with the real checksum
    4. Packs the .nupkg
    5. Optionally pushes to Chocolatey

    Use this for local publishing so the checksum is always correct.

.PARAMETER Version
    The version to publish (e.g. "0.3.9"). If omitted, reads from Cargo.toml.

.PARAMETER Push
    If specified, pushes the package to Chocolatey after packing.

.PARAMETER ApiKey
    Chocolatey API key. If not provided and -Push is set, uses $env:CHOCOLATEY_API_KEY.

.EXAMPLE
    # Just pack (dry run) - verify everything looks good
    .\scripts\publish-choco.ps1

    # Pack and push
    .\scripts\publish-choco.ps1 -Push

    # Specific version
    .\scripts\publish-choco.ps1 -Version 0.3.9 -Push
#>
param(
    [string]$Version,
    [switch]$Push,
    [string]$ApiKey
)

$ErrorActionPreference = 'Stop'
$RepoOwner = "psmux"
$RepoName = "psmux"
$PackageId = "psmux"

# --- Resolve version ---
if (-not $Version) {
    $cargoToml = Get-Content "$PSScriptRoot\..\Cargo.toml" -Raw
    if ($cargoToml -match 'version\s*=\s*"([^"]+)"') {
        $Version = $matches[1]
    } else {
        Write-Error "Could not extract version from Cargo.toml. Pass -Version explicitly."
        exit 1
    }
}

$Tag = "v$Version"
Write-Host "=== Publishing psmux $Tag to Chocolatey ===" -ForegroundColor Cyan

# --- Setup temp build directory ---
$buildDir = Join-Path $PSScriptRoot "..\target\choco-build"
if (Test-Path $buildDir) { Remove-Item $buildDir -Recurse -Force }
New-Item -ItemType Directory -Path "$buildDir\tools" -Force | Out-Null

# --- Download release zip ---
$zipUrl = "https://github.com/$RepoOwner/$RepoName/releases/download/$Tag/psmux-$Tag-windows-x64.zip"
$zipFile = Join-Path $buildDir "psmux-release.zip"

Write-Host "Downloading $zipUrl ..." -ForegroundColor Yellow
try {
    Invoke-WebRequest -Uri $zipUrl -OutFile $zipFile -UseBasicParsing -ErrorAction Stop
} catch {
    Write-Error "Failed to download release zip. Make sure the GitHub Release for $Tag exists.`n$_"
    exit 1
}

# --- Compute SHA256 ---
$hash = (Get-FileHash $zipFile -Algorithm SHA256).Hash
Write-Host "SHA256: $hash" -ForegroundColor Green

# --- Verify by re-downloading ---
$verifyFile = Join-Path $buildDir "psmux-verify.zip"
Write-Host "Verifying checksum (re-downloading)..." -ForegroundColor Yellow
Invoke-WebRequest -Uri $zipUrl -OutFile $verifyFile -UseBasicParsing -ErrorAction Stop
$hash2 = (Get-FileHash $verifyFile -Algorithm SHA256).Hash
if ($hash -ne $hash2) {
    Write-Error "Checksum mismatch on re-download! $hash vs $hash2"
    exit 1
}
Write-Host "Checksum verified!" -ForegroundColor Green

# --- Generate chocolateyinstall.ps1 ---
$installScript = @"
`$ErrorActionPreference = 'Stop'

`$toolsDir = "`$(Split-Path -Parent `$MyInvocation.MyCommand.Definition)"
`$url64 = '$zipUrl'

`$packageArgs = @{
  packageName    = `$env:ChocolateyPackageName
  unzipLocation  = `$toolsDir
  url64bit       = `$url64
  checksum64     = '$hash'
  checksumType64 = 'sha256'
}

Install-ChocolateyZipPackage @packageArgs

# Create shims for psmux, pmux, and tmux
`$psmuxPath = Join-Path `$toolsDir "psmux.exe"
`$pmuxPath = Join-Path `$toolsDir "pmux.exe"
`$tmuxPath = Join-Path `$toolsDir "tmux.exe"

Install-BinFile -Name "psmux" -Path `$psmuxPath
Install-BinFile -Name "pmux" -Path `$pmuxPath
Install-BinFile -Name "tmux" -Path `$tmuxPath
"@
Set-Content -Path "$buildDir\tools\chocolateyinstall.ps1" -Value $installScript -NoNewline
Write-Host "Generated chocolateyinstall.ps1" -ForegroundColor Green

# --- Generate chocolateyuninstall.ps1 ---
$uninstallScript = @"
Uninstall-BinFile -Name "psmux"
Uninstall-BinFile -Name "pmux"
Uninstall-BinFile -Name "tmux"
"@
Set-Content -Path "$buildDir\tools\chocolateyuninstall.ps1" -Value $uninstallScript -NoNewline

# --- Generate nuspec ---
$nuspec = @"
<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd">
  <metadata>
    <id>$PackageId</id>
    <version>$Version</version>
    <title>psmux - Terminal Multiplexer for Windows</title>
    <authors>Josh</authors>
    <owners>Josh</owners>
    <licenseUrl>https://github.com/$RepoOwner/$RepoName/blob/master/LICENSE</licenseUrl>
    <projectUrl>https://github.com/$RepoOwner/$RepoName</projectUrl>
    <requireLicenseAcceptance>false</requireLicenseAcceptance>
    <description>Terminal multiplexer for Windows - tmux alternative for PowerShell and Windows Terminal. Includes psmux, pmux, and tmux commands.</description>
    <summary>Terminal multiplexer for Windows (tmux alternative)</summary>
    <releaseNotes>https://github.com/$RepoOwner/$RepoName/releases</releaseNotes>
    <tags>terminal multiplexer tmux powershell cli windows psmux pmux</tags>
    <packageSourceUrl>https://github.com/$RepoOwner/$RepoName</packageSourceUrl>
    <docsUrl>https://github.com/$RepoOwner/$RepoName#readme</docsUrl>
    <bugTrackerUrl>https://github.com/$RepoOwner/$RepoName/issues</bugTrackerUrl>
  </metadata>
  <files>
    <file src="tools\**" target="tools" />
  </files>
</package>
"@
Set-Content -Path "$buildDir\psmux.nuspec" -Value $nuspec -NoNewline
Write-Host "Generated psmux.nuspec (v$Version)" -ForegroundColor Green

# --- Pack ---
Write-Host "`nPacking..." -ForegroundColor Cyan
Push-Location $buildDir
try {
    choco pack psmux.nuspec
    $nupkg = (Get-ChildItem *.nupkg)[0]
    Write-Host "Created: $($nupkg.Name) ($([math]::Round($nupkg.Length/1KB, 1)) KB)" -ForegroundColor Green
} finally {
    Pop-Location
}

# --- Push ---
if ($Push) {
    $key = if ($ApiKey) { $ApiKey } else { $env:CHOCOLATEY_API_KEY }
    if (-not $key) {
        Write-Error "No API key provided. Use -ApiKey or set `$env:CHOCOLATEY_API_KEY"
        exit 1
    }
    Write-Host "`nPushing $($nupkg.Name) to Chocolatey..." -ForegroundColor Cyan
    Push-Location $buildDir
    try {
        choco push $nupkg.Name --source https://push.chocolatey.org/ --api-key $key
        Write-Host "Successfully pushed to Chocolatey!" -ForegroundColor Green
    } finally {
        Pop-Location
    }
} else {
    Write-Host "`nDry run complete. Package at: $($nupkg.FullName)" -ForegroundColor Yellow
    Write-Host "To push: .\scripts\publish-choco.ps1 -Version $Version -Push" -ForegroundColor Yellow
}
