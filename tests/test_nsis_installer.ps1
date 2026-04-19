# psmux NSIS Installer Test Script
# Tests that the NSIS installer builds correctly and the resulting .exe works.
#
# Prerequisites:
#   - NSIS installed (makensis on PATH or at default location)
#   - psmux already built (cargo build --release)
#
# Usage:
#   .\tests\test_nsis_installer.ps1
#   .\tests\test_nsis_installer.ps1 -SkipBuild    # skip cargo build
#   .\tests\test_nsis_installer.ps1 -SkipInstall   # only test NSIS compilation

param(
    [switch]$SkipBuild,
    [switch]$SkipInstall
)

$ErrorActionPreference = "Stop"
$script:Passed = 0
$script:Failed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:Passed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:Failed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$root = Split-Path -Parent $PSScriptRoot
$nsiScript = Join-Path $root "installer\psmux.nsi"
$releaseDir = Join-Path $root "target\x86_64-pc-windows-msvc\release"
if (-not (Test-Path "$releaseDir\psmux.exe")) {
    $releaseDir = Join-Path $root "target\release"
}
$installerDir = Join-Path $root "target\installer"

# Detect NSIS
$makensis = $null
foreach ($path in @(
    "makensis",
    "C:\Program Files (x86)\NSIS\makensis.exe",
    "C:\Program Files\NSIS\makensis.exe"
)) {
    if (Get-Command $path -ErrorAction SilentlyContinue) {
        $makensis = (Get-Command $path).Source
        break
    }
    if (Test-Path $path) {
        $makensis = $path
        break
    }
}

Write-Host "=" * 60
Write-Host "PSMUX NSIS INSTALLER TEST"
Write-Host "=" * 60
Write-Host ""

# ── Test 1: NSIS script exists ──────────────────────────────────────
Write-Test "NSIS script exists"
if (Test-Path $nsiScript) {
    Write-Pass "Found $nsiScript"
} else {
    Write-Fail "NSIS script not found at $nsiScript"
    exit 1
}

# ── Test 2: NSIS script has required sections ───────────────────────
Write-Test "NSIS script has required sections"
$nsiContent = Get-Content $nsiScript -Raw
$requiredPatterns = @(
    @{ Name = "KillPsmuxServers macro"; Pattern = "macro KillPsmuxServers" },
    @{ Name = "Install section"; Pattern = 'Section "Install"' },
    @{ Name = "Uninstall section"; Pattern = 'Section "Uninstall"' },
    @{ Name = "kill-server call"; Pattern = "psmux.exe.*kill-server" },
    @{ Name = "taskkill force-kill"; Pattern = "taskkill /F /IM psmux.exe" },
    @{ Name = "EnVar PATH add"; Pattern = "EnVar::AddValue.*Path" },
    @{ Name = "EnVar PATH remove"; Pattern = "EnVar::DeleteValue.*Path" },
    @{ Name = "Uninstaller creation"; Pattern = "WriteUninstaller" },
    @{ Name = "Registry uninstall key"; Pattern = "CurrentVersion\\Uninstall\\psmux" },
    @{ Name = "LZMA compression"; Pattern = "SetCompressor.*lzma" },
    @{ Name = "User-level execution"; Pattern = "RequestExecutionLevel user" },
    @{ Name = "psmux.exe install"; Pattern = 'File.*psmux\.exe' },
    @{ Name = "pmux.exe install"; Pattern = 'File.*pmux\.exe' },
    @{ Name = "tmux.exe install"; Pattern = 'File.*tmux\.exe' },
    @{ Name = "WM_WININICHANGE broadcast"; Pattern = "WM_WININICHANGE" }
)
$allFound = $true
foreach ($req in $requiredPatterns) {
    if ($nsiContent -notmatch $req.Pattern) {
        Write-Fail "Missing: $($req.Name) (pattern: $($req.Pattern))"
        $allFound = $false
    }
}
if ($allFound) {
    Write-Pass "All $($requiredPatterns.Count) required patterns found"
}

