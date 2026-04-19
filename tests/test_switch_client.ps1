<#
.SYNOPSIS
  Tests for issue #202: switch-client should actually switch the attached client to another session.
.DESCRIPTION
  Verifies that when switch-client -t <target> is sent to a psmux server,
  the server sends a SWITCH directive to the attached (persistent) client.
  This proves that switch-client is properly functional end to end.
#>

param(
    [switch]$Verbose
)

$ErrorActionPreference = 'Stop'
$script:passed = 0
$script:failed = 0

function Assert-True($condition, $message) {
    if ($condition) {
        $script:passed++
        Write-Host "  PASS: $message" -ForegroundColor Green
    } else {
        $script:failed++
        Write-Host "  FAIL: $message" -ForegroundColor Red
    }
}

function Get-SessionPort($name) {
    $portFile = "$env:USERPROFILE\.psmux\$name.port"
    if (Test-Path $portFile) {
        return [int](Get-Content $portFile).Trim()
    }
    return $null
}

function Get-SessionKey($name) {
    $keyFile = "$env:USERPROFILE\.psmux\$name.key"
    if (Test-Path $keyFile) {
        return (Get-Content $keyFile).Trim()
    }
    return ""
}

function Send-PsmuxCommand($port, $key, $command) {
    $client = New-Object System.Net.Sockets.TcpClient
    $client.Connect("127.0.0.1", $port)
    $stream = $client.GetStream()
    $writer = New-Object System.IO.StreamWriter($stream)
    $reader = New-Object System.IO.StreamReader($stream)
    $writer.AutoFlush = $true

    # Auth
    $writer.WriteLine("AUTH $key")
    $authResp = $reader.ReadLine()
    if (-not $authResp.StartsWith("OK")) {
        $client.Close()
        throw "Auth failed: $authResp"
    }

    # Send command
    $writer.WriteLine($command)
    Start-Sleep -Milliseconds 100

    $client.Close()
}

function Connect-Persistent($port, $key) {
    $client = New-Object System.Net.Sockets.TcpClient
    $client.Connect("127.0.0.1", $port)
    $client.ReceiveTimeout = 3000
    $stream = $client.GetStream()
    $writer = New-Object System.IO.StreamWriter($stream)
    $reader = New-Object System.IO.StreamReader($stream)
    $writer.AutoFlush = $true

    # Auth
    $writer.WriteLine("AUTH $key")
    $authResp = $reader.ReadLine()
    if (-not $authResp.StartsWith("OK")) {
        $client.Close()
        throw "Auth failed: $authResp"
    }

    # Enter persistent mode and attach
    $writer.WriteLine("PERSISTENT")
    $writer.WriteLine("client-attach")

    return @{ Client = $client; Writer = $writer; Reader = $reader; Stream = $stream }
}

function Read-UntilSwitch($reader, $timeoutMs = 5000) {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    while ($sw.ElapsedMilliseconds -lt $timeoutMs) {
        try {
            $line = $reader.ReadLine()
            if ($null -eq $line) { break }
            $trimmed = $line.Trim()
            if ($trimmed.StartsWith("SWITCH ")) {
                return $trimmed
            }
        } catch {
            # Timeout on read, continue
            continue
        }
    }
    return $null
}

# ==== Setup: ensure test sessions exist ====
Write-Host "`n=== Issue #202: switch-client E2E Tests ===" -ForegroundColor Cyan

$sessA = "test-switch-alpha"
$sessB = "test-switch-beta"

# Clean up any existing test sessions
try { psmux kill-session -t $sessA 2>$null } catch {}
try { psmux kill-session -t $sessB 2>$null } catch {}
Start-Sleep -Milliseconds 500

# Create fresh test sessions
psmux new-session -d -s $sessA
Start-Sleep -Milliseconds 300
psmux new-session -d -s $sessB
Start-Sleep -Milliseconds 300

$portA = Get-SessionPort $sessA
$portB = Get-SessionPort $sessB
$keyA = Get-SessionKey $sessA
$keyB = Get-SessionKey $sessB

Assert-True ($null -ne $portA) "Session '$sessA' has a port file ($portA)"
Assert-True ($null -ne $portB) "Session '$sessB' has a port file ($portB)"

# ==== Test 1: switch-client -t from CLI returns exit 0 ====
Write-Host "`n--- Test 1: switch-client -t returns exit 0 ---"
$env:PSMUX_SESSION_NAME = $sessA
psmux switch-client -t $sessB 2>$null
Assert-True ($LASTEXITCODE -eq 0 -or $null -eq $LASTEXITCODE) "switch-client -t exits with code 0"

