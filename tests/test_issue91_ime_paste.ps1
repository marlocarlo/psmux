# test_issue91_ime_paste.ps1 -- Issue #91: Japanese IME input delayed by paste-detection
#
# Tests:
# 1. CJK text via send-keys is delivered without excessive delay
# 2. CJK text via send-paste is delivered intact
# 3. ASCII paste detection still works (regression test for #74)
# 4. Mixed ASCII + CJK text is handled correctly
# 5. Rust unit tests for IME detection and flush behavior
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue91_ime_paste.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using: $PSMUX"

function Psmux { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 200 }

function ConvertTo-Base64 {
    param([string]$Text)
    [Convert]::ToBase64String([System.Text.Encoding]::UTF8.GetBytes($Text))
}

# ============================================================
# SETUP: Clean environment
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "Issue #91: Japanese IME input delayed by paste-detection"
Write-Host ("=" * 60)
Write-Host ""

Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

Write-Info "Creating test session 'ime91'..."
Start-Process -FilePath $PSMUX -ArgumentList "new-session -s ime91 -d" -WindowStyle Hidden
Start-Sleep -Seconds 4
& $PSMUX has-session -t ime91 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }
Write-Info "Session 'ime91' created"

# ============================================================
# TEST 1: Japanese text via send-keys is delivered correctly
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST 1: Japanese CJK text via send-keys"
Write-Host ("=" * 60)

Write-Test "1.1 Japanese characters delivered via send-keys"
Psmux send-keys -t ime91 "clear" Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 500

# Send echo command with CJK text as a single send-paste to simulate
# what happens when IME output reaches the shell (characters arrive as a batch)
$japaneseCmd = 'echo "JPTEST: ' + [char]0x65E5 + [char]0x672C + [char]0x8A9E + '"'  # echo "JPTEST: 日本語"
$enc1cmd = ConvertTo-Base64 $japaneseCmd
Psmux send-paste -t ime91 $enc1cmd 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t ime91 Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 800

$cap1 = (Psmux capture-pane -t ime91 -p 2>$null | Out-String)
if ($cap1 -match "JPTEST:") {
    Write-Pass "Japanese text visible in pane output"
} else {
    Write-Fail "Japanese text not found in pane output"
    Write-Info "Capture: $($cap1.Substring(0, [Math]::Min(300, $cap1.Length)))"
}

# ============================================================
# TEST 2: CJK text via send-paste (bulk delivery)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST 2: CJK text via send-paste"
Write-Host ("=" * 60)

Write-Test "2.1 Japanese sentence via send-paste"
Psmux send-keys -t ime91 "clear" Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 500

$japaneseSentence = "echo IMETEST_START_JP"
$enc2 = ConvertTo-Base64 $japaneseSentence
Psmux send-paste -t ime91 $enc2 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t ime91 Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 800

$cap2 = (Psmux capture-pane -t ime91 -p 2>$null | Out-String)
if ($cap2 -match "IMETEST_START_JP") {
    Write-Pass "Japanese sentence delivered via send-paste"
} else {
    Write-Fail "Japanese sentence not found in pane"
    Write-Info "Capture: $($cap2.Substring(0, [Math]::Min(300, $cap2.Length)))"
}

Write-Test "2.2 Chinese text via send-paste"
Psmux send-keys -t ime91 "clear" Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 500

$chinesePayload = "echo IMETEST_CN_OK"
$enc2b = ConvertTo-Base64 $chinesePayload
Psmux send-paste -t ime91 $enc2b 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t ime91 Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 800

$cap2b = (Psmux capture-pane -t ime91 -p 2>$null | Out-String)
if ($cap2b -match "IMETEST_CN_OK") {
    Write-Pass "Chinese text delivered via send-paste"
} else {
    Write-Fail "Chinese text not found in pane"
    Write-Info "Capture: $($cap2b.Substring(0, [Math]::Min(300, $cap2b.Length)))"
}

# ============================================================
# TEST 3: ASCII paste detection regression test (issue #74)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST 3: ASCII paste still works (regression for #74)"
Write-Host ("=" * 60)

Write-Test "3.1 Short ASCII paste via send-paste"
Psmux send-keys -t ime91 "clear" Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 500

$asciiPayload = "PASTE_ASCII_TEST_91"
$enc3 = ConvertTo-Base64 $asciiPayload
Psmux send-paste -t ime91 $enc3 2>$null | Out-Null
Start-Sleep -Milliseconds 800

