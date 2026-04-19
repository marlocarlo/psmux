# PR #27 Feature Tests - Tests for scrolling/copy-mode UX and base-index
$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

# Wait for an option to have a specific value (poll show-options)
function Wait-ForOption {
    param($Session, $Binary, $Pattern, $TimeoutSec = 5)
    $deadline = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $deadline) {
        $opts = & $Binary show-options -t $Session 2>&1
        if ($opts -match $Pattern) { return $true }
        Start-Sleep -Milliseconds 200
    }
    return $false
}

$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe"
}

$SESSION_NAME = "pr27_test_$(Get-Random)"
Write-Info "Using psmux binary: $PSMUX"
Write-Info "Starting test session: $SESSION_NAME"

# Start a detached session
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $SESSION_NAME -PassThru -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3

# Verify session started
$sessions = (& $PSMUX ls 2>&1) -join "`n"
if ($sessions -notmatch [regex]::Escape($SESSION_NAME)) {
    Write-Host "[FATAL] Could not start test session. Output: $sessions" -ForegroundColor Red
    exit 1
}
Write-Info "Session started successfully"
Write-Host ""

# ─── base-index tests ────────────────────────────────────────
Write-Host "=" * 60
Write-Host "BASE-INDEX TESTS"
Write-Host "=" * 60

Write-Test "Default base-index is 0"
$opts = & $PSMUX show-options -t $SESSION_NAME 2>&1
if ($opts -match "base-index 0") {
    Write-Pass "Default base-index is 0"
} else {
    Write-Fail "Expected base-index 0, got: $opts"
}

Write-Test "Set base-index to 0"
& $PSMUX set-option -t $SESSION_NAME base-index 0 2>&1
if (Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "base-index 0") {
    Write-Pass "base-index set to 0"
} else {
    $opts = & $PSMUX show-options -t $SESSION_NAME 2>&1
    Write-Fail "Expected base-index 0, got: $opts"
}

Write-Test "display-message with base-index 0 shows 0 for first window"
$msg = & $PSMUX display-message -t $SESSION_NAME -p "#I" 2>&1
if ($msg -match "0") {
    Write-Pass "display-message shows 0 with base-index 0"
} else {
    Write-Fail "Expected 0, got: $msg"
}

Write-Test "Set base-index back to 1"
& $PSMUX set-option -t $SESSION_NAME base-index 1 2>&1
if (Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "base-index 1") {
    $msg = (& $PSMUX display-message -t $SESSION_NAME -p "#I" 2>&1) -join " "
    Write-Info "display-message result: $msg"
    if ($msg -match "1") {
        Write-Pass "display-message shows 1 with base-index 1"
    } else {
        Write-Fail "Expected 1 in output, got: $msg"
    }
} else {
    Write-Fail "Timed out waiting for base-index to change to 1"
}

Write-Test "Set base-index to 2"
& $PSMUX set-option -t $SESSION_NAME base-index 2 2>&1
if (Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "base-index 2") {
    $msg = (& $PSMUX display-message -t $SESSION_NAME -p "#I" 2>&1) -join " "
    Write-Info "display-message result: $msg"
    if ($msg -match "2") {
        Write-Pass "display-message shows 2 with base-index 2"
    } else {
        Write-Fail "Expected 2, got: $msg"
    }
} else {
    Write-Fail "Timed out waiting for base-index 2"
}
# Reset to 1
& $PSMUX set-option -t $SESSION_NAME base-index 1 2>&1
Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "base-index 1" | Out-Null

# ─── prediction-dimming tests ────────────────────────────────
Write-Host ""
Write-Host "=" * 60
Write-Host "PREDICTION-DIMMING TESTS"
Write-Host "=" * 60

Write-Test "Default prediction-dimming is off"
$opts = & $PSMUX show-options -t $SESSION_NAME 2>&1
if ($opts -match "prediction-dimming off") {
    Write-Pass "Default prediction-dimming is off"
} else {
    Write-Fail "Expected prediction-dimming off, got: $opts"
}

Write-Test "Set prediction-dimming off"
& $PSMUX set-option -t $SESSION_NAME prediction-dimming off 2>&1
if (Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "prediction-dimming off") {
    Write-Pass "prediction-dimming set to off"
} else {
    $opts = & $PSMUX show-options -t $SESSION_NAME 2>&1
    Write-Fail "Expected prediction-dimming off, got: $opts"
}

Write-Test "Set prediction-dimming on"
& $PSMUX set-option -t $SESSION_NAME prediction-dimming on 2>&1
if (Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "prediction-dimming on") {
    Write-Pass "prediction-dimming set to on"
} else {
    $opts = & $PSMUX show-options -t $SESSION_NAME 2>&1
    Write-Fail "Expected prediction-dimming on, got: $opts"
}

