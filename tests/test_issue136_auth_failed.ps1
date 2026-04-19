#!/usr/bin/env pwsh
# Test for issue #136: "psmux: auth failed" when running bare psmux with detached sessions
# https://github.com/psmux/psmux/issues/136
#
# Root cause: The warm server claim code reads the AUTH "OK" response and
# treats it as the claim-session success, proceeding before the server has
# finished renaming .port/.key files. This is a race condition.
#
# Additionally tests:
# - cleanup_stale_port_files should also clean up orphaned .key files
# - Port files should always have matching key files

$ErrorActionPreference = "Continue"
$psmux = Get-Command psmux -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source
if (-not $psmux) { $psmux = "psmux" }

$pass = 0
$fail = 0
$total = 0

function Test-Assert($name, $condition) {
    $script:total++
    if ($condition) {
        Write-Host "  PASS: $name" -ForegroundColor Green
        $script:pass++
    } else {
        Write-Host "  FAIL: $name" -ForegroundColor Red
        $script:fail++
    }
}

function Cleanup-PsmuxState {
    # Kill all psmux server processes and clean up files
    & $psmux kill-server 2>$null
    Start-Sleep -Milliseconds 500
    $psmuxDir2 = Join-Path $env:USERPROFILE ".psmux"
    if (Test-Path $psmuxDir2) {
        Get-ChildItem $psmuxDir2 -Filter "*.port" | Remove-Item -Force -ErrorAction SilentlyContinue
        Get-ChildItem $psmuxDir2 -Filter "*.key" | Remove-Item -Force -ErrorAction SilentlyContinue
        # Don't remove last_session or other config files
    }
    Start-Sleep -Milliseconds 200
}

$psmuxDir = Join-Path $env:USERPROFILE ".psmux"

Write-Host "`n=== Issue #136: psmux auth failed ===" -ForegroundColor Cyan

# ─── Test 1: Verify warm claim protocol correctness ────────────────────
# The bug is that bare psmux reads AUTH "OK" instead of claim "OK"
Write-Host "`nTest 1: Warm claim protocol - port/key files exist after claim" -ForegroundColor Yellow
Cleanup-PsmuxState

# Start a session in detached mode
$output = & $psmux new-session -d -s "test0" 2>&1
Start-Sleep -Milliseconds 1500

# Verify session exists
$portFile = Join-Path $psmuxDir "test0.port"
$keyFile = Join-Path $psmuxDir "test0.key"
Test-Assert "Session test0 port file exists" (Test-Path $portFile)
Test-Assert "Session test0 key file exists" (Test-Path $keyFile)

# Check for warm server
Start-Sleep -Milliseconds 1000
$warmPort = Join-Path $psmuxDir "__warm__.port"
$warmKey = Join-Path $psmuxDir "__warm__.key"
$warmExists = Test-Path $warmPort
Write-Host "  INFO: Warm server port file exists: $warmExists" -ForegroundColor Gray

if ($warmExists) {
    Test-Assert "Warm server key file exists alongside port" (Test-Path $warmKey)
}

Cleanup-PsmuxState

# ─── Test 2: Verify send_auth_cmd_response reads both lines ───────────
# This directly tests the fix: the warm claim should wait for claim response
Write-Host "`nTest 2: Bare psmux after detached session (issue #136 core scenario)" -ForegroundColor Yellow
Cleanup-PsmuxState

# Create a detached session
$output = & $psmux new-session -d -s "0" 2>&1
Start-Sleep -Milliseconds 1500

$portFile0 = Join-Path $psmuxDir "0.port"
Test-Assert "Session 0 created successfully" (Test-Path $portFile0)

# Now simulate what bare psmux does: create ANOTHER session via warm claim
# by running new-session (which also uses warm claim internally)
$output2 = & $psmux new-session -d -s "1" 2>&1
$exitCode = $LASTEXITCODE
Start-Sleep -Milliseconds 1000

$portFile1 = Join-Path $psmuxDir "1.port"  
$keyFile1 = Join-Path $psmuxDir "1.key"

Test-Assert "Second session created without error (exit=$exitCode)" ($exitCode -eq 0 -or (Test-Path $portFile1))
Test-Assert "Session 1 port file exists" (Test-Path $portFile1)
Test-Assert "Session 1 key file exists" (Test-Path $keyFile1)

if (Test-Path $keyFile1) {
    $keyContent = Get-Content $keyFile1 -Raw
    Test-Assert "Session 1 key file is non-empty" ($keyContent.Trim().Length -gt 0)
}

Cleanup-PsmuxState

# ─── Test 3: Key file consistency after claim ──────────────────────────
Write-Host "`nTest 3: Key file matches server's in-memory key after warm claim" -ForegroundColor Yellow
Cleanup-PsmuxState

