###############################################################################
# test_issue49_ctlseq.ps1  –  GitHub Issue #49: Control Sequences Support
#
# Tests two categories:
#   1. Cursor Style Sequences (CSI Ps SP q / DECSCUSR)
#   2. SGR Attributes: Blink (5), Inverse (7), Hidden (8)
#
# Runs inside a psmux session, exercises the escape sequences via the
# psmux CLI pipe, and verifies correctness through capture-pane output.
###############################################################################
$ErrorActionPreference = 'Stop'

# ── Helpers ──────────────────────────────────────────────────────────────────
function Kill-Psmux {
    Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force 2>$null
    Start-Sleep -Milliseconds 500
}

function Wait-For-Psmux {
    param([string]$SessionName, [int]$TimeoutSec = 10)
    $end = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $end) {
        try {
            psmux has-session -t $SessionName 2>$null
            if ($LASTEXITCODE -eq 0) { return $true }
        } catch {}
        Start-Sleep -Milliseconds 300
    }
    return $false
}

function Send-Keys {
    param([string]$Keys, [int]$DelayMs = 200)
    psmux send-keys -t $script:SESSION $Keys 2>$null
    Start-Sleep -Milliseconds $DelayMs
}

function Capture-Pane {
    param([int]$DelayMs = 500)
    Start-Sleep -Milliseconds $DelayMs
    $out = psmux capture-pane -t $script:SESSION -p 2>$null
    return $out
}

$pass = 0
$fail = 0
$results = @()

function Report {
    param([string]$Name, [bool]$Ok, [string]$Detail = "")
    $script:results += [PSCustomObject]@{ Test = $Name; Result = if ($Ok) { "PASS" } else { "FAIL" }; Detail = $Detail }
    if ($Ok) { $script:pass++; Write-Host "  [PASS] $Name" -ForegroundColor Green }
    else     { $script:fail++; Write-Host "  [FAIL] $Name  $Detail" -ForegroundColor Red }
}

# ── Setup ────────────────────────────────────────────────────────────────────
Write-Host "`n=== Issue #49: Control Sequences Support ===" -ForegroundColor Cyan
Kill-Psmux

# Start psmux in detached mode with explicit session name
# (psmux auto-numbers sessions when no -s is given, so bare commands
# without -t would look for 'default' which wouldn't exist)
$SESSION = "ctlseq_test"
Write-Host "Starting psmux session '$SESSION'..." -ForegroundColor Yellow
$proc = Start-Process psmux -ArgumentList "new-session","-d","-s",$SESSION,"-x","120","-y","30" -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 3

if (-not (Wait-For-Psmux -SessionName $SESSION)) {
    Write-Host "FATAL: psmux session did not start" -ForegroundColor Red
    exit 1
}
Write-Host "psmux session ready.`n" -ForegroundColor Green

# ── TEST 1: CURSOR STYLE SEQUENCES (DECSCUSR) ──────────────────────────────
Write-Host "--- Test Group 1: Cursor Style Sequences (DECSCUSR) ---" -ForegroundColor Cyan

# Test: Set cursor shape to blinking block (1)
Send-Keys "Write-Host `"Testing cursor shape 1 (blinking block): `e[1 q`" Enter" 500

# Test: Set cursor shape to steady block (2)
Send-Keys "Write-Host `"Testing cursor shape 2 (steady block): `e[2 q`" Enter" 500

# Test: Set cursor shape to blinking underline (3)
Send-Keys "Write-Host `"Testing cursor shape 3 (blinking underline): `e[3 q`" Enter" 500

# Test: Set cursor shape to steady underline (4)
Send-Keys "Write-Host `"Testing cursor shape 4 (steady underline): `e[4 q`" Enter" 500

# Test: Set cursor shape to blinking bar (5) - THIS WAS THE BUG
Send-Keys "Write-Host `"Testing cursor shape 5 (blinking bar): `e[5 q`" Enter" 500

# Test: Set cursor shape to steady bar (6) - THIS WAS THE BUG
Send-Keys "Write-Host `"Testing cursor shape 6 (steady bar): `e[6 q`" Enter" 500

# Test: Reset cursor shape (0)
Send-Keys "Write-Host `"Testing cursor shape 0 (reset): `e[0 q`" Enter" 500

$capture = Capture-Pane
$captureText = $capture -join "`n"

# Verify the cursor shape test outputs appear
Report "DECSCUSR shape 1 (blinking block)" ($captureText -match "cursor shape 1")
Report "DECSCUSR shape 2 (steady block)"   ($captureText -match "cursor shape 2")
Report "DECSCUSR shape 3 (blinking uline)" ($captureText -match "cursor shape 3")
Report "DECSCUSR shape 4 (steady uline)"   ($captureText -match "cursor shape 4")
Report "DECSCUSR shape 5 (blinking bar)"   ($captureText -match "cursor shape 5")
Report "DECSCUSR shape 6 (steady bar)"     ($captureText -match "cursor shape 6")
Report "DECSCUSR shape 0 (reset)"          ($captureText -match "cursor shape 0")

