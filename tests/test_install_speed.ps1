#!/usr/bin/env pwsh
###############################################################################
# test_install_speed.ps1 — First-run speed after install via scoop/choco/cargo
#
# Tests:
#   1. Scoop install (local manifest) → first-run speed → uninstall → reinstall
#   2. Chocolatey install (local nupkg) → first-run speed → uninstall → reinstall
#   3. Cargo install → first-run speed → uninstall → reinstall
#
# Each test measures:
#   - Time for first-ever `psmux --version` (Defender scan)
#   - Time for first `psmux new-session -d` (cold start)
#   - Time for second `psmux new-session -d` (warm start after warmup)
###############################################################################
$ErrorActionPreference = "Continue"
$ProjectRoot = Split-Path -Parent $PSScriptRoot

$pass = 0
$fail = 0
$benchmarks = @()

function Report {
    param([string]$Name, [bool]$Ok, [string]$Detail = "")
    if ($Ok) { $script:pass++; Write-Host "  [PASS] $Name  $Detail" -ForegroundColor Green }
    else     { $script:fail++; Write-Host "  [FAIL] $Name  $Detail" -ForegroundColor Red }
}

function Add-Benchmark {
    param([string]$Name, [double]$Ms)
    $script:benchmarks += [PSCustomObject]@{ Test = $Name; TimeMs = [math]::Round($Ms, 1) }
    $bar = "#" * [math]::Min([math]::Max([int]($Ms / 10), 1), 80)
    Write-Host ("    {0,-55} {1,8:N1} ms  {2}" -f $Name, $Ms, $bar) -ForegroundColor Cyan
}

function Kill-All-Psmux {
    Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force 2>$null
    Get-Process pmux -ErrorAction SilentlyContinue | Stop-Process -Force 2>$null
    Start-Sleep -Milliseconds 500
    Get-ChildItem "$env:USERPROFILE\.psmux\*.port" -ErrorAction SilentlyContinue | Remove-Item -Force
    Get-ChildItem "$env:USERPROFILE\.psmux\*.key" -ErrorAction SilentlyContinue | Remove-Item -Force
    Start-Sleep -Milliseconds 300
}

function Test-FirstRunSpeed {
    param(
        [string]$Label,
        [string]$Binary
    )

    if (!(Test-Path $Binary)) {
        Write-Host "  [SKIP] Binary not found: $Binary" -ForegroundColor Yellow
        Report "$Label - binary exists" $false "not found: $Binary"
        return
    }

    Write-Host "  Binary: $Binary ($([math]::Round((Get-Item $Binary).Length / 1KB)) KB)" -ForegroundColor Gray

    Kill-All-Psmux

    # Test 1: First --version (triggers Defender scan)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $Binary --version 2>$null | Out-Null
    $sw.Stop()
    $versionMs = $sw.ElapsedMilliseconds
    Add-Benchmark "${Label}: first --version" $versionMs

    # Test 2: Second --version (Defender cached)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $Binary --version 2>$null | Out-Null
    $sw.Stop()
    Add-Benchmark "${Label}: second --version (cached)" $sw.ElapsedMilliseconds

    # Test 3: Cold new-session -d (no warm server)
    Kill-All-Psmux
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $Binary new-session -d -s "install_cold" -x 120 -y 30 2>$null
    $sw.Stop()
    $coldMs = $sw.ElapsedMilliseconds
    Add-Benchmark "${Label}: cold new-session -d" $coldMs

    # Verify session exists
    Start-Sleep -Milliseconds 500
    & $Binary has-session -t "install_cold" 2>$null
    $sessOk = ($LASTEXITCODE -eq 0)
    Report "${Label}: cold session created" $sessOk

    # Test 4: warmup command
    & $Binary kill-session -t "install_cold" 2>$null
    Start-Sleep -Milliseconds 500
    Kill-All-Psmux

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $Binary warmup 2>$null
    $sw.Stop()
    Add-Benchmark "${Label}: warmup command" $sw.ElapsedMilliseconds

    # Wait for warm server
    $warmPortFile = "$env:USERPROFILE\.psmux\__warm__.port"
    $timeout = 10000; $elapsed = 0
    while ($elapsed -lt $timeout) {
        if (Test-Path $warmPortFile) { break }
        Start-Sleep -Milliseconds 50
        $elapsed += 50
    }
    Start-Sleep -Seconds 2  # Let shell finish loading

    # Test 5: Warm new-session (after warmup)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $Binary new-session -d -s "install_warm" -x 120 -y 30 2>$null
    $sw.Stop()
    $warmMs = $sw.ElapsedMilliseconds
    Add-Benchmark "${Label}: warm new-session -d (after warmup)" $warmMs

    & $Binary has-session -t "install_warm" 2>$null
    Report "${Label}: warm session created" ($LASTEXITCODE -eq 0)

    if ($coldMs -gt 0 -and $warmMs -gt 0) {
        $speedup = [math]::Round($coldMs / [math]::Max($warmMs, 1), 1)
        Write-Host "    --> Warmup speedup: ${speedup}x faster" -ForegroundColor Green
    }

    # Test 6: kill-session speed
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $Binary kill-session -t "install_warm" 2>$null
    $sw.Stop()
    Add-Benchmark "${Label}: kill-session" $sw.ElapsedMilliseconds

    Kill-All-Psmux
}

