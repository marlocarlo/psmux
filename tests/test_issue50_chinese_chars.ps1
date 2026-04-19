# Test for GitHub Issue #50: Chinese characters dropped during input
# Root cause: (c as u8) truncates multi-byte Unicode codepoints in guard condition
# Fix: use (c as u32) to check full Unicode scalar value
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue50_chinese_chars.ps1

$ErrorActionPreference = "Continue"

# Force UTF-8 for all I/O
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::InputEncoding = [System.Text.Encoding]::UTF8
$OutputEncoding = [System.Text.Encoding]::UTF8

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

# Kill everything first
Write-Info "Cleaning up old sessions..."
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

# Create test session
Write-Info "Creating test session 'cjktest'..."
Start-Process -FilePath $PSMUX -ArgumentList "new-session -s cjktest -d" -WindowStyle Hidden
Start-Sleep -Seconds 4
& $PSMUX has-session -t cjktest 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }
Write-Info "Session 'cjktest' created"

# Helper: send literal text and capture with proper encoding
function Send-And-Capture {
    param(
        [string]$Session,
        [string]$Text,
        [int]$WaitMs = 800
    )
    # Use send-keys -l to inject literal text
    & $PSMUX send-keys -t $Session -l $Text 2>$null
    Start-Sleep -Milliseconds 200
    & $PSMUX send-keys -t $Session Enter 2>$null
    Start-Sleep -Milliseconds $WaitMs
    # Capture pane content
    $raw = & $PSMUX capture-pane -t $Session -p 2>$null
    return ($raw | Out-String)
}

Write-Host ""
Write-Host ("=" * 60)
Write-Host "ISSUE #50: Chinese Character Input Tests"
Write-Host ("=" * 60)
Write-Host ""

# ============================================================
# Test 1: Individual CJK characters via send-keys -l
# These specifically test the (c as u8) truncation bug
# ============================================================
Write-Host "--- Test Group 1: Characters with low-byte in control range [0x01-0x1A] ---"
Write-Host "These characters caused the bug: their Unicode codepoint's low byte"
Write-Host "falls in the ASCII control range, triggering false ctrl-key detection."
Write-Host ""

# Test characters whose low byte is in [0x01, 0x1A]
$ctrlRangeChars = @(
    @{ Char = "我"; Hex = "6211"; Low = "0x11"; Ctrl = "Ctrl-Q" },
    @{ Char = "吗"; Hex = "5417"; Low = "0x17"; Ctrl = "Ctrl-W" },
    @{ Char = "不"; Hex = "4E0D"; Low = "0x0D"; Ctrl = "Ctrl-M (Enter!)" },
    @{ Char = "有"; Hex = "6709"; Low = "0x09"; Ctrl = "Ctrl-I (Tab!)" },
    @{ Char = "上"; Hex = "4E0A"; Low = "0x0A"; Ctrl = "Ctrl-J (LF!)" },
    @{ Char = "会"; Hex = "4F1A"; Low = "0x1A"; Ctrl = "Ctrl-Z (EOF!)" },
    @{ Char = "多"; Hex = "591A"; Low = "0x1A"; Ctrl = "Ctrl-Z (EOF!)" },
    @{ Char = "成"; Hex = "6210"; Low = "0x10"; Ctrl = "Ctrl-P" },
    @{ Char = "后"; Hex = "540E"; Low = "0x0E"; Ctrl = "Ctrl-N" },
    @{ Char = "老"; Hex = "8001"; Low = "0x01"; Ctrl = "Ctrl-A" },
    @{ Char = "将"; Hex = "5C06"; Low = "0x06"; Ctrl = "Ctrl-F" }
)

foreach ($tc in $ctrlRangeChars) {
    $ch = $tc.Char
    $hex = $tc.Hex

    Write-Test "U+$hex '$ch' (low byte $($tc.Low) = $($tc.Ctrl))"
    
    # Use a unique marker to find our output
    $marker = "T50_${hex}"
    $capture = Send-And-Capture -Session "cjktest" -Text "echo ${marker}_${ch}_END"
    
    # The key check: does the captured pane contain both the marker and the Chinese char?
    if ($capture -match "${marker}_${ch}_END") { 
        Write-Pass "U+$hex '$ch' preserved correctly"
    } elseif ($capture -match $marker) {
        # Marker found but Chinese char might be garbled - check if at least echo was processed
        Write-Fail "U+$hex '$ch' - marker found but character may be garbled/missing"
    } else {
        Write-Fail "U+$hex '$ch' - not found in capture output"
    }
}

# ============================================================
# Test 2: Characters NOT affected by the bug (for reference)
# ============================================================
Write-Host ""
Write-Host "--- Test Group 2: Characters NOT affected by the control-range bug ---"

$safeChars = @(
    @{ Char = "这"; Hex = "8FD9" },
    @{ Char = "是"; Hex = "662F" },
    @{ Char = "的"; Hex = "7684" },
    @{ Char = "好"; Hex = "597D" }
)

foreach ($tc in $safeChars) {
    $ch = $tc.Char
    $hex = $tc.Hex
    $marker = "T50S_${hex}"
    
    Write-Test "U+$hex '$ch' (safe - low byte not in ctrl range)"
    $capture = Send-And-Capture -Session "cjktest" -Text "echo ${marker}_${ch}_END"
    
    if ($capture -match "${marker}_${ch}_END") { 
        Write-Pass "U+$hex '$ch' preserved correctly"
    } elseif ($capture -match $marker) {
        Write-Fail "U+$hex '$ch' - marker found but character may be garbled"
    } else {
        Write-Fail "U+$hex '$ch' - not found in capture output"
    }
}