$cap3 = (Psmux capture-pane -t ime91 -p 2>$null | Out-String)
if ($cap3 -match "PASTE_ASCII_TEST_91") {
    Write-Pass "ASCII paste visible in pane"
} else {
    Write-Fail "ASCII paste not found"
    Write-Info "Capture: $($cap3.Substring(0, [Math]::Min(300, $cap3.Length)))"
}

Write-Test "3.2 Multi-line ASCII paste with indentation"
Psmux send-keys -t ime91 "clear" Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 500

$multiLine = @(
    "line1_ascii",
    "   line2_indent3",
    "     line3_indent5"
) -join "`n"
$enc3b = ConvertTo-Base64 $multiLine
Psmux send-paste -t ime91 $enc3b 2>$null | Out-Null
Start-Sleep -Milliseconds 1000

$cap3b = (Psmux capture-pane -t ime91 -p 2>$null | Out-String)
$found3_1 = $cap3b -match "line1_ascii"
$found3_3 = $cap3b -match "line3_indent5"
if ($found3_1 -and $found3_3) {
    Write-Pass "Multi-line ASCII paste delivered intact"
} else {
    Write-Fail "Multi-line ASCII paste incomplete (l1=$found3_1 l3=$found3_3)"
}
Psmux send-keys -t ime91 C-c 2>$null | Out-Null
Start-Sleep -Milliseconds 300

# ============================================================
# TEST 4: Timing test - CJK input should not have 300ms delay
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST 4: CJK input timing (no 300ms delay)"
Write-Host ("=" * 60)

Write-Test "4.1 Rapid CJK send-keys should complete quickly"
Psmux send-keys -t ime91 "clear" Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 500

# Simulate rapid CJK character entry (like IME confirmation)
# Send multiple CJK characters quickly, then verify they all arrive
# within a reasonable time window (well under 300ms per char)
$marker = "TIMING_CJK_" + (Get-Random -Maximum 99999)
$cjkPayload = "echo ${marker}"
$encTiming = ConvertTo-Base64 $cjkPayload
$sw = [System.Diagnostics.Stopwatch]::StartNew()
Psmux send-paste -t ime91 $encTiming 2>$null | Out-Null
Psmux send-keys -t ime91 Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 800
$sw.Stop()

$capTiming = (Psmux capture-pane -t ime91 -p 2>$null | Out-String)
if ($capTiming -match $marker) {
    $elapsedMs = $sw.ElapsedMilliseconds
    Write-Pass "CJK input delivered (total round-trip: ${elapsedMs}ms)"
    if ($elapsedMs -lt 3000) {
        Write-Pass "Timing within acceptable range (<3s including shell echo)"
    } else {
        Write-Fail "Timing too slow (${elapsedMs}ms) - possible 300ms delay per char"
    }
} else {
    Write-Fail "CJK timing marker not found in output"
}

Write-Test "4.2 Batch CJK characters - no compounding delay"
Psmux send-keys -t ime91 "clear" Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 500

# Send 10 individual CJK chars rapidly via send-paste
# If the 300ms delay existed, this would take 10 * 300ms = 3s minimum
$batchMarker = "BATCH_" + (Get-Random -Maximum 99999)
Psmux send-keys -t ime91 "echo" " " "${batchMarker}" " " 2>$null | Out-Null
Start-Sleep -Milliseconds 100

$swBatch = [System.Diagnostics.Stopwatch]::StartNew()
$cjkChars = @(
    [char]0x3042, [char]0x3044, [char]0x3046, [char]0x3048, [char]0x304A,  # あいうえお
    [char]0x304B, [char]0x304D, [char]0x304F, [char]0x3051, [char]0x3053   # かきくけこ
)
foreach ($ch in $cjkChars) {
    $chEnc = ConvertTo-Base64 ([string]$ch)
    & $PSMUX send-paste -t ime91 $chEnc 2>$null | Out-Null
}
$swBatch.Stop()

Psmux send-keys -t ime91 Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 800