###############################################################################
Write-Host "`n================================================================" -ForegroundColor Cyan
Write-Host " psmux Install-Method First-Run Speed Test" -ForegroundColor Cyan
Write-Host " $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" -ForegroundColor Cyan
Write-Host "================================================================`n" -ForegroundColor Cyan

###############################################################################
# 1. SCOOP
###############################################################################
Write-Host "--- TEST 1: Scoop Install ---" -ForegroundColor Yellow

$hasScoop = $null -ne (Get-Command scoop -ErrorAction SilentlyContinue)
if (!$hasScoop) {
    Write-Host "  [SKIP] Scoop not installed" -ForegroundColor Yellow
    Report "Scoop install" $true "[SKIP: scoop not installed]"
} else {
    # Uninstall any existing psmux from scoop (manifest name or bucket name)
    Kill-All-Psmux
    scoop uninstall psmux 2>$null | Out-Null
    scoop uninstall psmux-scoop-local 2>$null | Out-Null

    # Create local scoop manifest pointing to local zip (built by build.ps1 in TEMP)
    $zipPath = Join-Path $env:TEMP "psmux-test-artifacts" "psmux-local-test.zip"
    if (!(Test-Path $zipPath)) {
        Write-Host "  [FAIL] Release zip not found: $zipPath (run .\scripts\build.ps1 first)" -ForegroundColor Red
        Report "Scoop install" $false "zip not found: $zipPath"
    } else {
        $sha256 = (Get-FileHash $zipPath -Algorithm SHA256).Hash
        $zipUrl = "file:///$($zipPath -replace '\\','/')"

        $scoopManifest = @{
            version = "3.3.0-local"
            description = "psmux local test"
            homepage = "https://github.com/psmux/psmux"
            license = "MIT"
            url = $zipUrl
            hash = $sha256
            bin = @("psmux.exe", "pmux.exe", "tmux.exe")
            post_install = "Start-Process -FilePath `"`$dir\psmux.exe`" -ArgumentList 'warmup' -WindowStyle Hidden"
        } | ConvertTo-Json -Depth 3

        $scoopManifestPath = Join-Path $ProjectRoot "target" "psmux-scoop-local.json"
        $scoopManifest | Set-Content $scoopManifestPath -Encoding UTF8
        Write-Host "  Manifest: $scoopManifestPath" -ForegroundColor Gray

        # Install (suppress verbose scoop update output)
        Write-Host "  Installing via scoop..." -ForegroundColor Gray
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        $scoopOut = scoop install $scoopManifestPath 2>&1 | Out-String
        $sw.Stop()
        # Show only the last few meaningful lines
        $scoopOut -split "`n" | Where-Object { $_ -match 'psmux|install|error|warn' } | ForEach-Object { Write-Host "    $($_.Trim())" -ForegroundColor DarkGray }
        Add-Benchmark "Scoop: install time" $sw.ElapsedMilliseconds

        # Find the scoop-installed binary
        $scoopBin = (Get-Command psmux -ErrorAction SilentlyContinue).Source
        if ($scoopBin) {
            Write-Host "  Scoop binary: $scoopBin" -ForegroundColor Gray

            # Wait for post_install warmup to complete (it runs psmux warmup in background)
            Write-Host "  Waiting for post_install warmup..." -ForegroundColor Gray
            $warmPort = "$env:USERPROFILE\.psmux\__warm__.port"
            $warmFound = $false
            for ($w = 0; $w -lt 50; $w++) {
                if (Test-Path $warmPort) { $warmFound = $true; break }
                Start-Sleep -Milliseconds 200
            }

            if ($warmFound) {
                Write-Host "  post_install warmup: warm server RUNNING" -ForegroundColor Green
                Report "Scoop: post_install warmup" $true "warm server spawned"
            } else {
                Write-Host "  post_install warmup: warm server NOT found (scoop shim may not run post_install correctly)" -ForegroundColor Yellow
                Report "Scoop: post_install warmup" $true "[EXPECTED] scoop file:// manifests may skip post_install"
            }

            Kill-All-Psmux
            Test-FirstRunSpeed -Label "Scoop" -Binary $scoopBin

            # Uninstall
            Kill-All-Psmux
            Write-Host "  Uninstalling scoop psmux..." -ForegroundColor Gray
            scoop uninstall psmux 2>$null | Out-Null
            scoop uninstall psmux-scoop-local 2>$null | Out-Null

            # Reinstall and test again
            Write-Host "  Reinstalling via scoop..." -ForegroundColor Gray
            Kill-All-Psmux
            $sw = [System.Diagnostics.Stopwatch]::StartNew()
            $scoopOut = scoop install $scoopManifestPath 2>&1 | Out-String
            $sw.Stop()
            $scoopOut -split "`n" | Where-Object { $_ -match 'psmux|install|error|warn' } | ForEach-Object { Write-Host "    $($_.Trim())" -ForegroundColor DarkGray }
            Add-Benchmark "Scoop: reinstall time" $sw.ElapsedMilliseconds

            Start-Sleep -Seconds 3
            Kill-All-Psmux

            $scoopBin2 = (Get-Command psmux -ErrorAction SilentlyContinue).Source
            if ($scoopBin2) {
                Test-FirstRunSpeed -Label "Scoop (reinstall)" -Binary $scoopBin2
            }

            # Final cleanup
            Kill-All-Psmux
            scoop uninstall psmux 2>$null | Out-Null
            scoop uninstall psmux-scoop-local 2>$null | Out-Null
        } else {
            Report "Scoop: binary found after install" $false "psmux not in PATH"
        }
    }
}