# ── Test 3: makensis is available ───────────────────────────────────
Write-Test "NSIS compiler (makensis) available"
if ($makensis) {
    Write-Pass "Found makensis at: $makensis"
} else {
    Write-Fail "makensis not found — install NSIS to test compilation"
    Write-Info "Download from: https://nsis.sourceforge.io/Download"
    if (-not $SkipInstall) {
        Write-Info "Remaining tests require makensis. Exiting."
        Write-Host ""
        Write-Host "Results: $($script:Passed) passed, $($script:Failed) failed"
        exit $(if ($script:Failed -gt 0) { 1 } else { 0 })
    }
}

# ── Test 4: Build release binaries (unless skipped) ─────────────────
if (-not $SkipBuild) {
    Write-Test "Building release binaries..."
    Push-Location $root
    try {
        & cargo build --release --target x86_64-pc-windows-msvc 2>&1 | Out-Null
        if ($LASTEXITCODE -ne 0) {
            Write-Fail "cargo build failed"
            Pop-Location
            exit 1
        }
        Write-Pass "Release build succeeded"
    } finally {
        Pop-Location
    }
} else {
    Write-Info "Skipping cargo build (-SkipBuild)"
}

# ── Test 5: Release binaries exist ──────────────────────────────────
Write-Test "Release binaries exist"
$binaries = @("psmux.exe", "pmux.exe", "tmux.exe")
$allExist = $true
foreach ($bin in $binaries) {
    $path = Join-Path $releaseDir $bin
    if (-not (Test-Path $path)) {
        Write-Fail "Missing binary: $path"
        $allExist = $false
    }
}
if ($allExist) {
    Write-Pass "All binaries found in $releaseDir"
} else {
    Write-Fail "Some binaries missing — cannot build installer"
    exit 1
}

