# psmux Issue #111 follow-up — #{pane_current_path} in default-command
#
# Tests that format variables like #{pane_current_path} are expanded
# when used in `set -g default-command` (not just in -c arguments).
#
# User scenario:
#   set -g default-command "pwsh.exe -NoExit -WorkingDirectory #{pane_current_path}"
#   bind '"' split-window -v
#   -> new pane should open in the same directory as the source pane
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_default_command_format.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:TestsSkipped = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Skip { param($msg) Write-Host "[SKIP] $msg" -ForegroundColor Yellow; $script:TestsSkipped++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using: $PSMUX"

# Clean slate
Write-Info "Cleaning up existing sessions..."
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "test_defcmd"

function Wait-ForSession {
    param($name, $timeout = 10)
    for ($i = 0; $i -lt ($timeout * 2); $i++) {
        & $PSMUX has-session -t $name 2>$null
        if ($LASTEXITCODE -eq 0) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Cleanup-Session {
    param($name)
    & $PSMUX kill-session -t $name 2>$null
    Start-Sleep -Milliseconds 500
}

function Capture-Pane {
    param($target)
    $raw = & $PSMUX capture-pane -t $target -p 2>&1
    return ($raw | Out-String)
}

function New-TestSession {
    param($name)
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $name" -WindowStyle Hidden
    if (-not (Wait-ForSession $name)) {
        Write-Fail "Could not create session $name"
        return $false
    }
    Start-Sleep -Seconds 3
    return $true
}

# ======================================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "ISSUE #111: #{pane_current_path} in default-command"
Write-Host ("=" * 70)
# ======================================================================

# --- Test 1: default-command with #{pane_current_path} in split-window ---
Write-Test "1: default-command with #{pane_current_path} + split-window -v"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_defcmd_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    # Set default-command with #{pane_current_path}
    & $PSMUX set -g -t $SESSION default-command 'pwsh.exe -NoLogo -NoExit -WorkingDirectory "#{pane_current_path}"'
    Start-Sleep -Milliseconds 500

    # cd to testDir in the active pane
    & $PSMUX send-keys -t $SESSION "cd `"$testDir`"" Enter
    Start-Sleep -Seconds 2

    # Split WITHOUT -c (so default-command should be used with expanded format)
    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 5

    # Check CWD in the new pane
    & $PSMUX send-keys -t $SESSION 'Write-Output "DEFCMD_CWD=$($PWD.Path)"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION
    $capFlat = ($cap -replace "`r?`n", "")
    $dirName = Split-Path $testDir -Leaf

    if ($capFlat -match "DEFCMD_CWD=.*$([regex]::Escape($dirName))") {
        Write-Pass "1: default-command #{pane_current_path} expanded in split-window"
    } else {
        Write-Fail "1: CWD not preserved. Expected dir containing '$dirName'. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "1: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 2: default-command with #{pane_current_path} in split-window -h ---
Write-Test "2: default-command with #{pane_current_path} + split-window -h"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_defcmd_h_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    & $PSMUX set -g -t $SESSION default-command 'pwsh.exe -NoLogo -NoExit -WorkingDirectory "#{pane_current_path}"'
    Start-Sleep -Milliseconds 500

    & $PSMUX send-keys -t $SESSION "cd `"$testDir`"" Enter
    Start-Sleep -Seconds 2

    & $PSMUX split-window -h -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 5

    & $PSMUX send-keys -t $SESSION 'Write-Output "DEFCMD_H=$($PWD.Path)"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION
    $capFlat = ($cap -replace "`r?`n", "")
    $dirName = Split-Path $testDir -Leaf

    if ($capFlat -match "DEFCMD_H=.*$([regex]::Escape($dirName))") {
        Write-Pass "2: default-command #{pane_current_path} expanded in split-window -h"
    } else {
        Write-Fail "2: CWD not preserved. Expected '$dirName'. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "2: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 3: default-command with #{pane_current_path} in new-window ---
Write-Test "3: default-command with #{pane_current_path} + new-window"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_defcmd_nw_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    & $PSMUX set -g -t $SESSION default-command 'pwsh.exe -NoLogo -NoExit -WorkingDirectory "#{pane_current_path}"'
    Start-Sleep -Milliseconds 500

    & $PSMUX send-keys -t $SESSION "cd `"$testDir`"" Enter
    Start-Sleep -Seconds 2

    # new-window WITHOUT -c (should use default-command with format expansion)
    & $PSMUX new-window -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 5

    & $PSMUX send-keys -t $SESSION 'Write-Output "DEFCMD_NW=$($PWD.Path)"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION
    $capFlat = ($cap -replace "`r?`n", "")
    $dirName = Split-Path $testDir -Leaf

    if ($capFlat -match "DEFCMD_NW=.*$([regex]::Escape($dirName))") {
        Write-Pass "3: default-command #{pane_current_path} expanded in new-window"
    } else {
        Write-Fail "3: CWD not preserved. Expected '$dirName'. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "3: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 4: default-command without format vars still works (regression) ---
Write-Test "4: default-command without format vars (regression)"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    & $PSMUX set -g -t $SESSION default-command 'pwsh.exe -NoLogo -NoExit'
    Start-Sleep -Milliseconds 500

    & $PSMUX split-window -v -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 5

    & $PSMUX send-keys -t $SESSION 'Write-Output "PLAIN_OK=yes"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION
    $capFlat = ($cap -replace "`r?`n", "")

    if ($capFlat -match "PLAIN_OK=yes") {
        Write-Pass "4: default-command without format vars works"
    } else {
        Write-Fail "4: Plain default-command failed. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "4: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 5: -c flag still takes priority over default-command ---
Write-Test "5: -c flag overrides default-command"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_defcmd_override_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    # Set a default-command that does NOT use #{pane_current_path}
    & $PSMUX set -g -t $SESSION default-command 'pwsh.exe -NoLogo -NoExit'
    Start-Sleep -Milliseconds 500

    # But use -c with a specific path
    & $PSMUX split-window -v -c "$testDir" -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 5

    & $PSMUX send-keys -t $SESSION 'Write-Output "OVERRIDE_CWD=$($PWD.Path)"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION
    $capFlat = ($cap -replace "`r?`n", "")
    $dirName = Split-Path $testDir -Leaf

    if ($capFlat -match "OVERRIDE_CWD=.*$([regex]::Escape($dirName))") {
        Write-Pass "5: -c flag properly overrides default-command CWD"
    } else {
        Write-Fail "5: -c override failed. Expected '$dirName'. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "5: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 6: display-message shows correct pane_current_path ---
Write-Test "6: display-message #{pane_current_path} returns correct dir"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_defcmd_disp_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    & $PSMUX send-keys -t $SESSION "cd `"$testDir`"" Enter
    Start-Sleep -Seconds 2

    $result = & $PSMUX display-message -t $SESSION -p '#{pane_current_path}' 2>&1
    $resultStr = ($result | Out-String).Trim()
    $dirName = Split-Path $testDir -Leaf

    if ($resultStr -match [regex]::Escape($dirName)) {
        Write-Pass "6: display-message #{pane_current_path} returned correct dir"
    } else {
        Write-Fail "6: Expected '$dirName' in result, got: '$resultStr'"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "6: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
}

# ======================================================================
# Final cleanup
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 1

Write-Host ""
Write-Host ("=" * 70)
Write-Host "Results: $($script:TestsPassed) passed, $($script:TestsFailed) failed, $($script:TestsSkipped) skipped"
Write-Host ("=" * 70)
if ($script:TestsFailed -gt 0) { exit 1 } else { exit 0 }
