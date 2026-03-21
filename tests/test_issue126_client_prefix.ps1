#!/usr/bin/env pwsh
# test_issue126_client_prefix.ps1 — Verify client_prefix flag updates when prefix key is pressed
# https://github.com/psmux/psmux/issues/126

$ErrorActionPreference = 'Continue'
$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"

$script:TestsPassed = 0
$script:TestsFailed = 0
function Write-Pass($msg) { Write-Host "  PASS: $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  FAIL: $msg" -ForegroundColor Red;   $script:TestsFailed++ }
function Write-Test($msg) { Write-Host "`n[$($script:TestsPassed + $script:TestsFailed + 1)] $msg" -ForegroundColor Cyan }

$SESSION = "prefix_flag_$(Get-Random)"

# Cleanup any leftover session
Start-Process -FilePath $PSMUX -ArgumentList "kill-session -t $SESSION" -WindowStyle Hidden -ErrorAction SilentlyContinue
Start-Sleep -Seconds 1

# Create a detached session
Write-Host "`nCreating session '$SESSION'..." -ForegroundColor Yellow
Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $SESSION -d" -WindowStyle Hidden
Start-Sleep -Seconds 3

$hasSession = & $PSMUX has-session -t $SESSION 2>&1
if ($LASTEXITCODE -ne 0) {
    Write-Host "ERROR: Cannot create session '$SESSION'" -ForegroundColor Red
    exit 1
}

function Psmux { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 300 }
function Fmt { param($f) (& $PSMUX display-message -t $SESSION -p "$f" 2>&1 | Out-String).Trim() }

# ---------------------------------------------------------------------------
# Test 1: client_prefix is 0 by default
# ---------------------------------------------------------------------------
Write-Test "client_prefix is 0 by default"
$val = Fmt '#{client_prefix}'
if ($val -eq "0") { Write-Pass "client_prefix = $val" }
else              { Write-Fail "Expected '0', got '$val'" }

# ---------------------------------------------------------------------------
# Test 2: conditional format shows 'NORM' when prefix not active
# ---------------------------------------------------------------------------
Write-Test "Conditional shows 'NORM' when prefix not active"
$val = Fmt '#{?client_prefix,PREFIX,NORM}'
if ($val -eq "NORM") { Write-Pass "conditional = $val" }
else                  { Write-Fail "Expected 'NORM', got '$val'" }

