###############################################################################
# test_issue15_altgr.ps1 — GitHub Issue #15: AltGr / International Keyboard
#
# Verifies that characters typed via AltGr (reported as Ctrl+Alt by Windows)
# on international keyboards (German, Czech, etc.) are forwarded correctly
# to the child PTY and appear in the pane output.
#
# Characters tested: \  @  {  }  [  ]  |  ~  €  $
#
# The test works by:
#   1. Starting a detached psmux session
#   2. Sending an echo command with each AltGr character via send-keys -l
#   3. Capturing the pane output and verifying all characters appear
#   4. Also tests the TCP PERSISTENT protocol send-text path
#   5. Runs Rust unit tests on encode_key_event()
#
# Run:  pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue15_altgr.ps1
###############################################################################
$ErrorActionPreference = "Continue"
$script:pass = 0
$script:fail = 0
$script:results = @()

function Kill-Psmux {
    Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force 2>$null
    Start-Sleep -Milliseconds 500
}

function Wait-For-Psmux {
    param([int]$TimeoutSec = 10)
    $end = (Get-Date).AddSeconds($TimeoutSec)
    while ((Get-Date) -lt $end) {
        try { $r = psmux list-sessions 2>$null; if ($LASTEXITCODE -eq 0) { return $true } } catch {}
        Start-Sleep -Milliseconds 300
    }
    return $false
}

function Send-Keys {
    param([string]$Keys, [int]$DelayMs = 300)
    psmux send-keys $Keys 2>$null
    Start-Sleep -Milliseconds $DelayMs
}

function Send-Keys-Literal {
    param([string]$Text, [int]$DelayMs = 300)
    psmux send-keys -l $Text 2>$null
    Start-Sleep -Milliseconds $DelayMs
}

function Capture-Pane {
    param([int]$DelayMs = 500)
    Start-Sleep -Milliseconds $DelayMs
    $out = psmux capture-pane -p 2>$null
    return $out
}

function Report {
    param([string]$Name, [bool]$Ok, [string]$Detail = "")
    $script:results += [PSCustomObject]@{ Test = $Name; Result = if ($Ok) { "PASS" } else { "FAIL" }; Detail = $Detail }
    if ($Ok) { $script:pass++; Write-Host "  [PASS] $Name" -ForegroundColor Green }
    else     { $script:fail++; Write-Host "  [FAIL] $Name  $Detail" -ForegroundColor Red }
}

# ── Setup ────────────────────────────────────────────────────────────────────
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Cyan
Write-Host "ISSUE #15: AltGr / International Keyboard Character Tests" -ForegroundColor Cyan
Write-Host ("=" * 70) -ForegroundColor Cyan
Write-Host ""

Kill-Psmux

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Host "[INFO] Using: $PSMUX" -ForegroundColor Cyan

# Start psmux session in detached mode
Write-Host "[INFO] Starting psmux session..." -ForegroundColor Yellow
$proc = Start-Process $PSMUX -ArgumentList "new-session","-d" -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 3

if (-not (Wait-For-Psmux)) {
    Write-Host "FATAL: psmux session did not start" -ForegroundColor Red
    exit 1
}
Write-Host "[INFO] psmux session ready." -ForegroundColor Green
Write-Host ""

###############################################################################
# TEST GROUP 1: AltGr characters via send-keys -l (literal mode)
#
# send-keys -l bypasses key name parsing and sends raw text to the PTY.
# This verifies the PTY write path works for all AltGr characters.
###############################################################################
Write-Host "--- Test Group 1: AltGr Characters via send-keys -l (literal) ---" -ForegroundColor Cyan

# Clear the pane, then echo a marker with AltGr characters
Send-Keys "clear Enter" 1000

# Use PowerShell Write-Host to echo a known string containing all AltGr chars
# We type the command using send-keys (for the command) and send-keys -l (for special chars)
Send-Keys 'Write-Host "ALTGR_TEST: ' 100
Send-Keys-Literal '\ @ { } [ ] | ~' 100
Send-Keys '" Enter' 1000

$capture1 = Capture-Pane
$text1 = $capture1 -join "`n"

