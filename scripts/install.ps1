# psmux installation script for Windows
# Run as: irm https://raw.githubusercontent.com/marlocarlo/psmux/master/scripts/install.ps1 | iex
# Or locally: .\scripts\install.ps1

param(
    [string]$InstallDir = "$env:LOCALAPPDATA\psmux",
    [switch]$Force
)

$ErrorActionPreference = 'Stop'

Write-Host "psmux installer" -ForegroundColor Cyan
Write-Host "===============" -ForegroundColor Cyan

# Determine if we're installing from local build or downloading
$LocalBuild = Test-Path "$PSScriptRoot\..\target\release\psmux.exe"
$RepoRoot = Split-Path -Parent $PSScriptRoot

if ($LocalBuild) {
    Write-Host "Installing from local build..." -ForegroundColor Yellow
    $SourceDir = "$RepoRoot\target\release"
} else {
    Write-Host "Downloading latest release..." -ForegroundColor Yellow
    
    # Get latest release info
    $ReleasesUrl = "https://api.github.com/repos/marlocarlo/psmux/releases/latest"
    try {
        $Release = Invoke-RestMethod -Uri $ReleasesUrl -Headers @{ "User-Agent" = "psmux-installer" }
        $Asset = $Release.assets | Where-Object { $_.name -match "windows.*x64.*zip" } | Select-Object -First 1
        
        if (-not $Asset) {
            throw "No Windows x64 release asset found"
        }
        
        $DownloadUrl = $Asset.browser_download_url
        $TempZip = "$env:TEMP\psmux-download.zip"
        $TempExtract = "$env:TEMP\psmux-extract"
        
        Write-Host "Downloading from: $DownloadUrl"
        Invoke-WebRequest -Uri $DownloadUrl -OutFile $TempZip
        
        # Extract
        if (Test-Path $TempExtract) { Remove-Item -Recurse -Force $TempExtract }
        Expand-Archive -Path $TempZip -DestinationPath $TempExtract -Force
        
        $SourceDir = $TempExtract
        
    } catch {
        Write-Host "Error downloading release: $_" -ForegroundColor Red
        Write-Host "Try installing from a local build instead:" -ForegroundColor Yellow
        Write-Host "  cargo build --release" -ForegroundColor White
        Write-Host "  .\scripts\install.ps1" -ForegroundColor White
        exit 1
    }
}

# Create install directory
if (-not (Test-Path $InstallDir)) {
    Write-Host "Creating install directory: $InstallDir"
    New-Item -ItemType Directory -Path $InstallDir -Force | Out-Null
}

# Copy binaries
$Binaries = @("psmux.exe", "pmux.exe", "tmux.exe")
foreach ($bin in $Binaries) {
    $src = Join-Path $SourceDir $bin
    $dst = Join-Path $InstallDir $bin
    
    if (Test-Path $src) {
        Write-Host "  Installing $bin..." -ForegroundColor Green
        Copy-Item -Path $src -Destination $dst -Force
    } else {
        Write-Host "  Warning: $bin not found" -ForegroundColor Yellow
    }
}

# Add to PATH if not already there
$UserPath = [Environment]::GetEnvironmentVariable("Path", "User")
if ($UserPath -notlike "*$InstallDir*") {
    Write-Host "Adding to PATH..." -ForegroundColor Green
    $NewPath = "$UserPath;$InstallDir"
    [Environment]::SetEnvironmentVariable("Path", $NewPath, "User")
    $env:Path = "$env:Path;$InstallDir"
    Write-Host "  Added $InstallDir to user PATH" -ForegroundColor Green
} else {
    Write-Host "Already in PATH" -ForegroundColor Gray
}

# Cleanup temp files if downloaded
if (-not $LocalBuild) {
    if (Test-Path $TempZip) { Remove-Item $TempZip -Force }
    if (Test-Path $TempExtract) { Remove-Item -Recurse -Force $TempExtract }
}

Write-Host ""
Write-Host "Installation complete!" -ForegroundColor Green
Write-Host ""
Write-Host "You can now use:" -ForegroundColor Cyan
Write-Host "  psmux    - Start/attach to terminal multiplexer"
Write-Host "  pmux     - Alias for psmux"  
Write-Host "  tmux     - tmux-compatible alias"
Write-Host ""
Write-Host "Quick start:" -ForegroundColor Cyan
Write-Host "  psmux                    # Start new session or attach to 'default'"
Write-Host "  psmux new -s mysession   # Create named session"
Write-Host "  psmux ls                 # List sessions"
Write-Host "  psmux attach -t name     # Attach to session"
Write-Host ""
Write-Host "Note: Restart your terminal or run:" -ForegroundColor Yellow
Write-Host '  $env:Path = [Environment]::GetEnvironmentVariable("Path", "User") + ";" + [Environment]::GetEnvironmentVariable("Path", "Machine")'