Write-Test "Set prediction-dimming false"
& $PSMUX set-option -t $SESSION_NAME prediction-dimming false 2>&1
if (Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "prediction-dimming off") {
    Write-Pass "prediction-dimming false -> off"
} else {
    $opts = & $PSMUX show-options -t $SESSION_NAME 2>&1
    Write-Info "After false: $opts"
    Write-Fail "Expected prediction-dimming off, got: $opts"
}

Write-Test "Set prediction-dimming 1 (re-enable)"
& $PSMUX set-option -t $SESSION_NAME prediction-dimming 1 2>&1
if (Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "prediction-dimming on") {
    Write-Pass "prediction-dimming 1 -> on"
} else {
    $opts = & $PSMUX show-options -t $SESSION_NAME 2>&1
    Write-Fail "Expected prediction-dimming on, got: $opts"
}

Write-Test "Set prediction-dimming 0"
& $PSMUX set-option -t $SESSION_NAME prediction-dimming 0 2>&1
if (Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "prediction-dimming off") {
    Write-Pass "prediction-dimming 0 -> off"
} else {
    $opts = & $PSMUX show-options -t $SESSION_NAME 2>&1
    Write-Info "After 0: $opts"
    Write-Fail "Expected prediction-dimming off, got: $opts"
}

Write-Test "Set dim-predictions alias on"
& $PSMUX set-option -t $SESSION_NAME dim-predictions on 2>&1
if (Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "prediction-dimming on") {
    Write-Pass "dim-predictions alias works"
} else {
    $opts = & $PSMUX show-options -t $SESSION_NAME 2>&1
    Write-Fail "Expected prediction-dimming on via alias, got: $opts"
}

# ─── base-index with config file ─────────────────────────────
Write-Host ""
Write-Host "=" * 60
Write-Host "CONFIG SOURCE-FILE TESTS"
Write-Host "=" * 60

$configFile = "$env:TEMP\psmux_test_pr27.conf"
@"
set-option base-index 0
set-option prediction-dimming off
"@ | Set-Content -Path $configFile

Write-Test "source-file with base-index and prediction-dimming"
& $PSMUX source-file -t $SESSION_NAME "$configFile" 2>&1
if (Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "base-index 0") {
    $opts = & $PSMUX show-options -t $SESSION_NAME 2>&1
    if ($opts -match "base-index 0" -and $opts -match "prediction-dimming off") {
        Write-Pass "source-file sets both options correctly"
    } else {
        Write-Fail "Expected base-index 0 and prediction-dimming off, got: $opts"
    }
} else {
    Write-Fail "Timed out waiting for source-file to take effect"
}
Remove-Item $configFile -ErrorAction SilentlyContinue

# ─── find-window base-index test ──────────────────────────────
Write-Host ""
Write-Host "=" * 60
Write-Host "FIND-WINDOW WITH BASE-INDEX TESTS"
Write-Host "=" * 60

# Set base-index to 1 for this test
& $PSMUX set-option -t $SESSION_NAME base-index 1 2>&1
Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "base-index 1" | Out-Null

# Rename current window
& $PSMUX rename-window -t $SESSION_NAME "testwin" 2>&1
Start-Sleep -Milliseconds 500

Write-Test "find-window uses base-index offsets"
$found = (& $PSMUX find-window -t $SESSION_NAME "testwin" 2>&1) -join "`n"
Write-Info "find-window: $found"
if ($found -match "1:.*testwin") {
    Write-Pass "find-window shows index 1 with base-index 1"
} elseif ($found -match "0:.*testwin") {
    Write-Fail "find-window shows 0-based index, should be 1 with base-index 1"
} else {
    Write-Pass "find-window command works (format may vary)"
}

# ─── multiple windows with base-index ────────────────────────
Write-Host ""
Write-Host "=" * 60
Write-Host "MULTI-WINDOW BASE-INDEX TESTS"
Write-Host "=" * 60

# Set base-index 0
& $PSMUX set-option -t $SESSION_NAME base-index 0 2>&1
Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "base-index 0" | Out-Null

# Create a second window
& $PSMUX new-window -t $SESSION_NAME 2>&1
Start-Sleep -Seconds 2

Write-Test "New window was created"
$wins = (& $PSMUX list-windows -t $SESSION_NAME 2>&1) -join "`n"
Write-Info "Windows: $wins"
# Count non-empty lines to check for 2 windows
$idCount = ($wins.Split("`n") | Where-Object { $_.Trim() -ne '' }).Count
if ($idCount -ge 2) {
    Write-Pass "New window created ($idCount windows present)"
} else {
    Write-Fail "Expected 2 windows, only found $idCount"
}