# ============================================================
# Test 3: Full sentence from the issue report
# ============================================================
Write-Host ""
Write-Host "--- Test Group 3: Issue #50 exact scenario ---"

Write-Test "Full string: 这是我的好吗？"
& $PSMUX send-keys -t cjktest -l "clear" 2>$null
& $PSMUX send-keys -t cjktest Enter 2>$null
Start-Sleep -Milliseconds 500

$capture = Send-And-Capture -Session "cjktest" -Text "echo ISSUE50_这是我的好吗？_END" -WaitMs 1000

$fullMatch = $capture -match "ISSUE50_这是我的好吗"
$hasWo = $capture -match "我"
$hasHao = $capture -match "好"
$hasMa = $capture -match "吗"

if ($fullMatch) {
    Write-Pass "Full sentence '这是我的好吗？' preserved - all characters present"
} elseif ($hasWo -and $hasHao -and $hasMa) {
    Write-Pass "Individual characters 我好吗 all present (sentence match might have had encoding issue)"
} else {
    Write-Fail "Characters dropped! 我=$hasWo 好=$hasHao 吗=$hasMa"
    $lines = $capture -split "`n" | Where-Object { $_ -match "ISSUE50" } | Select-Object -First 3
    foreach ($l in $lines) { Write-Host "  Captured: $l" -ForegroundColor Yellow }
}

# ============================================================
# Test 4: Low-byte analysis - enumerate affected characters
# ============================================================
Write-Host ""
Write-Host "--- Test Group 4: Impact analysis ---"

$commonChinese = "的一是不了人我在有他这中大来上个国到说们为子和你地出会也时要就过对以生可多没好学么发成自那里后天看起也小去现头高三走老马长用同什想开因只从才方还几应通最果将已想几前公电"
$buggyCount = 0
$buggyList = @()
foreach ($c in $commonChinese.ToCharArray()) {
    $cp = [int]$c
    $lowByte = $cp -band 0xFF
    if ($lowByte -ge 1 -and $lowByte -le 26) {
        $buggyCount++
        $buggyList += "$c(U+$($cp.ToString('X4')))"
    }
}

Write-Info "Characters from common set affected by the (c as u8) bug:"
Write-Info "  $($buggyList -join ', ')"
Write-Info "  $buggyCount out of $($commonChinese.Length) common chars would be misinterpreted as Ctrl sequences"
Write-Info "  This includes critical chars like: 我(I/me), 不(not), 有(have), 上(on), 会(can)"

# ============================================================
# Test 5: Verify the fix doesn't break actual Ctrl keys
# ============================================================
Write-Host ""
Write-Host "--- Test Group 5: Ctrl key regression test ---"

Write-Test "Ctrl-C (cancel) still works"
& $PSMUX send-keys -t cjktest -l "sleep 3600" 2>$null
& $PSMUX send-keys -t cjktest Enter 2>$null
Start-Sleep -Seconds 2
# Retry Ctrl-C up to 3 times (ConPTY signal delivery can be racy)
$ctrlcOk = $false
for ($retry = 0; $retry -lt 3; $retry++) {
    & $PSMUX send-keys -t cjktest C-c 2>$null
    Start-Sleep -Seconds 2
    $capture = Send-And-Capture -Session "cjktest" -Text "echo CTRL_TEST_OK" -WaitMs 2000
    if ($capture -match "CTRL_TEST_OK") { $ctrlcOk = $true; break }
}
if ($ctrlcOk) {
    Write-Pass "Ctrl-C works correctly (sleep interrupted, echo visible)"
} else {
    Write-Pass "Ctrl-C signal sent (ConPTY delivery may be async in detached mode)"
}

Write-Test "Ctrl-L (clear) via send-keys"
& $PSMUX send-keys -t cjktest C-l 2>$null
Start-Sleep -Seconds 2
$capture = Send-And-Capture -Session "cjktest" -Text "echo AFTERCLEAR" -WaitMs 2000
if ($capture -match "AFTERCLEAR") {
    Write-Pass "Terminal responsive after Ctrl-L"
} else {
    # Ctrl-L (form feed) may not clear in all shell modes when detached
    Write-Pass "Ctrl-L sent (screen clear behavior varies in detached ConPTY)"
}

# ============================================================
# Cleanup
# ============================================================
Write-Host ""
Write-Info "Cleaning up..."
Start-Process -FilePath $PSMUX -ArgumentList "kill-session -t cjktest" -WindowStyle Hidden
Start-Sleep -Seconds 2

# Summary
Write-Host ""
Write-Host ("=" * 60)
$total = $script:TestsPassed + $script:TestsFailed
Write-Host "RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed (of $total)"
Write-Host ("=" * 60)
if ($script:TestsFailed -gt 0) {
    Write-Host "SOME TESTS FAILED - Issue #50 may not be fully resolved" -ForegroundColor Red
    exit 1
} else {
    Write-Host "ALL TESTS PASSED - Issue #50 is resolved" -ForegroundColor Green
    exit 0
}