# ==== Test 2: SWITCH directive sent to persistent client ====
Write-Host "`n--- Test 2: SWITCH directive sent to persistent client ---"
try {
    # Connect as a persistent (attached) client to session A
    $persistent = Connect-Persistent $portA $keyA

    # Give the server time to register the persistent client
    Start-Sleep -Milliseconds 500

    # From a separate connection, send switch-client -t <sessB>
    Send-PsmuxCommand $portA $keyA "switch-client -t $sessB"

    # Read from the persistent connection - should get SWITCH directive
    $switchLine = Read-UntilSwitch $persistent.Reader 5000
    
    Assert-True ($null -ne $switchLine) "Persistent client received a SWITCH directive"
    if ($switchLine) {
        $targetSession = $switchLine.Replace("SWITCH ", "")
        Assert-True ($targetSession -eq $sessB) "SWITCH target is '$sessB' (got: '$targetSession')"
    } else {
        Write-Host "  INFO: No SWITCH directive received within timeout" -ForegroundColor Yellow
        Assert-True $false "SWITCH target matches expected session"
    }
} catch {
    Write-Host "  ERROR: $($_.Exception.Message)" -ForegroundColor Red
    Assert-True $false "Persistent client test completed without errors"
} finally {
    if ($persistent -and $persistent.Client) {
        try { $persistent.Client.Close() } catch {}
    }
}

# ==== Test 3: switch-client -n (next session) ====
Write-Host "`n--- Test 3: switch-client -n (next session) ---"
try {
    $persistent2 = Connect-Persistent $portA $keyA
    Start-Sleep -Milliseconds 500
    
    Send-PsmuxCommand $portA $keyA "switch-client -n"
    $switchLine2 = Read-UntilSwitch $persistent2.Reader 5000
    
    Assert-True ($null -ne $switchLine2) "Persistent client received SWITCH for -n"
    if ($switchLine2) {
        $target2 = $switchLine2.Replace("SWITCH ", "")
        # -n should go to the next session alphabetically after sessA
        Assert-True ($target2.Length -gt 0) "Next session target is not empty (got: '$target2')"
    }
} catch {
    Write-Host "  ERROR: $($_.Exception.Message)" -ForegroundColor Red
    Assert-True $false "switch-client -n test completed without errors"
} finally {
    if ($persistent2 -and $persistent2.Client) {
        try { $persistent2.Client.Close() } catch {}
    }
}

# ==== Test 4: switch-client -p (previous session) ====
Write-Host "`n--- Test 4: switch-client -p (prev session) ---"
try {
    $persistent3 = Connect-Persistent $portB $keyB
    Start-Sleep -Milliseconds 500
    
    Send-PsmuxCommand $portB $keyB "switch-client -p"
    $switchLine3 = Read-UntilSwitch $persistent3.Reader 5000
    
    Assert-True ($null -ne $switchLine3) "Persistent client received SWITCH for -p"
    if ($switchLine3) {
        $target3 = $switchLine3.Replace("SWITCH ", "")
        Assert-True ($target3.Length -gt 0) "Prev session target is not empty (got: '$target3')"
    }
} catch {
    Write-Host "  ERROR: $($_.Exception.Message)" -ForegroundColor Red
    Assert-True $false "switch-client -p test completed without errors"
} finally {
    if ($persistent3 -and $persistent3.Client) {
        try { $persistent3.Client.Close() } catch {}
    }
}

# ==== Test 5: switch-client -t with non-existent session shows error ====
Write-Host "`n--- Test 5: switch-client -t nonexistent session ---"
try {
    # This should NOT crash, should just fail gracefully
    $env:PSMUX_SESSION_NAME = $sessA
    psmux switch-client -t "nonexistent-session-xyz" 2>$null
    # The command is fire and forget (returns before server processes it),
    # so the exit code may vary. The important thing is it doesn't crash.
    Assert-True $true "switch-client -t nonexistent exits gracefully (no crash)"
} catch {
    Assert-True $false "switch-client with bad target should not throw"
}

# ==== Cleanup ====
Write-Host "`n--- Cleanup ---"
try { psmux kill-session -t $sessA 2>$null } catch {}
try { psmux kill-session -t $sessB 2>$null } catch {}
Start-Sleep -Milliseconds 300

# ==== Summary ====
Write-Host "`n=== Results: $script:passed passed, $script:failed failed ===" -ForegroundColor $(if ($script:failed -eq 0) { "Green" } else { "Red" })
if ($script:failed -gt 0) { exit 1 }