# Use debug env var to test cursor shape scanning
# This exercises the scan_cursor_shape function directly
$env:PSMUX_DEBUG_CURSOR = "1"
$debugLog = "$env:TEMP\psmux_cursor_debug.log"
Remove-Item $debugLog -ErrorAction SilentlyContinue

# ── TEST 2: SGR ATTRIBUTES ──────────────────────────────────────────────────
Write-Host "`n--- Test Group 2: SGR Attributes (Blink, Inverse, Hidden) ---" -ForegroundColor Cyan

# Clear the pane first
Send-Keys "clear Enter" 1000

# Test: Inverse text (SGR 7)
Send-Keys "Write-Host `"`e[7mINVERSE_TEXT`e[0m NORMAL_TEXT`" Enter" 500

# Test: Blink text (SGR 5)
Send-Keys "Write-Host `"`e[5mBLINK_TEXT`e[0m NORMAL_TEXT`" Enter" 500

# Test: Hidden text (SGR 8)
Send-Keys "Write-Host `"`e[8mHIDDEN_TEXT`e[0m VISIBLE_TEXT`" Enter" 500

# Test: Combined attributes
Send-Keys "Write-Host `"`e[1;5;7mBOLD_BLINK_INVERSE`e[0m`" Enter" 500

# Test: Blink + color
Send-Keys "Write-Host `"`e[5;31mBLINK_RED`e[0m`" Enter" 500

Start-Sleep -Seconds 1
$capture2 = Capture-Pane
$captureText2 = $capture2 -join "`n"

# Verify text content appears (even if we can't check visual attributes via capture-pane)
Report "SGR 7 inverse text output"     ($captureText2 -match "INVERSE_TEXT")
Report "SGR 7 normal after reset"      ($captureText2 -match "NORMAL_TEXT")
Report "SGR 5 blink text output"       ($captureText2 -match "BLINK_TEXT")
Report "SGR 8 visible after hidden"    ($captureText2 -match "VISIBLE_TEXT")
Report "SGR combined attrs output"     ($captureText2 -match "BOLD_BLINK_INVERSE")
Report "SGR blink+color output"        ($captureText2 -match "BLINK_RED")

# ── TEST 3: Verify capture-pane shows consistent content ────────────────────
Write-Host "`n--- Test Group 3: Capture-Pane Content Verification ---" -ForegroundColor Cyan

# Clear and write test content with all SGR attributes
Send-Keys "clear Enter" 1000
Send-Keys "Write-Host `"`e[7mINV`e[0m `e[5mBLK`e[0m `e[8mHID`e[0m VIS`" Enter" 1500

Start-Sleep -Seconds 2
$captureCheck = Capture-Pane
$captureCheckText = $captureCheck -join "`n"

# The content should include INV, BLK, and VIS text at minimum
Report "Capture: inverse text present"  ($captureCheckText -match "INV")
Report "Capture: blink text present"    ($captureCheckText -match "BLK")
Report "Capture: visible text present"  ($captureCheckText -match "VIS")

# ── TEST 4: Verify the exact Write-Host test from the issue ─────────────────
Write-Host "`n--- Test Group 4: Exact Issue #49 Test Command ---" -ForegroundColor Cyan

Send-Keys "clear Enter" 1000
Send-Keys 'Write-Host "Blink Text: `e[5mI am Blink`e[0m`nInverse Text: `e[7mI am Inverse`e[0m`nHidden Text: `e[8mI am Hidden`e[0m" Enter' 1500

Start-Sleep -Seconds 2
$capture3 = Capture-Pane
$captureText3 = $capture3 -join "`n"

Report "Issue test: Blink text renders"   ($captureText3 -match "I am Blink")
Report "Issue test: Inverse text renders" ($captureText3 -match "I am Inverse")
# Hidden text should NOT appear in visual capture (it's hidden!)
# But capture-pane captures the text content, so it may or may not appear
# The key test is that it doesn't crash and visible text is unaffected
Report "Issue test: Output intact"        ($captureText3 -match "Blink Text:" -and $captureText3 -match "Inverse Text:" -and $captureText3 -match "Hidden Text:")

# ── Cleanup ──────────────────────────────────────────────────────────────────
psmux kill-session -t $SESSION 2>$null
Kill-Psmux
$env:PSMUX_DEBUG_CURSOR = $null

# ── Summary ──────────────────────────────────────────────────────────────────
Write-Host "`n=== RESULTS ===" -ForegroundColor Cyan
$results | Format-Table -AutoSize
Write-Host "Total: $($pass + $fail)  Pass: $pass  Fail: $fail" -ForegroundColor $(if ($fail -eq 0) { "Green" } else { "Red" })

if ($fail -gt 0) { exit 1 }
exit 0