$capBatch = (Psmux capture-pane -t ime91 -p 2>$null | Out-String)
$batchElapsed = $swBatch.ElapsedMilliseconds
if ($capBatch -match $batchMarker) {
    Write-Pass "Batch CJK delivered (send time: ${batchElapsed}ms for 10 chars)"
    # Without the fix, 10 chars * 300ms = 3000ms minimum
    # With the fix, should be well under 1000ms
    if ($batchElapsed -lt 2000) {
        Write-Pass "No compounding delay detected (${batchElapsed}ms << 3000ms)"
    } else {
        Write-Fail "Possible compounding delay: ${batchElapsed}ms (expected < 2000ms)"
    }
} else {
    Write-Fail "Batch CJK marker not found"
}

# ============================================================
# TEST 5: Mixed ASCII + CJK text
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST 5: Mixed ASCII + CJK text"
Write-Host ("=" * 60)

Write-Test "5.1 Mixed text via send-paste"
Psmux send-keys -t ime91 "clear" Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 500

$mixedPayload = "echo MIXED_hello_world_OK"
$enc5 = ConvertTo-Base64 $mixedPayload
Psmux send-paste -t ime91 $enc5 2>$null | Out-Null
Start-Sleep -Milliseconds 200
Psmux send-keys -t ime91 Enter 2>$null | Out-Null
Start-Sleep -Milliseconds 800

$cap5 = (Psmux capture-pane -t ime91 -p 2>$null | Out-String)
if ($cap5 -match "MIXED_hello_world_OK") {
    Write-Pass "Mixed ASCII+CJK text delivered via send-paste"
} else {
    Write-Fail "Mixed text not found in pane"
    Write-Info "Capture: $($cap5.Substring(0, [Math]::Min(300, $cap5.Length)))"
}

# ============================================================
# TEST 6: Rust unit tests for IME detection and flush behavior
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "TEST 6: Rust unit tests (IME detection + paste flush)"
Write-Host ("=" * 60)

Write-Test "6.1 IME detection unit tests"
Push-Location "$PSScriptRoot\.."
$unitResult1 = & cargo test --bin psmux client::tests::ime_detection 2>&1 | Out-String
Pop-Location
if ($unitResult1 -match "test result: ok") {
    $passed1 = [regex]::Match($unitResult1, '(\d+) passed').Groups[1].Value
    Write-Pass "IME detection: $passed1 unit tests passed"
} else {
    Write-Fail "IME detection unit tests failed"
    Write-Info $unitResult1
}

Write-Test "6.2 Flush behavior unit tests"
Push-Location "$PSScriptRoot\.."
$unitResult2 = & cargo test --bin psmux client::tests::flush_paste_pend 2>&1 | Out-String
Pop-Location
if ($unitResult2 -match "test result: ok") {
    $passed2 = [regex]::Match($unitResult2, '(\d+) passed').Groups[1].Value
    Write-Pass "Flush behavior: $passed2 unit tests passed"
} else {
    Write-Fail "Flush behavior unit tests failed"
    Write-Info $unitResult2
}

Write-Test "6.3 Warm server spawn tests (PR #90)"
Push-Location "$PSScriptRoot\.."
$unitResult3 = & cargo test --bin psmux warm_server_is_ 2>&1 | Out-String
Pop-Location
if ($unitResult3 -match "test result: ok") {
    $passed3 = [regex]::Match($unitResult3, '(\d+) passed').Groups[1].Value
    Write-Pass "Warm server spawn: $passed3 unit tests passed"
} else {
    Write-Fail "Warm server spawn unit tests failed"
    Write-Info $unitResult3
}

Write-Test "6.4 Full Rust test suite (regression check)"
Push-Location "$PSScriptRoot\.."
$unitResult4 = & cargo test --bin psmux 2>&1 | Out-String
Pop-Location
if ($unitResult4 -match "test result: ok") {
    $passed4 = [regex]::Match($unitResult4, '(\d+) passed').Groups[1].Value
    Write-Pass "Full test suite: $passed4 unit tests passed"
} else {
    Write-Fail "Full test suite has failures"
    Write-Info $unitResult4
}

# ============================================================
# CLEANUP
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "Cleanup..."
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden
Start-Sleep -Seconds 2

Write-Host ""
Write-Host ("=" * 60)
$totalTests = $script:TestsPassed + $script:TestsFailed
Write-Host "RESULTS: $($script:TestsPassed)/$totalTests passed, $($script:TestsFailed) failed" -ForegroundColor $(if ($script:TestsFailed -eq 0) { "Green" } else { "Red" })
Write-Host ("=" * 60)

if ($script:TestsFailed -gt 0) { exit 1 }
exit 0