# ── Test 6: Compile NSIS installer ──────────────────────────────────
if (-not $makensis) {
    Write-Info "Skipping compilation (no makensis)"
} else {
    Write-Test "Compiling NSIS installer..."

    # Read version from Cargo.toml
    $cargoToml = Get-Content (Join-Path $root "Cargo.toml") -Raw
    if ($cargoToml -match 'version\s*=\s*"([^"]+)"') {
        $version = $Matches[1]
    } else {
        $version = "0.0.0-test"
    }

    # Ensure output directory exists
    New-Item -ItemType Directory -Force -Path $installerDir | Out-Null

    & $makensis /NOCD /DVERSION=$version /DARCH=x64 "/DSOURCE_DIR=$releaseDir" "/DREPO_DIR=$root" $nsiScript
    if ($LASTEXITCODE -ne 0) {
        Write-Fail "makensis compilation failed (exit code: $LASTEXITCODE)"
    } else {
        Write-Pass "NSIS compilation succeeded"

        # ── Test 7: Installer file was created ───────────────────────
        $installerExe = Join-Path $installerDir "psmux-v${version}-x64-setup.exe"
        Write-Test "Installer file created"
        if (Test-Path $installerExe) {
            $size = (Get-Item $installerExe).Length
            $sizeMB = [math]::Round($size / 1MB, 2)
            Write-Pass "Installer created: $installerExe ($sizeMB MB)"
        } else {
            Write-Fail "Installer not found at: $installerExe"
        }

        # ── Test 8: Installer is signed as NSIS (has NSIS marker) ────
        Write-Test "Installer is valid PE executable"
        if (Test-Path $installerExe) {
            $bytes = [System.IO.File]::ReadAllBytes($installerExe)
            if ($bytes.Length -ge 2 -and $bytes[0] -eq 0x4D -and $bytes[1] -eq 0x5A) {
                Write-Pass "Valid PE executable (MZ header)"
            } else {
                Write-Fail "Not a valid PE executable"
            }
        }

        # ── Test 9: Installer supports /S silent switch ─────────────
        if (-not $SkipInstall -and (Test-Path $installerExe)) {
            Write-Test "Silent install /S /D=<tmpdir>"
            $testDir = Join-Path $env:TEMP "psmux-installer-test-$(Get-Random)"
            try {
                # Run installer silently to a temp directory
                $proc = Start-Process -FilePath $installerExe -ArgumentList "/S", "/D=$testDir" -Wait -PassThru -NoNewWindow
                if ($proc.ExitCode -eq 0) {
                    Write-Pass "Silent install completed (exit code 0)"
                } else {
                    Write-Fail "Silent install exited with code: $($proc.ExitCode)"
                }

                # ── Test 10: Installed files exist ─────────────────────
                Write-Test "Installed files present"
                $installedOk = $true
                foreach ($bin in @("psmux.exe", "pmux.exe", "tmux.exe", "uninstall.exe")) {
                    $f = Join-Path $testDir $bin
                    if (-not (Test-Path $f)) {
                        Write-Fail "Missing installed file: $bin"
                        $installedOk = $false
                    }
                }
                if ($installedOk) {
                    Write-Pass "All expected files installed"
                }

                # ── Test 11: psmux --version works from install dir ────
                Write-Test "psmux --version from install dir"
                $psmux = Join-Path $testDir "psmux.exe"
                if (Test-Path $psmux) {
                    $vout = & $psmux --version 2>&1 | Out-String
                    if ($vout -match "psmux|$version") {
                        Write-Pass "psmux --version: $($vout.Trim())"
                    } else {
                        Write-Fail "Unexpected version output: $vout"
                    }
                }

                # ── Test 12: Registry keys written ─────────────────────
                Write-Test "Registry uninstall key exists"
                $regPath = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\psmux"
                if (Test-Path $regPath) {
                    $displayVer = (Get-ItemProperty $regPath -ErrorAction SilentlyContinue).DisplayVersion
                    if ($displayVer -eq $version) {
                        Write-Pass "Registry key present, version = $displayVer"
                    } else {
                        Write-Fail "Registry version mismatch: expected $version, got $displayVer"
                    }
                } else {
                    Write-Fail "Registry key not found at $regPath"
                }

                # ── Test 13: Silent uninstall ──────────────────────────
                Write-Test "Silent uninstall"
                $uninstaller = Join-Path $testDir "uninstall.exe"
                if (Test-Path $uninstaller) {
                    $proc = Start-Process -FilePath $uninstaller -ArgumentList "/S" -Wait -PassThru -NoNewWindow
                    Start-Sleep -Seconds 2
                    if ($proc.ExitCode -eq 0) {
                        Write-Pass "Silent uninstall completed (exit code 0)"
                    } else {
                        Write-Fail "Silent uninstall exited with code: $($proc.ExitCode)"
                    }

                    # ── Test 14: Files removed after uninstall ─────────
                    Write-Test "Files removed after uninstall"
                    $cleanedUp = $true
                    foreach ($bin in @("psmux.exe", "pmux.exe", "tmux.exe")) {
                        if (Test-Path (Join-Path $testDir $bin)) {
                            Write-Fail "File still exists after uninstall: $bin"
                            $cleanedUp = $false
                        }
                    }
                    if ($cleanedUp) {
                        Write-Pass "All binaries removed by uninstaller"
                    }

                    # ── Test 15: Registry cleaned up ───────────────────
                    Write-Test "Registry cleaned after uninstall"
                    if (-not (Test-Path $regPath)) {
                        Write-Pass "Registry uninstall key removed"
                    } else {
                        Write-Fail "Registry key still present after uninstall"
                    }
                }
            } finally {
                # Cleanup temp dir if it still exists
                if (Test-Path $testDir) {
                    Start-Process -FilePath "cmd.exe" -ArgumentList "/c", "rd", "/s", "/q", $testDir -Wait -NoNewWindow -ErrorAction SilentlyContinue
                }
            }
        } else {
            Write-Info "Skipping install/uninstall tests (-SkipInstall or no installer)"
        }
    }
}

# ── Summary ──────────────────────────────────────────────────────────
Write-Host ""
Write-Host "=" * 60
Write-Host "RESULTS: $($script:Passed) passed, $($script:Failed) failed" -ForegroundColor $(if ($script:Failed -gt 0) { "Red" } else { "Green" })
Write-Host "=" * 60

exit $(if ($script:Failed -gt 0) { 1 } else { 0 })