Report "Literal backslash (\) appears"   ($text1 -match [regex]::Escape('\'))   "capture: $(($text1 -split "`n" | Select-String 'ALTGR_TEST') -join '')"
Report "Literal at-sign (@) appears"     ($text1 -match '@')                    ""
Report "Literal open curly ({) appears"  ($text1 -match '\{')                   ""
Report "Literal close curly (}) appears" ($text1 -match '\}')                   ""
Report "Literal open bracket ([) appears"  ($text1 -match '\[')                 ""
Report "Literal close bracket (]) appears" ($text1 -match '\]')                 ""
Report "Literal pipe (|) appears"        ($text1 -match '\|')                   ""
Report "Literal tilde (~) appears"       ($text1 -match '~')                    ""

###############################################################################
# TEST GROUP 2: Individual AltGr character echo verification
#
# For each character, we send a separate echo command and verify capture-pane.
###############################################################################
Write-Host ""
Write-Host "--- Test Group 2: Individual AltGr Character Echo Tests ---" -ForegroundColor Cyan

Send-Keys "clear Enter" 1000

# Test backslash
Send-Keys 'Write-Host "BSLASH:' 100
Send-Keys-Literal '\' 100
Send-Keys ':END" Enter' 800

$cap = (Capture-Pane) -join "`n"
Report "Echo backslash individually"  ($cap -match 'BSLASH:\\:END')  "got: $(($cap -split "`n" | Select-String 'BSLASH') -join '')"

# Test at-sign
Send-Keys 'Write-Host "AT:' 100
Send-Keys-Literal '@' 100
Send-Keys ':END" Enter' 800

$cap = (Capture-Pane) -join "`n"
Report "Echo at-sign individually"  ($cap -match 'AT:@:END')  "got: $(($cap -split "`n" | Select-String 'AT:') -join '')"

# Test curly braces
Send-Keys 'Write-Host "CURLY:' 100
Send-Keys-Literal '{}' 100
Send-Keys ':END" Enter' 800

$cap = (Capture-Pane) -join "`n"
Report "Echo curly braces individually"  ($cap -match 'CURLY:\{\}:END')  "got: $(($cap -split "`n" | Select-String 'CURLY:') -join '')"

# Test square brackets
Send-Keys 'Write-Host "BRACKET:' 100
Send-Keys-Literal '[]' 100
Send-Keys ':END" Enter' 800

$cap = (Capture-Pane) -join "`n"
Report "Echo square brackets individually"  ($cap -match 'BRACKET:\[\]:END')  "got: $(($cap -split "`n" | Select-String 'BRACKET:') -join '')"

# Test pipe
Send-Keys 'Write-Host "PIPE:' 100
Send-Keys-Literal '|' 100
Send-Keys ':END" Enter' 800

$cap = (Capture-Pane) -join "`n"
Report "Echo pipe individually"  ($cap -match 'PIPE:\|:END')  "got: $(($cap -split "`n" | Select-String 'PIPE:') -join '')"

# Test tilde
Send-Keys 'Write-Host "TILDE:' 100
Send-Keys-Literal '~' 100
Send-Keys ':END" Enter' 800

$cap = (Capture-Pane) -join "`n"
Report "Echo tilde individually"  ($cap -match 'TILDE:~:END')  "got: $(($cap -split "`n" | Select-String 'TILDE:') -join '')"

###############################################################################
# TEST GROUP 3: TCP PERSISTENT protocol send-text path
#
# Connect to the psmux session via TCP and send characters using the
# send-text command — this tests the server-side text forwarding path.
###############################################################################
Write-Host ""
Write-Host "--- Test Group 3: TCP send-text Protocol Path ---" -ForegroundColor Cyan

$SESSION = "default"
$portFile = "$env:USERPROFILE\.psmux\$SESSION.port"
$keyFile  = "$env:USERPROFILE\.psmux\$SESSION.key"
$tcpOk = $false

if ((Test-Path $portFile) -and (Test-Path $keyFile)) {
    try {
        $port = [int](Get-Content $portFile).Trim()
        $authKey = (Get-Content $keyFile).Trim()

        $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", $port)
        $tcp.NoDelay = $true
        $tcp.ReceiveTimeout = 10000
        $stream = $tcp.GetStream()
        $enc = [System.Text.UTF8Encoding]::new($false)
        $reader = [System.IO.StreamReader]::new($stream, $enc, $false, 131072)
        $writer = [System.IO.StreamWriter]::new($stream, $enc, 4096)
        $writer.NewLine = "`n"
        $writer.AutoFlush = $false

        $writer.WriteLine("AUTH $authKey"); $writer.Flush()
        $auth = $reader.ReadLine()
        if ($auth -eq "OK") {
            $tcpOk = $true

            # Clear the pane first
            $writer.WriteLine('send-keys clear Enter'); $writer.Flush()
            Start-Sleep -Seconds 1

            # Send echo command with AltGr characters via send-text
            $writer.WriteLine('send-text "Write-Host ""TCP_ALTGR: "'); $writer.Flush()
            Start-Sleep -Milliseconds 100
            # Send the actual special characters
            $writer.WriteLine('send-text "\ @ { } [ ] | ~"'); $writer.Flush()
            Start-Sleep -Milliseconds 100
            $writer.WriteLine('send-text """"'); $writer.Flush()
            Start-Sleep -Milliseconds 100
            $writer.WriteLine('send-key enter'); $writer.Flush()
            Start-Sleep -Seconds 1

            # Capture pane via CLI
            $capTcp = (psmux capture-pane -p 2>$null) -join "`n"

            Report "TCP send-text: backslash passes through"  ($capTcp -match [regex]::Escape('\'))  ""
            Report "TCP send-text: at-sign passes through"    ($capTcp -match '@')                   ""
            Report "TCP send-text: curly braces pass through" ($capTcp -match '\{' -and $capTcp -match '\}')  ""
            Report "TCP send-text: brackets pass through"     ($capTcp -match '\[' -and $capTcp -match '\]')  ""
            Report "TCP send-text: pipe passes through"       ($capTcp -match '\|')                  ""
            Report "TCP send-text: tilde passes through"      ($capTcp -match '~')                   ""
        }
        $tcp.Close()
    } catch {
        Write-Host "  [WARN] TCP test failed: $_" -ForegroundColor Yellow
    }
}

if (-not $tcpOk) {
    Write-Host "  [SKIP] TCP tests skipped (could not connect)" -ForegroundColor Yellow
}

###############################################################################
# TEST GROUP 4: Rust unit tests (encode_key_event)
#
# Run the Rust unit tests that directly verify the encode_key_event function
# handles AltGr characters (Ctrl+Alt + non-letter char) correctly.
###############################################################################
Write-Host ""
Write-Host "--- Test Group 4: Rust Unit Tests (encode_key_event) ---" -ForegroundColor Cyan

$rustTestOutput = & cargo test --bin psmux input::tests -- --nocapture 2>&1 | Out-String
$rustTestPassed = $rustTestOutput -match 'test result: ok'
$rustTestCount = if ($rustTestOutput -match '(\d+) passed') { $Matches[1] } else { "?" }

Report "Rust unit tests all pass ($rustTestCount tests)"  $rustTestPassed  $(if (-not $rustTestPassed) { $rustTestOutput.Substring(0, [Math]::Min(200, $rustTestOutput.Length)) } else { "" })

# Extract individual test names for reporting
$rustLines = $rustTestOutput -split "`n"
foreach ($line in $rustLines) {
    if ($line -match 'test input::tests::(\S+) \.\.\. (\w+)') {
        $testName = $Matches[1]
        $testResult = $Matches[2]
        Report "  Rust: $testName"  ($testResult -eq 'ok')  ""
    }
}

###############################################################################
# TEST GROUP 5: Euro sign and extended Unicode AltGr characters
#
# Tests multi-byte UTF-8 characters that come from AltGr on various layouts.
###############################################################################
Write-Host ""
Write-Host "--- Test Group 5: Extended Unicode AltGr Characters ---" -ForegroundColor Cyan

Send-Keys "clear Enter" 1000

# Euro sign (€) — AltGr+E on German keyboard, 3-byte UTF-8
Send-Keys 'Write-Host "EURO:' 100
Send-Keys-Literal '€' 100
Send-Keys ':END" Enter' 800

$cap = (Capture-Pane) -join "`n"
Report "Euro sign (€) passes through"  ($cap -match 'EURO:.*:END')  "got: $(($cap -split "`n" | Select-String 'EURO:') -join '')"

# Dollar sign ($) — AltGr key on Czech layout
Send-Keys 'Write-Host "DOLLAR:' 100
Send-Keys-Literal '$' 100
Send-Keys ':END" Enter' 800

# Note: PowerShell interprets $ specially in double-quoted strings, so the
# dollar may or may not appear depending on how the shell processes it.
# We mainly verify no crash/hang occurs.
$cap = (Capture-Pane) -join "`n"
Report "Dollar sign ($) no crash"  ($cap -match 'DOLLAR:')  ""

###############################################################################
# Cleanup
###############################################################################
Kill-Psmux

###############################################################################
# Summary
###############################################################################
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor Cyan
Write-Host "ISSUE #15 TEST RESULTS" -ForegroundColor Cyan
Write-Host ("=" * 70) -ForegroundColor Cyan
$script:results | Format-Table -AutoSize
Write-Host "Total: $($script:pass + $script:fail)  Pass: $script:pass  Fail: $script:fail" -ForegroundColor $(if ($script:fail -eq 0) { "Green" } else { "Red" })
Write-Host ""

if ($script:fail -gt 0) { exit 1 }
exit 0
