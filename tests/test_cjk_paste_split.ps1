#!/usr/bin/env pwsh
###############################################################################
# test_cjk_paste_split.ps1 — Regression test for Issue #103
#
# Pasting CJK text (>100 UTF-8 bytes) then splitting panes should NOT crash
# the session.
###############################################################################
$ErrorActionPreference = "Continue"

$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) { $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe" }
if (-not (Test-Path $PSMUX)) { $PSMUX = (Get-Command psmux -ErrorAction SilentlyContinue).Source }
if (-not $PSMUX -or -not (Test-Path $PSMUX)) { Write-Error "psmux binary not found"; exit 1 }

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
Write-Host " Issue #103: CJK paste + split pane crash test" -ForegroundColor Cyan
Write-Host "================================================================`n" -ForegroundColor Cyan

# CJK text from the issue: 34 chars, 102 UTF-8 bytes
$cjkText = "然后然后然后然后然后然后然后然后然后然后然后然后然后然后然后然后然后"
$cjkBytes = [System.Text.Encoding]::UTF8.GetByteCount($cjkText)
Write-Host "  CJK text: $($cjkText.Length) chars, $cjkBytes UTF-8 bytes" -ForegroundColor Gray

# Even longer CJK text (200+ bytes)
$longCjk = $cjkText + $cjkText + $cjkText
$longBytes = [System.Text.Encoding]::UTF8.GetByteCount($longCjk)
Write-Host "  Long CJK: $($longCjk.Length) chars, $longBytes UTF-8 bytes" -ForegroundColor Gray

###############################################################################
# TEST 1: Paste CJK text, then split horizontally
###############################################################################
Write-Host "`n--- TEST 1: Paste CJK + split-window -h ---" -ForegroundColor Yellow
Kill-All

& $PSMUX new-session -d -s "cjk_test1" -x 120 -y 30 2>$null
Start-Sleep -Seconds 2

# Paste CJK text
& $PSMUX send-keys -t "cjk_test1" "$cjkText" Enter 2>$null
Start-Sleep -Milliseconds 500

# Split pane
& $PSMUX split-window -t "cjk_test1" -h 2>$null
Start-Sleep -Seconds 1

# Check if session survived
& $PSMUX has-session -t "cjk_test1" 2>$null
$alive = ($LASTEXITCODE -eq 0)
Report "Paste CJK + split-h: session survives" $alive

if ($alive) {
    # Paste again in the new pane
    & $PSMUX send-keys -t "cjk_test1" "$cjkText" Enter 2>$null
    Start-Sleep -Milliseconds 500

    # Split again
    & $PSMUX split-window -t "cjk_test1" -v 2>$null
    Start-Sleep -Seconds 1

    & $PSMUX has-session -t "cjk_test1" 2>$null
    Report "Paste CJK + split-v: session survives" ($LASTEXITCODE -eq 0)
}

& $PSMUX kill-session -t "cjk_test1" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 2: Paste LONG CJK text (300+ bytes), then split
###############################################################################
Write-Host "`n--- TEST 2: Paste long CJK ($longBytes bytes) + split ---" -ForegroundColor Yellow
Kill-All

& $PSMUX new-session -d -s "cjk_test2" -x 80 -y 24 2>$null
Start-Sleep -Seconds 2

& $PSMUX send-keys -t "cjk_test2" "$longCjk" Enter 2>$null
Start-Sleep -Milliseconds 500

& $PSMUX split-window -t "cjk_test2" -h 2>$null
Start-Sleep -Seconds 1

& $PSMUX has-session -t "cjk_test2" 2>$null
Report "Long CJK paste + split: session survives" ($LASTEXITCODE -eq 0)

& $PSMUX kill-session -t "cjk_test2" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 3: Repeat paste-split cycle multiple times (the user's exact repro)
###############################################################################
Write-Host "`n--- TEST 3: Repeated paste + split cycle ---" -ForegroundColor Yellow
Kill-All

& $PSMUX new-session -d -s "cjk_test3" -x 120 -y 40 2>$null
Start-Sleep -Seconds 2

$cycleOk = $true
for ($i = 1; $i -le 4; $i++) {
    # Paste CJK text
    & $PSMUX send-keys -t "cjk_test3" "$cjkText" Enter 2>$null
    Start-Sleep -Milliseconds 300

    # Split
    if ($i % 2 -eq 1) {
        & $PSMUX split-window -t "cjk_test3" -h 2>$null
    } else {
        & $PSMUX split-window -t "cjk_test3" -v 2>$null
    }
    Start-Sleep -Seconds 1

    & $PSMUX has-session -t "cjk_test3" 2>$null
    if ($LASTEXITCODE -ne 0) {
        Report "Cycle ${i}: session crashed" $false
        $cycleOk = $false
        break
    }
    Write-Host "    Cycle ${i}: paste + split OK" -ForegroundColor DarkGray
}

if ($cycleOk) {
    Report "4x paste+split cycle: session survives" $true
}

& $PSMUX kill-session -t "cjk_test3" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 4: Narrow pane + CJK (worst case: wide char at edge)
###############################################################################
Write-Host "`n--- TEST 4: Narrow pane (20 cols) + CJK paste ---" -ForegroundColor Yellow
Kill-All

& $PSMUX new-session -d -s "cjk_test4" -x 20 -y 24 2>$null
Start-Sleep -Seconds 2

& $PSMUX send-keys -t "cjk_test4" "$cjkText" Enter 2>$null
Start-Sleep -Milliseconds 500

& $PSMUX split-window -t "cjk_test4" -h 2>$null
Start-Sleep -Seconds 1

& $PSMUX has-session -t "cjk_test4" 2>$null
Report "Narrow pane CJK + split: session survives" ($LASTEXITCODE -eq 0)

& $PSMUX kill-session -t "cjk_test4" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 5: Mixed ASCII + CJK
###############################################################################
Write-Host "`n--- TEST 5: Mixed ASCII + CJK paste + split ---" -ForegroundColor Yellow
Kill-All

$mixedText = "Hello World " + $cjkText + " Test 123 " + $cjkText
& $PSMUX new-session -d -s "cjk_test5" -x 100 -y 30 2>$null
Start-Sleep -Seconds 2

& $PSMUX send-keys -t "cjk_test5" "$mixedText" Enter 2>$null
Start-Sleep -Milliseconds 500

& $PSMUX split-window -t "cjk_test5" -h 2>$null
Start-Sleep -Seconds 1

& $PSMUX has-session -t "cjk_test5" 2>$null
Report "Mixed ASCII+CJK + split: session survives" ($LASTEXITCODE -eq 0)

& $PSMUX kill-session -t "cjk_test5" 2>$null
Start-Sleep -Milliseconds 500

###############################################################################
# TEST 6: Verify no crash log was generated
###############################################################################
Write-Host "`n--- TEST 6: No crash log ---" -ForegroundColor Yellow

$crashLog = "$env:USERPROFILE\.psmux\crash.log"
# Remove any pre-existing crash log
Remove-Item $crashLog -Force -ErrorAction SilentlyContinue 2>$null

# Run the crash scenario one more time
Kill-All
& $PSMUX new-session -d -s "cjk_crash_check" -x 80 -y 24 2>$null
Start-Sleep -Seconds 2

& $PSMUX send-keys -t "cjk_crash_check" "$longCjk" Enter 2>$null
Start-Sleep -Milliseconds 500
& $PSMUX split-window -t "cjk_crash_check" -h 2>$null
Start-Sleep -Seconds 1
& $PSMUX send-keys -t "cjk_crash_check" "$cjkText" Enter 2>$null
Start-Sleep -Milliseconds 500
& $PSMUX split-window -t "cjk_crash_check" -v 2>$null
Start-Sleep -Seconds 1

& $PSMUX has-session -t "cjk_crash_check" 2>$null
$sessionAlive = ($LASTEXITCODE -eq 0)

if (Test-Path $crashLog) {
    $crashContent = Get-Content $crashLog -Raw
    Report "No crash log generated" $false "crash.log: $($crashContent.Substring(0, [Math]::Min(200, $crashContent.Length)))"
} else {
    Report "No crash log generated" $sessionAlive
}

& $PSMUX kill-session -t "cjk_crash_check" 2>$null
Kill-All

###############################################################################
# SUMMARY
###############################################################################
Write-Host "`n================================================================" -ForegroundColor Cyan
Write-Host " Results: $pass passed, $fail failed" -ForegroundColor $(if ($fail -eq 0) { "Green" } else { "Red" })
Write-Host "================================================================`n" -ForegroundColor Cyan

if ($fail -gt 0) { exit 1 }
exit 0
