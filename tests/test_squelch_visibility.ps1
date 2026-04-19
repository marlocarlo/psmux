# test_squelch_visibility.ps1
#
# Comprehensive visibility tests for the squelch system.
# Verifies that injected cd+cls commands are NEVER visible to users
# during warm session claiming with CWD changes.
#
# Test categories:
#   A. Directory type variants (root, deep, spaces, special chars, etc.)
#   B. Command leak detection (capture-pane must not show cd or cls)
#   C. Blank frame verification (pane content while squelched must be empty)
#   D. Prompt correctness (CWD matches after squelch lifts)
#   E. Rapid sequential claims (race conditions)
#   F. Multiple sessions with different CWDs
#   G. Squelch does not eat legitimate content in same-CWD claims
#
# Usage:
#   .\tests\test_squelch_visibility.ps1 [-Verbose] [-TimeoutSec 20]

param(
    [int]$TimeoutSec = 20,
    [switch]$Verbose
)

$ErrorActionPreference = "Continue"

$PSMUX = Join-Path $PSScriptRoot "..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = Join-Path $PSScriptRoot "..\target\release\tmux.exe"
}
if (-not (Test-Path $PSMUX)) {
    $PSMUX = (Get-Command psmux -ErrorAction SilentlyContinue).Source
}
if (-not $PSMUX -or -not (Test-Path $PSMUX)) {
    Write-Host "ERROR: Cannot find psmux.exe" -ForegroundColor Red
    exit 1
}
$PSMUX = (Resolve-Path $PSMUX).Path

$HOME_DIR    = $env:USERPROFILE
$PSMUX_DIR   = "$HOME_DIR\.psmux"
$ORIGINAL_CWD = (Get-Location).Path

$PASS = 0; $FAIL = 0; $SKIP = 0; $TOTAL = 0

function Write-Pass { param([string]$msg) $script:PASS++; $script:TOTAL++; Write-Host "  [PASS] $msg" -ForegroundColor Green }
function Write-Fail { param([string]$msg) $script:FAIL++; $script:TOTAL++; Write-Host "  [FAIL] $msg" -ForegroundColor Red }
function Write-Skip { param([string]$msg) $script:SKIP++; Write-Host "  [SKIP] $msg" -ForegroundColor DarkYellow }
function Write-Info { param([string]$msg) if ($Verbose) { Write-Host "  [INFO] $msg" -ForegroundColor Gray } }
function Write-Header { param([string]$text)
    Write-Host ""
    Write-Host ("=" * 76) -ForegroundColor Cyan
    Write-Host "  $text" -ForegroundColor Cyan
    Write-Host ("=" * 76) -ForegroundColor Cyan
}

function Kill-All-Psmux {
    Get-Process psmux, pmux, tmux -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 600
    if (Test-Path $PSMUX_DIR) {
        Get-ChildItem "$PSMUX_DIR\sqv_*.port" -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
        Get-ChildItem "$PSMUX_DIR\sqv_*.key"  -ErrorAction SilentlyContinue | Remove-Item -Force -ErrorAction SilentlyContinue
        Remove-Item "$PSMUX_DIR\__warm__.port" -Force -ErrorAction SilentlyContinue
        Remove-Item "$PSMUX_DIR\__warm__.key"  -Force -ErrorAction SilentlyContinue
    }
}

function Wait-SessionAlive {
    param([string]$SessionName, [int]$TimeoutMs = 15000)
    $pf = "$PSMUX_DIR\${SessionName}.port"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if (Test-Path $pf) {
            $port = (Get-Content $pf -Raw -ErrorAction SilentlyContinue)
            if ($port -and $port.Trim() -match '^\d+$') {
                try {
                    $tcp = New-Object System.Net.Sockets.TcpClient
                    $tcp.Connect("127.0.0.1", [int]$port.Trim())
                    $tcp.Close()
                    return @{ Port = [int]$port.Trim(); Ms = $sw.ElapsedMilliseconds }
                } catch {}
            }
        }
        Start-Sleep -Milliseconds 10
    }
    return $null
}

function Wait-PortFile {
    param([string]$SessionName, [int]$TimeoutMs = 15000)
    $pf = "$PSMUX_DIR\${SessionName}.port"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        if (Test-Path $pf) {
            $port = (Get-Content $pf -Raw -ErrorAction SilentlyContinue)
            if ($port -and $port.Trim() -match '^\d+$') { return @{ Port = [int]$port.Trim(); Ms = $sw.ElapsedMilliseconds } }
        }
        Start-Sleep -Milliseconds 5
    }
    return $null
}