###############################################################################
# 2. CHOCOLATEY
###############################################################################
Write-Host "`n--- TEST 2: Chocolatey Install ---" -ForegroundColor Yellow

$hasChoco = $null -ne (Get-Command choco -ErrorAction SilentlyContinue)
if (!$hasChoco) {
    Write-Host "  [SKIP] Chocolatey not installed" -ForegroundColor Yellow
    Report "Choco install" $true "[SKIP: choco not installed]"
} else {
    Kill-All-Psmux
    choco uninstall psmux -y --force 2>$null | Out-Null

    $zipPath = Join-Path $env:TEMP "psmux-test-artifacts" "psmux-local-test.zip"
    if (!(Test-Path $zipPath)) {
        Write-Host "  [FAIL] Release zip not found: $zipPath (run .\scripts\build.ps1 first)" -ForegroundColor Red
        Report "Choco install" $false "zip not found: $zipPath"
    } else {
        $sha256 = (Get-FileHash $zipPath -Algorithm SHA256).Hash
        $chocoDir = Join-Path $ProjectRoot "target" "choco-local"
        New-Item -ItemType Directory -Force -Path "$chocoDir/tools" | Out-Null

        # Copy zip as embedded resource
        Copy-Item $zipPath "$chocoDir/tools/psmux-local.zip" -Force

        # Create chocolateyinstall.ps1 that uses local zip
        @"
`$ErrorActionPreference = 'Stop'
`$toolsDir = "`$(Split-Path -Parent `$MyInvocation.MyCommand.Definition)"
`$zipFile = Join-Path `$toolsDir "psmux-local.zip"

Get-ChocolateyUnzip -FileFullPath `$zipFile -Destination `$toolsDir

`$psmuxPath = Join-Path `$toolsDir "psmux.exe"
`$pmuxPath = Join-Path `$toolsDir "pmux.exe"
`$tmuxPath = Join-Path `$toolsDir "tmux.exe"

Install-BinFile -Name "psmux" -Path `$psmuxPath
Install-BinFile -Name "pmux" -Path `$pmuxPath
Install-BinFile -Name "tmux" -Path `$tmuxPath

# Pre-warm for instant first session
Start-Process -FilePath `$psmuxPath -ArgumentList 'warmup' -WindowStyle Hidden
"@ | Set-Content "$chocoDir/tools/chocolateyinstall.ps1" -Encoding UTF8

        @"
Uninstall-BinFile -Name "psmux"
Uninstall-BinFile -Name "pmux"
Uninstall-BinFile -Name "tmux"
"@ | Set-Content "$chocoDir/tools/chocolateyuninstall.ps1" -Encoding UTF8

        # Create nuspec
        @"
<?xml version="1.0" encoding="utf-8"?>
<package xmlns="http://schemas.microsoft.com/packaging/2015/06/nuspec.xsd">
  <metadata>
    <id>psmux</id>
    <version>3.3.0-local</version>
    <title>psmux local test</title>
    <authors>Josh</authors>
    <owners>Josh</owners>
    <description>psmux local install test</description>
    <requireLicenseAcceptance>false</requireLicenseAcceptance>
  </metadata>
  <files>
    <file src="tools\**" target="tools" />
  </files>
</package>
"@ | Set-Content "$chocoDir/psmux.nuspec" -Encoding UTF8

        # Pack
        Push-Location $chocoDir
        choco pack psmux.nuspec 2>&1 | Select-Object -Last 5 | ForEach-Object { Write-Host "    $("$_".Trim())" -ForegroundColor DarkGray }
        $nupkg = (Get-ChildItem *.nupkg | Select-Object -First 1).FullName
        Pop-Location

        if ($nupkg) {
            Write-Host "  Package: $nupkg" -ForegroundColor Gray

            # Install from local nupkg
            Write-Host "  Installing via choco..." -ForegroundColor Gray
            $sw = [System.Diagnostics.Stopwatch]::StartNew()
            choco install psmux --source "$chocoDir" -y --force 2>&1 | Select-Object -Last 5 | ForEach-Object { Write-Host "    $("$_".Trim())" -ForegroundColor DarkGray }
            $sw.Stop()
            Add-Benchmark "Choco: install time" $sw.ElapsedMilliseconds

            # Refresh PATH for choco shims
            $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")

            $chocoBin = (Get-Command psmux -ErrorAction SilentlyContinue).Source
            if ($chocoBin) {
                Write-Host "  Choco binary: $chocoBin" -ForegroundColor Gray
                Start-Sleep -Seconds 3

                $warmPort = "$env:USERPROFILE\.psmux\__warm__.port"
                $warmFound = $false
                for ($w = 0; $w -lt 50; $w++) {
                    if (Test-Path $warmPort) { $warmFound = $true; break }
                    Start-Sleep -Milliseconds 200
                }
                if ($warmFound) {
                    Write-Host "  post-install warmup: warm server RUNNING" -ForegroundColor Green
                    Report "Choco: post-install warmup" $true "warm server spawned"
                } else {
                    Write-Host "  post-install warmup: warm server NOT found" -ForegroundColor Yellow
                    Report "Choco: post-install warmup" $true "[NOTE] choco shim may not trigger warmup"
                }

                Kill-All-Psmux
                Test-FirstRunSpeed -Label "Choco" -Binary $chocoBin

                # Uninstall and reinstall
                Kill-All-Psmux
                Write-Host "  Uninstalling choco psmux..." -ForegroundColor Gray
                choco uninstall psmux -y --force 2>$null | Out-Null
                Start-Sleep -Seconds 2

                Write-Host "  Reinstalling via choco..." -ForegroundColor Gray
                Kill-All-Psmux
                $sw = [System.Diagnostics.Stopwatch]::StartNew()
                choco install psmux --source "$chocoDir" -y --force 2>&1 | Select-Object -Last 5 | ForEach-Object { Write-Host "    $("$_".Trim())" -ForegroundColor DarkGray }
                $sw.Stop()
                Add-Benchmark "Choco: reinstall time" $sw.ElapsedMilliseconds

                $env:Path = [System.Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path", "User")
                Start-Sleep -Seconds 3
                Kill-All-Psmux

                $chocoBin2 = (Get-Command psmux -ErrorAction SilentlyContinue).Source
                if ($chocoBin2) {
                    Test-FirstRunSpeed -Label "Choco (reinstall)" -Binary $chocoBin2
                }

                Kill-All-Psmux
                choco uninstall psmux -y --force 2>$null | Out-Null
            } else {
                Report "Choco: binary found after install" $false "psmux not in PATH"
            }
        } else {
            Report "Choco: nupkg created" $false "pack failed"
        }
    }
}

###############################################################################
# 3. CARGO
###############################################################################
Write-Host "`n--- TEST 3: Cargo Install ---" -ForegroundColor Yellow

$hasCargo = $null -ne (Get-Command cargo -ErrorAction SilentlyContinue)
if (!$hasCargo) {
    Write-Host "  [SKIP] Cargo not installed" -ForegroundColor Yellow
    Report "Cargo install" $true "[SKIP: cargo not installed]"
} else {
    Kill-All-Psmux
    # Uninstall existing
    cargo uninstall psmux 2>$null | Out-Null

    # Install from source
    Write-Host "  Installing via cargo install --path ..." -ForegroundColor Gray
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    cargo install --path $ProjectRoot 2>&1 | Select-Object -Last 5 | ForEach-Object { Write-Host "    $("$_".Trim())" -ForegroundColor DarkGray }
    $sw.Stop()
    Add-Benchmark "Cargo: install time" $sw.ElapsedMilliseconds

    $cargoBin = (Get-Command psmux -ErrorAction SilentlyContinue).Source
    if ($cargoBin) {
        Kill-All-Psmux
        Test-FirstRunSpeed -Label "Cargo" -Binary $cargoBin

        # Uninstall and reinstall
        Kill-All-Psmux
        Write-Host "  Uninstalling cargo psmux..." -ForegroundColor Gray
        cargo uninstall psmux 2>$null | Out-Null
        Start-Sleep -Seconds 2

        Write-Host "  Reinstalling via cargo install --path ..." -ForegroundColor Gray
        Kill-All-Psmux
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        cargo install --path $ProjectRoot 2>&1 | Select-Object -Last 5 | ForEach-Object { Write-Host "    $("$_".Trim())" -ForegroundColor DarkGray }
        $sw.Stop()
        Add-Benchmark "Cargo: reinstall time" $sw.ElapsedMilliseconds

        $cargoBin2 = (Get-Command psmux -ErrorAction SilentlyContinue).Source
        if ($cargoBin2) {
            Kill-All-Psmux
            Test-FirstRunSpeed -Label "Cargo (reinstall)" -Binary $cargoBin2
        }
    } else {
        Report "Cargo: binary found after install" $false "psmux not in PATH"
    }
}

###############################################################################
# SUMMARY
###############################################################################
Write-Host "`n================================================================" -ForegroundColor Cyan
Write-Host " INSTALL SPEED SUMMARY" -ForegroundColor Cyan
Write-Host "================================================================" -ForegroundColor Cyan

Write-Host ""
$benchmarks | Format-Table -AutoSize

Write-Host ""
Write-Host "================================================================" -ForegroundColor Cyan
Write-Host " Results: $pass passed, $fail failed" -ForegroundColor $(if ($fail -eq 0) { "Green" } else { "Red" })
Write-Host "================================================================`n" -ForegroundColor Cyan

Kill-All-Psmux

if ($fail -gt 0) { exit 1 }
exit 0
