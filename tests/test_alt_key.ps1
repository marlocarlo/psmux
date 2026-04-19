#!/usr/bin/env pwsh
###############################################################################
# test_alt_key.ps1 — Verify Alt+key events reach the child shell correctly
#
# Issue #102: Alt+f / Alt+b consumed by psmux, not delivered to PSReadLine.
#
# Strategy: Type "echo hello world", press Home to go to start, then send
# Alt+f (ForwardWord in Emacs mode) which should move cursor past "echo".
# Then type "X" — if Alt+f worked, output is "echo Xhello world";
# if Alt+f was consumed, output is "Xecho hello world" (cursor stayed at col 0).
###############################################################################
$ErrorActionPreference = "Continue"

$pass = 0
$fail = 0

function Report {
    param([string]$Name, [bool]$Ok, [string]$Detail = "")
    if ($Ok) { $script:pass++; Write-Host "  [PASS] $Name  $Detail" -ForegroundColor Green }
    else     { $script:fail++; Write-Host "  [FAIL] $Name  $Detail" -ForegroundColor Red }
}

function Kill-All {
    Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force 2>$null
    Start-Sleep -Milliseconds 500
    Get-ChildItem "$env:USERPROFILE\.psmux\*.port" -ErrorAction SilentlyContinue | Remove-Item -Force
    Get-ChildItem "$env:USERPROFILE\.psmux\*.key" -ErrorAction SilentlyContinue | Remove-Item -Force
    Start-Sleep -Milliseconds 300
}

Write-Host "`n================================================================" -ForegroundColor Cyan
Write-Host " Issue #102: Alt+key delivery to PSReadLine" -ForegroundColor Cyan
Write-Host "================================================================`n" -ForegroundColor Cyan

###############################################################################
# TEST 1: Alt+f (ForwardWord) — Emacs mode
###############################################################################
Write-Host "--- TEST 1: Alt+f moves cursor forward one word ---" -ForegroundColor Yellow
Kill-All

psmux new-session -d -s "alt_test" -x 120 -y 30 2>$null
Start-Sleep -Seconds 3

# Configure PSReadLine Emacs mode and type a test string
psmux send-keys -t "alt_test" 'Set-PSReadLineOption -EditMode Emacs' Enter
Start-Sleep -Milliseconds 800

# Type the test text
psmux send-keys -t "alt_test" 'echo hello world'
Start-Sleep -Milliseconds 300

# Press Home to go to beginning of line
psmux send-keys -t "alt_test" Home
Start-Sleep -Milliseconds 300

# Send Alt+f to move forward one word (should move past "echo")
psmux send-keys -t "alt_test" M-f
Start-Sleep -Milliseconds 500

# Type 'X' as a marker — if Alt+f worked, cursor is after "echo"
# so we get "echoX hello world" or "echo Xhello world"
psmux send-keys -t "alt_test" X
Start-Sleep -Milliseconds 300

# Now press Home + Shift+End to select all, or just press Enter to execute
# Actually, let's capture the pane content to see where the X ended up
$content = psmux capture-pane -t "alt_test" -p 2>$null
Start-Sleep -Milliseconds 200

Write-Host "  Captured content (last lines):" -ForegroundColor Gray
$lines = $content -split "`n" | Where-Object { $_.Trim() -ne "" }
$lastLines = $lines | Select-Object -Last 5
foreach ($l in $lastLines) {
    Write-Host "    |$l|" -ForegroundColor DarkGray
}

# Check if 'X' appears after "echo" (Alt+f worked) vs at position 0 (Alt+f failed)
$editLine = $lines | Where-Object { $_ -match "echo.*hello.*world" -or $_ -match "Xecho" -or $_ -match "echoX" } | Select-Object -Last 1
Write-Host "  Edit line: |$editLine|" -ForegroundColor Gray

if ($editLine -match "echoX" -or $editLine -match "echo X") {
    Report "Alt+f ForwardWord moves cursor" $true "cursor moved past 'echo'"
} elseif ($editLine -match "Xecho") {
    Report "Alt+f ForwardWord moves cursor" $false "cursor stayed at col 0 — Alt+f was consumed"
} else {
    # Maybe Alt+f moved further — check if X is anywhere after position 0
    $xPos = $editLine.IndexOf('X')
    if ($xPos -gt 0) {
        Report "Alt+f ForwardWord moves cursor" $true "X at position $xPos"
    } else {
        Report "Alt+f ForwardWord moves cursor" $false "could not determine cursor position. Line: $editLine"
    }
}

psmux kill-session -t "alt_test" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 2: Alt+b (BackwardWord)
###############################################################################
Write-Host "`n--- TEST 2: Alt+b moves cursor backward one word ---" -ForegroundColor Yellow
Kill-All

psmux new-session -d -s "alt_test2" -x 120 -y 30 2>$null
Start-Sleep -Seconds 3