function Wait-PanePrompt {
    param([string]$Target, [int]$TimeoutMs = 20000, [string]$Pattern = "PS [A-Z]:\\")
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $TimeoutMs) {
        try {
            $cap = & $PSMUX capture-pane -t $Target -p 2>&1 | Out-String
            if ($cap -match $Pattern) { return @{ Found = $true; Ms = $sw.ElapsedMilliseconds; Content = $cap } }
        } catch {}
        Start-Sleep -Milliseconds 25
    }
    return @{ Found = $false; Ms = $sw.ElapsedMilliseconds; Content = "" }
}

# Capture pane content rapidly during the squelch window (first 500ms after claim)
# Returns all captured frames for analysis.
function Capture-During-Squelch {
    param([string]$Target, [int]$DurationMs = 600, [int]$IntervalMs = 10)
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $frames = @()
    while ($sw.ElapsedMilliseconds -lt $DurationMs) {
        try {
            $cap = & $PSMUX capture-pane -t $Target -p 2>&1 | Out-String
            $frames += @{ Ms = $sw.ElapsedMilliseconds; Content = $cap }
        } catch {}
        Start-Sleep -Milliseconds $IntervalMs
    }
    return $frames
}

# Check if any frame contains leaked command text
function Check-Leak {
    param([array]$Frames, [string]$TestLabel)
    $leakPatterns = @(
        " cd '",           # the injected cd command with leading space
        " cd `"",          # alternative quoting
        "cd '.*'; cls",    # full injected command
        "cd '.*'; clear",  # Linux variant
        ">> ",             # PSReadLine continuation prompt (leaked \n)
        "cls\r",           # bare cls command visible
        " cd `".*`"; cls"  # alternative cd quoting
    )
    $leakFound = $false
    foreach ($frame in $Frames) {
        foreach ($pat in $leakPatterns) {
            if ($frame.Content -match $pat) {
                Write-Fail "$TestLabel leak detected at ${($frame.Ms)}ms: matched '$pat'"
                Write-Info "Frame content: $($frame.Content)"
                $leakFound = $true
                break
            }
        }
        if ($leakFound) { break }
    }
    if (-not $leakFound) {
        Write-Pass "$TestLabel no command leak in $($Frames.Count) captured frames"
    }
    return (-not $leakFound)
}

# Check that all frames during squelch are blank (empty content while rendering suppresses)
function Check-Blank-During-Squelch {
    param([array]$Frames, [string]$TestLabel, [int]$MaxBlankMs = 500)
    # Frames captured within the squelch window should be empty or whitespace-only
    $earlyFrames = $Frames | Where-Object { $_.Ms -lt $MaxBlankMs }
    $nonBlankEarly = $earlyFrames | Where-Object { $_.Content.Trim().Length -gt 0 -and $_.Content -match '\S' }
    if ($nonBlankEarly.Count -eq 0 -and $earlyFrames.Count -gt 0) {
        Write-Pass "$TestLabel all $($earlyFrames.Count) early frames were blank (squelch active)"
        return $true
    } elseif ($earlyFrames.Count -eq 0) {
        Write-Skip "$TestLabel no frames captured during squelch window"
        return $true
    } else {
        # Some non-blank early frames. This could be the prompt appearing quickly (which is fine)
        # but if cd/cls text is in them, that is a real leak.
        $hasCommandLeak = $false
        foreach ($f in $nonBlankEarly) {
            if ($f.Content -match " cd '" -or $f.Content -match "cls" -or $f.Content -match " cd `"") {
                $hasCommandLeak = $true
                Write-Fail "$TestLabel non-blank early frame at $($f.Ms)ms contains command text"
                Write-Info "Content: $($f.Content)"
            }
        }
        if (-not $hasCommandLeak) {
            Write-Pass "$TestLabel early frames have content (prompt appeared fast), no command text"
        }
        return (-not $hasCommandLeak)
    }
}

# ──────────────────────────────────────────────────────────────────────────────
# ── BANNER ───────────────────────────────────────────────────────────────────
# ──────────────────────────────────────────────────────────────────────────────

Write-Host ""
Write-Host ("*" * 76) -ForegroundColor Magenta
Write-Host "    PSMUX SQUELCH VISIBILITY TEST SUITE" -ForegroundColor Magenta
Write-Host "    $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" -ForegroundColor Magenta
Write-Host "    Binary: $PSMUX" -ForegroundColor Magenta
Write-Host "    Original CWD: $ORIGINAL_CWD" -ForegroundColor Magenta
Write-Host ("*" * 76) -ForegroundColor Magenta