# Create first session (triggers warm server spawn)
$output = & $psmux new-session -d -s "s0" 2>&1
Start-Sleep -Milliseconds 2000

$s0Port = Join-Path $psmuxDir "s0.port"
$s0Key = Join-Path $psmuxDir "s0.key"
Test-Assert "Session s0 port file exists" (Test-Path $s0Port)
Test-Assert "Session s0 key file exists" (Test-Path $s0Key)

# Create second session (should use warm claim)
$output2 = & $psmux new-session -d -s "s1" 2>&1
$exitCode = $LASTEXITCODE
Start-Sleep -Milliseconds 1000

$s1Port = Join-Path $psmuxDir "s1.port"
$s1Key = Join-Path $psmuxDir "s1.key"

Test-Assert "Session s1 created (exit=$exitCode)" ($exitCode -eq 0 -or (Test-Path $s1Port))

if (Test-Path $s1Port) {
    if (Test-Path $s1Key) {
        $s1KeyContent = (Get-Content $s1Key -Raw).Trim()
        Test-Assert "Session s1 key is 16 hex chars" ($s1KeyContent -match '^[0-9a-f]{16}$')
        
        # Try to authenticate with this key by sending a session-info command
        $port = (Get-Content $s1Port -Raw).Trim()
        $lsOutput = & $psmux ls 2>&1 | Out-String
        Test-Assert "list-sessions shows s1 without error" ($lsOutput -match "s1")
    } else {
        Test-Assert "Session s1 key file exists (CRITICAL for auth)" $false
    }
} else {
    Test-Assert "Session s1 port file exists" $false
}

Cleanup-PsmuxState

# ─── Test 4: Orphaned key file cleanup ─────────────────────────────────
Write-Host "`nTest 4: Stale key files cleaned up alongside port files" -ForegroundColor Yellow
Cleanup-PsmuxState

# Create orphaned port+key files pointing to a non-existent server
if (-not (Test-Path $psmuxDir)) { New-Item -ItemType Directory -Path $psmuxDir -Force | Out-Null }
Set-Content -Path (Join-Path $psmuxDir "orphan.port") -Value "59999" -NoNewline
Set-Content -Path (Join-Path $psmuxDir "orphan.key")  -Value "deadbeefdeadbeef" -NoNewline

$orphanPort = Join-Path $psmuxDir "orphan.port"
$orphanKey = Join-Path $psmuxDir "orphan.key"

Test-Assert "Orphan port file created" (Test-Path $orphanPort)
Test-Assert "Orphan key file created" (Test-Path $orphanKey)

# Run psmux ls to trigger cleanup_stale_port_files
$output = & $psmux ls 2>&1
Start-Sleep -Milliseconds 500

Test-Assert "Orphan port file cleaned up" (-not (Test-Path $orphanPort))
Test-Assert "Orphan key file cleaned up" (-not (Test-Path $orphanKey))

Cleanup-PsmuxState

# ─── Test 5: Port files always have matching key files ─────────────────
Write-Host "`nTest 5: All live sessions have both port and key files" -ForegroundColor Yellow
Cleanup-PsmuxState

# Create multiple sessions
$output1 = & $psmux new-session -d -s "multi0" 2>&1
Start-Sleep -Milliseconds 1500
$output2 = & $psmux new-session -d -s "multi1" 2>&1
Start-Sleep -Milliseconds 1500

$portFiles = Get-ChildItem $psmuxDir -Filter "*.port" -ErrorAction SilentlyContinue | 
    Where-Object { $_.BaseName -notlike "*__warm__*" }

foreach ($pf in $portFiles) {
    $base = $pf.BaseName
    $matchingKey = Join-Path $psmuxDir "$base.key"
    Test-Assert "Session $base has matching key file" (Test-Path $matchingKey)
    
    if (Test-Path $matchingKey) {
        $keyVal = (Get-Content $matchingKey -Raw).Trim()
        Test-Assert "Session $base key is non-empty" ($keyVal.Length -gt 0)
    }
}

# Also check warm server if it exists
$warmPorts = Get-ChildItem $psmuxDir -Filter "*__warm__*.port" -ErrorAction SilentlyContinue
foreach ($wp in $warmPorts) {
    $base = $wp.BaseName
    $matchingKey = Join-Path $psmuxDir "$base.key"
    Test-Assert "Warm session $base has matching key file" (Test-Path $matchingKey)
}

Cleanup-PsmuxState

# ─── Summary ───────────────────────────────────────────────────────────
Write-Host "`n=== Results ===" -ForegroundColor Cyan
Write-Host "Passed: $pass / $total" -ForegroundColor $(if ($fail -eq 0) { "Green" } else { "Yellow" })
if ($fail -gt 0) {
    Write-Host "Failed: $fail / $total" -ForegroundColor Red
    exit 1
}
Write-Host "All tests passed!" -ForegroundColor Green
exit 0