psmux send-keys -t "alt_test2" 'Set-PSReadLineOption -EditMode Emacs' Enter
Start-Sleep -Milliseconds 800

# Type test text
psmux send-keys -t "alt_test2" 'echo hello world'
Start-Sleep -Milliseconds 300

# Cursor is at end. Send Alt+b to go back one word (past "world")
psmux send-keys -t "alt_test2" M-b
Start-Sleep -Milliseconds 500

# Type X — should appear before "world": "echo hello Xworld"
psmux send-keys -t "alt_test2" X
Start-Sleep -Milliseconds 300

$content2 = psmux capture-pane -t "alt_test2" -p 2>$null
$lines2 = $content2 -split "`n" | Where-Object { $_.Trim() -ne "" }

$lastLines2 = $lines2 | Select-Object -Last 5
foreach ($l in $lastLines2) {
    Write-Host "    |$l|" -ForegroundColor DarkGray
}

$editLine2 = $lines2 | Where-Object { $_ -match "echo.*hello" -and $_ -match "world" } | Select-Object -Last 1
Write-Host "  Edit line: |$editLine2|" -ForegroundColor Gray

if ($editLine2 -match "Xworld") {
    Report "Alt+b BackwardWord moves cursor" $true "cursor moved before 'world'"
} elseif ($editLine2 -match "worldX") {
    Report "Alt+b BackwardWord moves cursor" $false "cursor stayed at end — Alt+b was consumed"
} else {
    $xPos2 = $editLine2.IndexOf('X')
    if ($xPos2 -ge 0 -and $xPos2 -lt $editLine2.Length - 1) {
        Report "Alt+b BackwardWord moves cursor" $true "X at position $xPos2 (not at end)"
    } else {
        Report "Alt+b BackwardWord moves cursor" $false "could not determine. Line: $editLine2"
    }
}

psmux kill-session -t "alt_test2" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 3: Alt+d (KillWord — deletes forward word in Emacs mode)
###############################################################################
Write-Host "`n--- TEST 3: Alt+d KillWord deletes forward word ---" -ForegroundColor Yellow
Kill-All

psmux new-session -d -s "alt_test3d" -x 120 -y 30 2>$null
Start-Sleep -Seconds 3

psmux send-keys -t "alt_test3d" 'Set-PSReadLineOption -EditMode Emacs' Enter
Start-Sleep -Milliseconds 800

# Type "echo hello world", press Home, then Alt+d should delete "echo"
psmux send-keys -t "alt_test3d" 'echo hello world'
Start-Sleep -Milliseconds 300
psmux send-keys -t "alt_test3d" Home
Start-Sleep -Milliseconds 300
psmux send-keys -t "alt_test3d" M-d
Start-Sleep -Milliseconds 500

$content3d = psmux capture-pane -t "alt_test3d" -p 2>$null
$lines3d = $content3d -split "`n" | Where-Object { $_.Trim() -ne "" }
$editLine3d = $lines3d | Select-Object -Last 1
Write-Host "  Edit line: |$editLine3d|" -ForegroundColor Gray

# After Alt+d at start, "echo" should be deleted, leaving " hello world"
if ($editLine3d -match "hello world" -and $editLine3d -notmatch "echo") {
    Report "Alt+d KillWord deletes forward word" $true
} elseif ($editLine3d -match "echo hello world") {
    Report "Alt+d KillWord deletes forward word" $false "word not deleted — Alt+d consumed"
} else {
    Report "Alt+d KillWord deletes forward word" $true "line changed: $editLine3d"
}

psmux kill-session -t "alt_test3d" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 4: Plain 'f' key should NOT be affected (regression check)
###############################################################################
Write-Host "`n--- TEST 4: Plain 'f' key still works ---" -ForegroundColor Yellow
Kill-All

psmux new-session -d -s "alt_test3" -x 120 -y 30 2>$null
Start-Sleep -Seconds 3

psmux send-keys -t "alt_test3" 'echo foo' Enter
Start-Sleep -Milliseconds 500

$content3 = psmux capture-pane -t "alt_test3" -p 2>$null
$lines3 = $content3 -split "`n" | Where-Object { $_.Trim() -ne "" }

$hasFoo = $lines3 | Where-Object { $_ -match "^foo$" -or $_ -match "^foo\s*$" }
Report "Plain keys unaffected (echo foo outputs foo)" ($null -ne $hasFoo)

psmux kill-session -t "alt_test3" 2>$null
Kill-All

###############################################################################
# SUMMARY
###############################################################################
Write-Host "`n================================================================" -ForegroundColor Cyan
Write-Host " Results: $pass passed, $fail failed" -ForegroundColor $(if ($fail -eq 0) { "Green" } else { "Red" })
Write-Host "================================================================`n" -ForegroundColor Cyan

if ($fail -gt 0) { exit 1 }
exit 0