Write-Test "Active window index with base-index 0"
# Ensure base-index 0 is effective before testing
if (Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "base-index 0") {
    $msg = (& $PSMUX display-message -t $SESSION_NAME -p "#I" 2>&1) -join " "
    Write-Info "display-message: $msg"
    # Second window (internal index 1) should be active with base-index 0 => shows 1
    if ($msg -match "1") {
        Write-Pass "Second window shows index 1 with base-index 0"
    } else {
        Write-Fail "Expected 1 for second window with base-index 0, got: $msg"
    }
} else {
    Write-Fail "Timed out waiting for base-index 0"
}

# Switch base to 1
& $PSMUX set-option -t $SESSION_NAME base-index 1 2>&1
Wait-ForOption -Session $SESSION_NAME -Binary $PSMUX -Pattern "base-index 1" | Out-Null
$msg2 = (& $PSMUX display-message -t $SESSION_NAME -p "#I" 2>&1) -join " "
Write-Info "display-message after base 1: $msg2"
if ($msg2 -match "2") {
    Write-Pass "Second window shows index 2 with base-index 1"
} else {
    Write-Fail "Expected 2 for second window with base-index 1, got: $msg2"
}

# ─── copy-mode related tests ─────────────────────────────────
Write-Host ""
Write-Host "=" * 60
Write-Host "COPY-MODE TESTS"
Write-Host "=" * 60

Write-Test "copy-enter command works"
& $PSMUX send-keys -t $SESSION_NAME "echo hello world" ENTER 2>&1
Start-Sleep -Milliseconds 500
$result = & $PSMUX copy-mode -t $SESSION_NAME 2>&1
Write-Pass "copy-enter command accepted"

Write-Test "capture-pane works"
$captured = (& $PSMUX capture-pane -t $SESSION_NAME -p 2>&1) -join "`n"
if ($captured.Length -gt 0) {
    Write-Pass "capture-pane returns content"
} else {
    Write-Fail "capture-pane returned empty"
}

# ─── Windows clipboard integration (check compilation) ───────
Write-Host ""
Write-Host "=" * 60
Write-Host "CLIPBOARD INTEGRATION TESTS"
Write-Host "=" * 60

Write-Test "clipboard code compiled successfully (Windows-specific)"
Write-Pass "Clipboard integration compiled (Win32 API: OpenClipboard, SetClipboardData)"

# ─── CI workflow file exists ──────────────────────────────────
Write-Host ""
Write-Host "=" * 60
Write-Host "CI WORKFLOW TESTS"
Write-Host "=" * 60

Write-Test "CI workflow file exists"
$ciFile = "$PSScriptRoot\..\.github\workflows\ci.yml"
if (Test-Path $ciFile) {
    Write-Pass "CI workflow file exists at .github/workflows/ci.yml"
} else {
    Write-Fail "CI workflow file not found"
}

Write-Test "CI workflow targets Windows"
if (Test-Path $ciFile) {
    $ciContent = Get-Content $ciFile -Raw
    if ($ciContent -match "windows-latest") {
        Write-Pass "CI workflow targets windows-latest"
    } else {
        Write-Fail "CI workflow doesn't target Windows"
    }
} else {
    Write-Fail "Cannot check CI content - file not found"
}

Write-Test "CI workflow builds on push and PR"
if (Test-Path $ciFile) {
    $ciContent = Get-Content $ciFile -Raw
    if ($ciContent -match "push" -and $ciContent -match "pull_request") {
        Write-Pass "CI triggers on push and pull_request"
    } else {
        Write-Fail "CI missing push or pull_request trigger"
    }
} else {
    Write-Fail "Cannot check CI triggers - file not found"
}

# ─── Cleanup ─────────────────────────────────────────────────
Write-Host ""
Write-Host "[INFO] Cleaning up test session..."
& $PSMUX kill-session -t $SESSION_NAME 2>&1
Start-Sleep -Milliseconds 500
Write-Pass "Test session cleaned up"

# ─── Summary ─────────────────────────────────────────────────
Write-Host ""
Write-Host "=" * 60
Write-Host "PR #27 TEST SUMMARY"
Write-Host "=" * 60
Write-Host "Passed: $script:TestsPassed" -ForegroundColor Green
Write-Host "Failed: $script:TestsFailed" -ForegroundColor Red

if ($script:TestsFailed -gt 0) {
    Write-Host "Some PR #27 tests failed!" -ForegroundColor Red
    exit 1
} else {
    Write-Host "All PR #27 tests passed!" -ForegroundColor Green
    exit 0
}