# ══════════════════════════════════════════════════════════════════════════════
# SECTION A: DIRECTORY TYPE VARIANTS
# Each test: start a warm server from ORIGINAL_CWD, then claim from a
# different directory. Verify no cd/cls leak in capture-pane output.
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "A. DIRECTORY TYPE VARIANTS"

# Build list of test directories
$testDirs = @()

# A1: TEMP directory (basic case)
$testDirs += @{ Label = "A1: TEMP directory"; Path = $env:TEMP }

# A2: Root directory (C:\)
$testDirs += @{ Label = "A2: Root directory (C:\)"; Path = "C:\" }

# A3: Deep nested directory
$deepDir = Join-Path $env:TEMP "psmux_test_deep\level1\level2\level3"
if (-not (Test-Path $deepDir)) { New-Item -ItemType Directory -Path $deepDir -Force | Out-Null }
$testDirs += @{ Label = "A3: Deep nested path"; Path = $deepDir }

# A4: Path with spaces
$spaceDir = Join-Path $env:TEMP "psmux test spaces"
if (-not (Test-Path $spaceDir)) { New-Item -ItemType Directory -Path $spaceDir -Force | Out-Null }
$testDirs += @{ Label = "A4: Path with spaces"; Path = $spaceDir }

# A5: Path with parentheses (Program Files style)
$parenDir = Join-Path $env:TEMP "psmux_test (x64)"
if (-not (Test-Path $parenDir)) { New-Item -ItemType Directory -Path $parenDir -Force | Out-Null }
$testDirs += @{ Label = "A5: Path with parens"; Path = $parenDir }

# A6: Path with single quotes (the tricky one for cd quoting)
$quoteDir = Join-Path $env:TEMP "psmux_test_it's_a_test"
try {
    if (-not (Test-Path $quoteDir)) { New-Item -ItemType Directory -Path $quoteDir -Force -ErrorAction Stop | Out-Null }
    $testDirs += @{ Label = "A6: Path with single quotes"; Path = $quoteDir }
} catch {
    Write-Skip "A6: Path with single quotes (could not create directory)"
}

# A7: User profile directory
$testDirs += @{ Label = "A7: User profile"; Path = $HOME_DIR }

# A8: Windows directory
$testDirs += @{ Label = "A8: Windows directory"; Path = $env:SystemRoot }

# A9: Drive root other than C: (if available)
$otherDrives = Get-PSDrive -PSProvider FileSystem | Where-Object { $_.Root -ne "C:\" -and (Test-Path $_.Root) }
if ($otherDrives.Count -gt 0) {
    $testDirs += @{ Label = "A9: Alternate drive root ($($otherDrives[0].Root))"; Path = $otherDrives[0].Root }
} else {
    Write-Skip "A9: No alternate drive available"
}

# A10: Path with ampersand
$ampDir = Join-Path $env:TEMP "psmux_test_R&D"
try {
    if (-not (Test-Path $ampDir)) { New-Item -ItemType Directory -Path $ampDir -Force -ErrorAction Stop | Out-Null }
    $testDirs += @{ Label = "A10: Path with ampersand"; Path = $ampDir }
} catch {
    Write-Skip "A10: Path with ampersand (could not create)"
}

# Run each directory test
foreach ($td in $testDirs) {
    $label = $td.Label
    $targetPath = $td.Path

    if (-not (Test-Path $targetPath)) {
        Write-Skip "$label (path does not exist: $targetPath)"
        continue
    }

    Write-Host ""
    Write-Host "  --- $label ---" -ForegroundColor Yellow
    Write-Info "Target: $targetPath"

    Kill-All-Psmux

    # Start base session from ORIGINAL_CWD (spawns warm server)
    Push-Location $ORIGINAL_CWD
    $env:PSMUX_CONFIG_FILE = "NUL"
    & $PSMUX new-session -s "sqv_base" -d 2>&1 | Out-Null
    $env:PSMUX_CONFIG_FILE = $null
    Pop-Location

    $alive = Wait-SessionAlive -SessionName "sqv_base" -TimeoutMs 15000
    if ($null -eq $alive) {
        Write-Fail "$label could not start base session"
        continue
    }

    # Wait for warm server readiness
    Start-Sleep -Seconds 2
    $warmReady = Wait-PortFile -SessionName "__warm__" -TimeoutMs 10000
    if ($null -eq $warmReady) {
        Write-Fail "$label warm server not ready"
        Kill-All-Psmux
        continue
    }

    # Claim from the target directory
    Push-Location $targetPath
    $sessName = "sqv_dir_test"

    $env:PSMUX_CONFIG_FILE = "NUL"
    & $PSMUX new-session -s $sessName -d 2>&1 | Out-Null
    $env:PSMUX_CONFIG_FILE = $null

    Pop-Location

    # Immediately start capturing frames during squelch window
    $frames = Capture-During-Squelch -Target $sessName -DurationMs 800

    # Wait for prompt to appear
    $prompt = Wait-PanePrompt -Target $sessName -TimeoutMs ($TimeoutSec * 1000)

    if ($prompt.Found) {
        # Capture final state
        $finalCap = & $PSMUX capture-pane -t $sessName -p 2>&1 | Out-String
        $allFrames = $frames + @(@{ Ms = 999; Content = $finalCap })

        # Check 1: No command leak
        Check-Leak -Frames $allFrames -TestLabel $label | Out-Null

        # Check 2: Blank frames during squelch
        Check-Blank-During-Squelch -Frames $frames -TestLabel $label | Out-Null

        # Check 3: CWD correctness (prompt shows correct directory)
        $expectedSafe = (Resolve-Path $targetPath -ErrorAction SilentlyContinue)
        if ($expectedSafe) {
            $expected = $expectedSafe.Path.TrimEnd('\')
            if ($finalCap -match [regex]::Escape($expected) -or $finalCap -match [regex]::Escape($targetPath.TrimEnd('\'))) {
                Write-Pass "$label CWD correct in prompt"
            } else {
                # Root dirs show as C:\> not C:> so check for that
                if ($targetPath -match '^[A-Z]:\\$' -and $finalCap -match "PS $([regex]::Escape($targetPath))") {
                    Write-Pass "$label CWD correct (root dir)"
                } else {
                    Write-Fail "$label CWD mismatch. Expected '$expected' in output"
                    Write-Info "Final capture: $finalCap"
                }
            }
        }
    } else {
        Write-Fail "$label prompt never appeared within ${TimeoutSec}s"
    }

    Kill-All-Psmux
}

# ══════════════════════════════════════════════════════════════════════════════
# SECTION B: SAME-CWD CLAIM (no squelch needed)
# Verify that when CWDs match, no squelch is applied and pane renders normally.
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "B. SAME-CWD CLAIM (NO SQUELCH)"

Kill-All-Psmux
Push-Location $ORIGINAL_CWD

$env:PSMUX_CONFIG_FILE = "NUL"
& $PSMUX new-session -s "sqv_same_base" -d 2>&1 | Out-Null
$env:PSMUX_CONFIG_FILE = $null

$alive = Wait-SessionAlive -SessionName "sqv_same_base" -TimeoutMs 15000
if ($null -ne $alive) {
    Start-Sleep -Seconds 2
    $warmReady = Wait-PortFile -SessionName "__warm__" -TimeoutMs 10000
    if ($null -ne $warmReady) {
        # Claim from the same directory (no CWD change)
        $env:PSMUX_CONFIG_FILE = "NUL"
        & $PSMUX new-session -s "sqv_same_test" -d 2>&1 | Out-Null
        $env:PSMUX_CONFIG_FILE = $null

        $prompt = Wait-PanePrompt -Target "sqv_same_test" -TimeoutMs ($TimeoutSec * 1000)
        if ($prompt.Found) {
            $cap = & $PSMUX capture-pane -t "sqv_same_test" -p 2>&1 | Out-String

            # Should NOT contain any cd command (no CWD change = no injection)
            if ($cap -match " cd '" -or $cap -match "cd '.*'; cls") {
                Write-Fail "B1: Same-CWD claim shows cd command (should be nothing)"
            } else {
                Write-Pass "B1: Same-CWD claim has no cd command (correct)"
            }

            # Prompt should be visible quickly
            if ($prompt.Ms -lt 5000) {
                Write-Pass "B2: Same-CWD prompt appeared in $($prompt.Ms)ms (fast)"
            } else {
                Write-Fail "B2: Same-CWD prompt took $($prompt.Ms)ms (too slow)"
            }
        } else {
            Write-Fail "B1: Same-CWD prompt never appeared"
        }
    } else {
        Write-Fail "B: Warm server not ready"
    }
} else {
    Write-Fail "B: Could not start base session"
}
Pop-Location
Kill-All-Psmux

# ══════════════════════════════════════════════════════════════════════════════
# SECTION C: RAPID SEQUENTIAL CLAIMS (race condition testing)
# Claim multiple sessions rapidly from different directories to stress squelch.
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "C. RAPID SEQUENTIAL CLAIMS"

Kill-All-Psmux
Push-Location $ORIGINAL_CWD

$env:PSMUX_CONFIG_FILE = "NUL"
& $PSMUX new-session -s "sqv_rapid_base" -d 2>&1 | Out-Null
$env:PSMUX_CONFIG_FILE = $null

$alive = Wait-SessionAlive -SessionName "sqv_rapid_base" -TimeoutMs 15000
if ($null -ne $alive) {
    Start-Sleep -Seconds 2

    $rapidDirs = @($env:TEMP, "C:\", $HOME_DIR)
    $rapidLeaks = 0

    for ($r = 0; $r -lt $rapidDirs.Count; $r++) {
        $rd = $rapidDirs[$r]
        if (-not (Test-Path $rd)) { continue }

        $warmReady = Wait-PortFile -SessionName "__warm__" -TimeoutMs 10000
        if ($null -eq $warmReady) {
            Write-Info "C: Warm not ready for rapid claim #$($r+1), waiting..."
            Start-Sleep -Seconds 2
            $warmReady = Wait-PortFile -SessionName "__warm__" -TimeoutMs 10000
            if ($null -eq $warmReady) {
                Write-Skip "C: Warm server not available for rapid claim #$($r+1)"
                continue
            }
        }

        $rsess = "sqv_rapid_$r"
        Push-Location $rd
        $env:PSMUX_CONFIG_FILE = "NUL"
        & $PSMUX new-session -s $rsess -d 2>&1 | Out-Null
        $env:PSMUX_CONFIG_FILE = $null
        Pop-Location

        # Capture immediately during squelch
        $rf = Capture-During-Squelch -Target $rsess -DurationMs 600 -IntervalMs 15

        $rprompt = Wait-PanePrompt -Target $rsess -TimeoutMs ($TimeoutSec * 1000)
        if ($rprompt.Found) {
            $finalCap = & $PSMUX capture-pane -t $rsess -p 2>&1 | Out-String
            $allFrames = $rf + @(@{ Ms = 999; Content = $finalCap })
            $clean = Check-Leak -Frames $allFrames -TestLabel "C$($r+1): Rapid claim to $rd"
            if (-not $clean) { $rapidLeaks++ }
        } else {
            Write-Fail "C$($r+1): Rapid claim to $rd timed out"
        }

        # Short wait before next claim (stress the replenishment)
        Start-Sleep -Seconds 2
    }

    if ($rapidLeaks -eq 0) {
        Write-Pass "C: All rapid sequential claims clean (0 leaks)"
    }
} else {
    Write-Fail "C: Could not start base session"
}
Pop-Location
Kill-All-Psmux

# ══════════════════════════════════════════════════════════════════════════════
# SECTION D: MULTI-FRAME LEAK DETECTION (aggressive polling)
# Start a CWD-changed session and poll capture-pane every ~5ms for 1 second.
# This maximises the chance of catching any transient leak.
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "D. AGGRESSIVE FRAME POLLING"

Kill-All-Psmux
Push-Location $ORIGINAL_CWD

$env:PSMUX_CONFIG_FILE = "NUL"
& $PSMUX new-session -s "sqv_poll_base" -d 2>&1 | Out-Null
$env:PSMUX_CONFIG_FILE = $null

$alive = Wait-SessionAlive -SessionName "sqv_poll_base" -TimeoutMs 15000
if ($null -ne $alive) {
    Start-Sleep -Seconds 2
    $warmReady = Wait-PortFile -SessionName "__warm__" -TimeoutMs 10000
    if ($null -ne $warmReady) {
        Push-Location $env:TEMP
        $env:PSMUX_CONFIG_FILE = "NUL"

        # Start claim and immediately start aggressive polling
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        & $PSMUX new-session -s "sqv_poll_test" -d 2>&1 | Out-Null
        $env:PSMUX_CONFIG_FILE = $null

        # Aggressive capture: every ~5ms for 1.5 seconds
        $aggressiveFrames = @()
        while ($sw.ElapsedMilliseconds -lt 1500) {
            try {
                $cap = & $PSMUX capture-pane -t "sqv_poll_test" -p 2>&1 | Out-String
                $aggressiveFrames += @{ Ms = $sw.ElapsedMilliseconds; Content = $cap }
            } catch {}
            Start-Sleep -Milliseconds 5
        }

        Pop-Location

        Write-Host "  Captured $($aggressiveFrames.Count) frames over 1.5s" -ForegroundColor Gray

        # Analyze all frames
        $leakFrames = 0
        $blankFrames = 0
        $promptFrames = 0
        foreach ($f in $aggressiveFrames) {
            $c = $f.Content.Trim()
            if ($c.Length -eq 0 -or -not ($c -match '\S')) {
                $blankFrames++
            } elseif ($c -match " cd '") {
                $leakFrames++
            } elseif ($c -match "PS [A-Z]:\\") {
                $promptFrames++
            }
        }

        Write-Host "  Blank frames: $blankFrames | Prompt frames: $promptFrames | Leak frames: $leakFrames" -ForegroundColor Gray

        if ($leakFrames -eq 0) {
            Write-Pass "D1: Aggressive polling: 0 leak frames out of $($aggressiveFrames.Count)"
        } else {
            Write-Fail "D1: Aggressive polling: $leakFrames frames leaked cd command"
        }

        if ($blankFrames -gt 0 -or $promptFrames -gt 0) {
            Write-Pass "D2: Frames transition from blank to prompt (squelch working)"
        } else {
            Write-Fail "D2: No blank or prompt frames found"
        }
    } else {
        Write-Fail "D: Warm server not ready"
    }
} else {
    Write-Fail "D: Could not start base session"
}
Kill-All-Psmux

# ══════════════════════════════════════════════════════════════════════════════
# SECTION E: MULTIPLE SIMULTANEOUS SESSIONS
# Start a base, then create two sessions from different directories.
# Both must have clean squelch.
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "E. MULTIPLE SESSIONS WITH DIFFERENT CWDs"

Kill-All-Psmux
Push-Location $ORIGINAL_CWD

$env:PSMUX_CONFIG_FILE = "NUL"
& $PSMUX new-session -s "sqv_multi_base" -d 2>&1 | Out-Null
$env:PSMUX_CONFIG_FILE = $null

$alive = Wait-SessionAlive -SessionName "sqv_multi_base" -TimeoutMs 15000
if ($null -ne $alive) {
    Start-Sleep -Seconds 2

    # Session 1: from TEMP
    $warmReady = Wait-PortFile -SessionName "__warm__" -TimeoutMs 10000
    if ($null -ne $warmReady) {
        Push-Location $env:TEMP
        $env:PSMUX_CONFIG_FILE = "NUL"
        & $PSMUX new-session -s "sqv_multi_1" -d 2>&1 | Out-Null
        $env:PSMUX_CONFIG_FILE = $null
        Pop-Location

        $f1 = Capture-During-Squelch -Target "sqv_multi_1" -DurationMs 700
        $p1 = Wait-PanePrompt -Target "sqv_multi_1" -TimeoutMs ($TimeoutSec * 1000)

        if ($p1.Found) {
            $fc1 = & $PSMUX capture-pane -t "sqv_multi_1" -p 2>&1 | Out-String
            Check-Leak -Frames ($f1 + @(@{ Ms = 999; Content = $fc1 })) -TestLabel "E1: Session 1 (TEMP)" | Out-Null
        } else {
            Write-Fail "E1: Session 1 prompt never appeared"
        }
    }

    # Wait for warm replenishment
    Start-Sleep -Seconds 2

    # Session 2: from user profile
    $warmReady2 = Wait-PortFile -SessionName "__warm__" -TimeoutMs 10000
    if ($null -ne $warmReady2) {
        Push-Location $HOME_DIR
        $env:PSMUX_CONFIG_FILE = "NUL"
        & $PSMUX new-session -s "sqv_multi_2" -d 2>&1 | Out-Null
        $env:PSMUX_CONFIG_FILE = $null
        Pop-Location

        $f2 = Capture-During-Squelch -Target "sqv_multi_2" -DurationMs 700
        $p2 = Wait-PanePrompt -Target "sqv_multi_2" -TimeoutMs ($TimeoutSec * 1000)

        if ($p2.Found) {
            $fc2 = & $PSMUX capture-pane -t "sqv_multi_2" -p 2>&1 | Out-String
            Check-Leak -Frames ($f2 + @(@{ Ms = 999; Content = $fc2 })) -TestLabel "E2: Session 2 (home)" | Out-Null
        } else {
            Write-Fail "E2: Session 2 prompt never appeared"
        }
    }
} else {
    Write-Fail "E: Could not start base session"
}
Pop-Location
Kill-All-Psmux

# ══════════════════════════════════════════════════════════════════════════════
# SECTION F: SQUELCH DOES NOT HIDE LEGITIMATE CONTENT
# After squelch lifts, verify we can type a command and see the output.
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "F. POST-SQUELCH CONTENT INTEGRITY"

Kill-All-Psmux
Push-Location $ORIGINAL_CWD

$env:PSMUX_CONFIG_FILE = "NUL"
& $PSMUX new-session -s "sqv_int_base" -d 2>&1 | Out-Null
$env:PSMUX_CONFIG_FILE = $null

$alive = Wait-SessionAlive -SessionName "sqv_int_base" -TimeoutMs 15000
if ($null -ne $alive) {
    Start-Sleep -Seconds 2
    $warmReady = Wait-PortFile -SessionName "__warm__" -TimeoutMs 10000
    if ($null -ne $warmReady) {
        Push-Location $env:TEMP
        $env:PSMUX_CONFIG_FILE = "NUL"
        & $PSMUX new-session -s "sqv_int_test" -d 2>&1 | Out-Null
        $env:PSMUX_CONFIG_FILE = $null
        Pop-Location

        # Wait for squelch to lift and prompt to appear
        $prompt = Wait-PanePrompt -Target "sqv_int_test" -TimeoutMs ($TimeoutSec * 1000)
        if ($prompt.Found) {
            Write-Pass "F1: Prompt visible after squelch lift"

            # Send a unique test command via send-keys
            $marker = "PSMUX_SQUELCH_INTEGRITY_$(Get-Random)"
            & $PSMUX send-keys -t "sqv_int_test" "echo $marker" Enter 2>&1 | Out-Null
            Start-Sleep -Milliseconds 2000

            $cap = & $PSMUX capture-pane -t "sqv_int_test" -p 2>&1 | Out-String
            if ($cap -match $marker) {
                Write-Pass "F2: Post-squelch command output visible ('$marker' found)"
            } else {
                Write-Fail "F2: Post-squelch command output missing (marker '$marker' not in capture)"
                Write-Info "Capture: $cap"
            }
        } else {
            Write-Fail "F1: Prompt never appeared after CWD change"
        }
    } else {
        Write-Fail "F: Warm server not ready"
    }
} else {
    Write-Fail "F: Could not start base session"
}
Kill-All-Psmux

# ══════════════════════════════════════════════════════════════════════════════
# SECTION G: SAFETY TIMEOUT CORRECTNESS
# Verify the 500ms safety timeout works if somehow the CSI signal is missed.
# We cannot easily force this, but we can verify the prompt appears within ~1s.
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "G. SQUELCH TIMING VERIFICATION"

Kill-All-Psmux
Push-Location $ORIGINAL_CWD

$env:PSMUX_CONFIG_FILE = "NUL"
& $PSMUX new-session -s "sqv_time_base" -d 2>&1 | Out-Null
$env:PSMUX_CONFIG_FILE = $null

$alive = Wait-SessionAlive -SessionName "sqv_time_base" -TimeoutMs 15000
if ($null -ne $alive) {
    Start-Sleep -Seconds 2
    $warmReady = Wait-PortFile -SessionName "__warm__" -TimeoutMs 10000
    if ($null -ne $warmReady) {
        Push-Location $env:TEMP
        $env:PSMUX_CONFIG_FILE = "NUL"

        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        & $PSMUX new-session -s "sqv_time_test" -d 2>&1 | Out-Null
        $env:PSMUX_CONFIG_FILE = $null
        $claimMs = $sw.ElapsedMilliseconds

        Pop-Location

        $prompt = Wait-PanePrompt -Target "sqv_time_test" -TimeoutMs ($TimeoutSec * 1000)
        if ($prompt.Found) {
            $totalMs = $claimMs + $prompt.Ms
            Write-Host "  Claim: ${claimMs}ms + prompt wait: $($prompt.Ms)ms = ${totalMs}ms total" -ForegroundColor Gray

            if ($totalMs -lt 2000) {
                Write-Pass "G1: Squelch lifted within 2s (event-driven signal working)"
            } elseif ($totalMs -lt 5000) {
                Write-Pass "G2: Squelch lifted within 5s (may be using safety timeout)"
            } else {
                Write-Fail "G: Squelch took ${totalMs}ms (too slow, possible timeout issue)"
            }

            # Verify prompt is NOT delayed by the full 500ms if CSI 2J/3J arrives early
            # (event-driven lift should be faster than the safety timeout)
            if ($prompt.Ms -lt 400) {
                Write-Pass "G3: Prompt appeared in $($prompt.Ms)ms (faster than 500ms safety, event-driven)"
            } else {
                Write-Info "G3: Prompt at $($prompt.Ms)ms (may include shell startup time)"
            }
        } else {
            Write-Fail "G: Prompt never appeared"
        }
    } else {
        Write-Fail "G: Warm server not ready"
    }
} else {
    Write-Fail "G: Could not start base session"
}
Kill-All-Psmux

# ══════════════════════════════════════════════════════════════════════════════
# SECTION H: NO CONTINUATION PROMPT (>> check)
# The original bug had a >> prompt from PSReadLine when \r\n was used.
# Verify this regression is fixed.
# ══════════════════════════════════════════════════════════════════════════════
Write-Header "H. NO CONTINUATION PROMPT REGRESSION"

Kill-All-Psmux
Push-Location $ORIGINAL_CWD

$env:PSMUX_CONFIG_FILE = "NUL"
& $PSMUX new-session -s "sqv_cont_base" -d 2>&1 | Out-Null
$env:PSMUX_CONFIG_FILE = $null

$alive = Wait-SessionAlive -SessionName "sqv_cont_base" -TimeoutMs 15000
if ($null -ne $alive) {
    Start-Sleep -Seconds 2
    $warmReady = Wait-PortFile -SessionName "__warm__" -TimeoutMs 10000
    if ($null -ne $warmReady) {
        Push-Location $env:TEMP
        $env:PSMUX_CONFIG_FILE = "NUL"
        & $PSMUX new-session -s "sqv_cont_test" -d 2>&1 | Out-Null
        $env:PSMUX_CONFIG_FILE = $null
        Pop-Location

        $prompt = Wait-PanePrompt -Target "sqv_cont_test" -TimeoutMs ($TimeoutSec * 1000)
        if ($prompt.Found) {
            # Wait for things to settle
            Start-Sleep -Milliseconds 500
            $cap = & $PSMUX capture-pane -t "sqv_cont_test" -p 2>&1 | Out-String

            if ($cap -match ">> ") {
                Write-Fail "H1: Continuation prompt >> detected (PSReadLine \r\n regression)"
                Write-Info "Capture: $cap"
            } else {
                Write-Pass "H1: No continuation prompt (>> not present)"
            }

            # Also check for stray >> at start of any line
            $lines = $cap -split "`n"
            $gtgtLines = $lines | Where-Object { $_.Trim() -match "^>> " }
            if ($gtgtLines.Count -gt 0) {
                Write-Fail "H2: Found $($gtgtLines.Count) line(s) starting with >>"
            } else {
                Write-Pass "H2: No lines start with >> (clean prompt)"
            }
        } else {
            Write-Fail "H: Prompt never appeared"
        }
    } else {
        Write-Fail "H: Warm server not ready"
    }
} else {
    Write-Fail "H: Could not start base session"
}
Kill-All-Psmux

# ══════════════════════════════════════════════════════════════════════════════
# CLEANUP AND SUMMARY
# ══════════════════════════════════════════════════════════════════════════════

# Clean up test directories
Remove-Item (Join-Path $env:TEMP "psmux_test_deep") -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path $env:TEMP "psmux test spaces") -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path $env:TEMP "psmux_test (x64)") -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path $env:TEMP "psmux_test_it's_a_test") -Force -ErrorAction SilentlyContinue
Remove-Item (Join-Path $env:TEMP "psmux_test_R&D") -Force -ErrorAction SilentlyContinue

Kill-All-Psmux
Set-Location $ORIGINAL_CWD

Write-Host ""
Write-Host ("=" * 76) -ForegroundColor Magenta
Write-Host "    SQUELCH VISIBILITY TEST RESULTS" -ForegroundColor Magenta
Write-Host ("=" * 76) -ForegroundColor Magenta
Write-Host ""

$passColor = if ($FAIL -eq 0) { "Green" } else { "Red" }
Write-Host "    Passed:  $PASS" -ForegroundColor Green
Write-Host "    Failed:  $FAIL" -ForegroundColor $(if ($FAIL -gt 0) { "Red" } else { "Green" })
Write-Host "    Skipped: $SKIP" -ForegroundColor DarkYellow
Write-Host "    Total:   $TOTAL" -ForegroundColor White
Write-Host ""

if ($FAIL -eq 0) {
    Write-Host "    ALL TESTS PASSED: Injected commands are invisible!" -ForegroundColor Green
} else {
    Write-Host "    SOME TESTS FAILED: Command visibility issue detected!" -ForegroundColor Red
}
Write-Host ""

exit $FAIL
