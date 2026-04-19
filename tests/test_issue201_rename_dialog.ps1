# Issue #201: prefix+$ shows "rename window" dialog instead of "rename session"
#
# The bug: client.rs hardcoded the overlay title as "rename window" even when
# the user pressed prefix+$ (rename session). The fix adds a conditional check
# on session_renaming to display the correct title.
#
# NOTE: The overlay is a client-side TUI element rendered by ratatui, so
# capture-pane cannot see it. The overlay title correctness is verified by
# Rust rendering tests (test_issue201_rename_dialog.rs) using TestBackend.
# This E2E test verifies the FUNCTIONAL behavior: that rename-session and
# rename-window commands actually work correctly.

$ErrorActionPreference = "Stop"
$PSMUX = Get-Command psmux -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source
if (-not $PSMUX) { $PSMUX = "psmux" }
$SESSION = "issue201_test_$(Get-Random)"

$pass = 0
$fail = 0
$results = @()

function Write-Test($msg)  { Write-Host "  TEST: $msg" -ForegroundColor Yellow }
function Write-Pass($msg)  { Write-Host "  PASS: $msg" -ForegroundColor Green; $script:pass++ }
function Write-Fail($msg)  { Write-Host "  FAIL: $msg" -ForegroundColor Red; $script:fail++ }
function Add-Result($name, $ok, $detail) {
    if ($ok) { Write-Pass "$name $detail" } else { Write-Fail "$name $detail" }
    $script:results += [PSCustomObject]@{ Test=$name; Pass=$ok; Detail=$detail }
}

Write-Host "`n=== Issue #201: Rename Session Dialog Text ===" -ForegroundColor Cyan

# Start a detached session
& $PSMUX new-session -d -s $SESSION 2>&1 | Out-Null
Start-Sleep -Seconds 2

$alive = & $PSMUX has-session -t $SESSION 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Fail "Could not create test session $SESSION"
    exit 1
}
Write-Pass "Session $SESSION created"

# ---- TEST 1: rename-session command works correctly ----
Write-Test "rename-session command changes session name"
$NEW_NAME = "renamed_sess_201"
& $PSMUX rename-session -t $SESSION $NEW_NAME 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

$alive = & $PSMUX has-session -t $NEW_NAME 2>&1
$ok = $LASTEXITCODE -eq 0
Add-Result "rename-session changes name" $ok "has-session exit=$LASTEXITCODE"
if ($ok) { $SESSION = $NEW_NAME }

# ---- TEST 2: rename-window command works correctly ----
Write-Test "rename-window command changes window name"
$WIN_NAME = "renamed_win_201"
& $PSMUX rename-window -t $SESSION $WIN_NAME 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

$wlist = & $PSMUX list-windows -t $SESSION 2>&1 | Out-String
$ok2 = $wlist -match $WIN_NAME
Add-Result "rename-window changes name" $ok2 "list-windows contains '$WIN_NAME': $ok2"

# ---- TEST 3: Source code verification (the fix is in place) ----
Write-Test "Source code uses session_renaming conditional for overlay title"
$srcFile = Join-Path $PSScriptRoot "..\src\client.rs"
if (Test-Path $srcFile) {
    $src = Get-Content $srcFile -Raw
    # The fix: the title must be conditionally chosen based on session_renaming
    $hasConditional = $src -match 'if session_renaming.*rename session.*rename window'
    # The bug: hardcoded "rename window" in the renaming block (should NOT exist)
    # Look for the EXACT buggy pattern: if renaming { ... title("rename window") without conditional
    $lines = Get-Content $srcFile
    $inRenamingBlock = $false
    $buggyHardcode = $false
    foreach ($line in $lines) {
        if ($line -match 'if renaming \{') { $inRenamingBlock = $true }
        if ($inRenamingBlock -and $line -match 'title\("rename window"\)' -and $line -notmatch 'session_renaming') {
            $buggyHardcode = $true
        }
        if ($inRenamingBlock -and $line -match '^\s*\}') { $inRenamingBlock = $false }
    }
    Add-Result "Fix present: conditional title selection" $hasConditional ""
    Add-Result "Bug absent: no hardcoded 'rename window' in renaming block" (-not $buggyHardcode) ""
} else {
    Write-Fail "Source file not found at $srcFile"
}

# ---- TEST 4: Verify via control mode that rename-session dispatches correctly ----
Write-Test "rename-session via control mode"
$NEW_NAME2 = "ctrl_renamed_201"
$ctrl_out = & $PSMUX -CC rename-session -t $SESSION $NEW_NAME2 2>&1 | Out-String
Start-Sleep -Milliseconds 500

$alive2 = & $PSMUX has-session -t $NEW_NAME2 2>&1
$ok4 = $LASTEXITCODE -eq 0
Add-Result "rename-session via control mode" $ok4 "exit=$LASTEXITCODE"
if ($ok4) { $SESSION = $NEW_NAME2 }

# Cleanup
& $PSMUX kill-session -t $SESSION 2>&1 | Out-Null

# Summary
Write-Host "`n=== Results ===" -ForegroundColor Cyan
Write-Host "  Passed: $pass / $($pass + $fail)" -ForegroundColor $(if ($fail -eq 0) { "Green" } else { "Yellow" })
foreach ($r in $results) {
    $color = if ($r.Pass) { "Green" } else { "Red" }
    $status = if ($r.Pass) { "PASS" } else { "FAIL" }
    Write-Host "  [$status] $($r.Test)" -ForegroundColor $color
}

if ($fail -gt 0) {
    Write-Host "`n  Some tests failed." -ForegroundColor Red
    exit 1
}
Write-Host "`n  All tests passed. Issue #201 fix verified." -ForegroundColor Green
exit 0