# ---------------------------------------------------------------------------
# Test 3: After prefix-begin, client_prefix is 1
# ---------------------------------------------------------------------------
Write-Test "client_prefix is 1 after prefix-begin"
# Send prefix-begin directly to set the flag on the server
$port = (Get-Content "$env:USERPROFILE\.psmux\$SESSION.port" -ErrorAction SilentlyContinue)
$key = (Get-Content "$env:USERPROFILE\.psmux\$SESSION.key" -ErrorAction SilentlyContinue)
if ($port -and $key) {
    # Send prefix-begin via raw TCP using PERSISTENT mode (like the real client does)
    try {
        $tcp = New-Object System.Net.Sockets.TcpClient("127.0.0.1", [int]$port)
        $stream = $tcp.GetStream()
        $stream.ReadTimeout = 5000
        $writer = New-Object System.IO.StreamWriter($stream)
        $reader = New-Object System.IO.StreamReader($stream)
        $writer.AutoFlush = $true
        $writer.WriteLine("AUTH $key")
        $auth_response = $reader.ReadLine()
        if ($auth_response -ne "OK") {
            Write-Fail "Auth failed: $auth_response"
            $tcp.Close()
            Psmux kill-session -t $SESSION | Out-Null
            exit 1
        }
        # Enter persistent mode so the connection stays alive for multiple commands
        $writer.WriteLine("PERSISTENT")
        Start-Sleep -Milliseconds 200

        $writer.WriteLine("prefix-begin")
        Start-Sleep -Milliseconds 300
        $val = Fmt '#{client_prefix}'
        if ($val -eq "1") { Write-Pass "client_prefix = $val" }
        else              { Write-Fail "Expected '1', got '$val'" }

        # -------------------------------------------------------------------
        # Test 4: conditional format shows 'PREFIX' when prefix active
        # -------------------------------------------------------------------
        Write-Test "Conditional shows 'PREFIX' when prefix active"
        $val = Fmt '#{?client_prefix,PREFIX,NORM}'
        if ($val -eq "PREFIX") { Write-Pass "conditional = $val" }
        else                    { Write-Fail "Expected 'PREFIX', got '$val'" }

        # -------------------------------------------------------------------
        # Test 5: client_key_table is 'prefix' when prefix active
        # -------------------------------------------------------------------
        Write-Test "client_key_table is 'prefix' when prefix active"
        $val = Fmt '#{client_key_table}'
        if ($val -eq "prefix") { Write-Pass "client_key_table = $val" }
        else                    { Write-Fail "Expected 'prefix', got '$val'" }

        # -------------------------------------------------------------------
        # Test 6: After prefix-end, client_prefix goes back to 0
        # -------------------------------------------------------------------
        Write-Test "client_prefix is 0 after prefix-end"
        $writer.WriteLine("prefix-end")
        Start-Sleep -Milliseconds 200
        $val = Fmt '#{client_prefix}'
        if ($val -eq "0") { Write-Pass "client_prefix = $val" }
        else              { Write-Fail "Expected '0', got '$val'" }

        # -------------------------------------------------------------------
        # Test 7: conditional reverts to 'NORM' after prefix-end
        # -------------------------------------------------------------------
        Write-Test "Conditional reverts to 'NORM' after prefix-end"
        $val = Fmt '#{?client_prefix,PREFIX,NORM}'
        if ($val -eq "NORM") { Write-Pass "conditional = $val" }
        else                  { Write-Fail "Expected 'NORM', got '$val'" }

        # -------------------------------------------------------------------
        # Test 8: client_key_table reverts to 'root' after prefix-end
        # -------------------------------------------------------------------
        Write-Test "client_key_table reverts to 'root' after prefix-end"
        $val = Fmt '#{client_key_table}'
        if ($val -eq "root") { Write-Pass "client_key_table = $val" }
        else                  { Write-Fail "Expected 'root', got '$val'" }

        # -------------------------------------------------------------------
        # Test 9: Rapid prefix toggle — correct on each
        # -------------------------------------------------------------------
        Write-Test "Rapid prefix toggle — flag flips correctly"
        $allCorrect = $true
        for ($i = 0; $i -lt 6; $i++) {
            if ($i % 2 -eq 0) { $writer.WriteLine("prefix-begin") }
            else               { $writer.WriteLine("prefix-end") }
            Start-Sleep -Milliseconds 150
            $val = Fmt '#{client_prefix}'
            $expected = if ($i % 2 -eq 0) { "1" } else { "0" }
            if ($val -ne $expected) {
                Write-Fail "Toggle $($i+1): expected '$expected', got '$val'"
                $allCorrect = $false
            }
        }
        if ($allCorrect) { Write-Pass "All 6 rapid toggles correct" }
        # Final cleanup: make sure prefix ends
        $writer.WriteLine("prefix-end")
        Start-Sleep -Milliseconds 100

        $tcp.Close()
    } catch {
        Write-Fail "TCP connection failed: $_"
    }
} else {
    Write-Fail "Cannot find session port/key files for direct protocol test"
}

# ---------------------------------------------------------------------------
# Cleanup
# ---------------------------------------------------------------------------
Psmux kill-session -t $SESSION | Out-Null
Write-Host "`n========================================" -ForegroundColor Yellow
Write-Host "Results: $($script:TestsPassed) passed, $($script:TestsFailed) failed" -ForegroundColor $(if ($script:TestsFailed -gt 0) { 'Red' } else { 'Green' })
exit $script:TestsFailed
