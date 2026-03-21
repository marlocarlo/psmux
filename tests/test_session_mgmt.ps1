$ErrorActionPreference = "Continue"
$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) { $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe" }
$results = @()

function Add-Result {
    param($TestNum, $Name, $ExitCode, $Pass, $Output)
    $script:results += [PSCustomObject]@{
        Test     = $TestNum
        Name     = $Name
        ExitCode = $ExitCode
        Pass     = if ($Pass) { "PASS" } else { "FAIL" }
        Output   = $Output
    }
}

# Cleanup first
Write-Host ">>> Killing any existing psmux processes..."
taskkill /f /im psmux.exe 2>$null
Start-Sleep -Seconds 3
Write-Host ">>> Cleanup done."
Write-Host ""

# ========== TEST 1 ==========
Write-Host "=== TEST 1: new-session -s test1 -d ==="
$out = & $PSMUX new-session -s test1 -d 2>&1 | Out-String
$ec = $LASTEXITCODE
Write-Host "  Exit code: $ec | Output: [$($out.Trim())]"
Add-Result 1 "new-session -s test1 -d" $ec ($ec -eq 0) $out.Trim()
Start-Sleep -Seconds 3

# ========== TEST 2 ==========
Write-Host "=== TEST 2: has-session -t test1 ==="
$out = & $PSMUX has-session -t test1 2>&1 | Out-String
$ec = $LASTEXITCODE
Write-Host "  Exit code: $ec | Output: [$($out.Trim())]"
Add-Result 2 "has-session -t test1" $ec ($ec -eq 0) $out.Trim()
Start-Sleep -Seconds 2

# ========== TEST 3 ==========
Write-Host "=== TEST 3: has-session -t nonexistent (expect non-zero) ==="
$out = & $PSMUX has-session -t nonexistent 2>&1 | Out-String
$ec = $LASTEXITCODE
Write-Host "  Exit code: $ec | Output: [$($out.Trim())]"
Add-Result 3 "has-session -t nonexistent (expect fail)" $ec ($ec -ne 0) $out.Trim()
Start-Sleep -Seconds 2

# ========== TEST 4 ==========
Write-Host "=== TEST 4: list-sessions (expect test1) ==="
$out = & $PSMUX list-sessions 2>&1 | Out-String
$ec = $LASTEXITCODE
$hasTest1 = $out -match "test1"
Write-Host "  Exit code: $ec | Contains test1: $hasTest1 | Output: [$($out.Trim())]"
Add-Result 4 "list-sessions contains test1" $ec ($ec -eq 0 -and $hasTest1) $out.Trim()
Start-Sleep -Seconds 2

# ========== TEST 5 ==========
Write-Host "=== TEST 5: new-session -s test2 -d ==="
$out = & $PSMUX new-session -s test2 -d 2>&1 | Out-String
$ec = $LASTEXITCODE
Write-Host "  Exit code: $ec | Output: [$($out.Trim())]"
Add-Result 5 "new-session -s test2 -d" $ec ($ec -eq 0) $out.Trim()
Start-Sleep -Seconds 3

# ========== TEST 6 ==========
Write-Host "=== TEST 6: list-sessions (expect test1 and test2) ==="
$out = & $PSMUX list-sessions 2>&1 | Out-String
$ec = $LASTEXITCODE
$hasTest1 = $out -match "test1"
$hasTest2 = $out -match "test2"
Write-Host "  Exit code: $ec | Has test1: $hasTest1 | Has test2: $hasTest2 | Output: [$($out.Trim())]"
Add-Result 6 "list-sessions contains test1+test2" $ec ($ec -eq 0 -and $hasTest1 -and $hasTest2) $out.Trim()
Start-Sleep -Seconds 2

# ========== TEST 7 ==========
Write-Host "=== TEST 7: rename-session -t test1 renamed1 ==="
$out = & $PSMUX rename-session -t test1 renamed1 2>&1 | Out-String
$ec = $LASTEXITCODE
Write-Host "  Exit code: $ec | Output: [$($out.Trim())]"
Add-Result 7 "rename-session -t test1 renamed1" $ec ($ec -eq 0) $out.Trim()
Start-Sleep -Seconds 2

# ========== TEST 8 ==========
Write-Host "=== TEST 8: has-session -t renamed1 ==="
$out = & $PSMUX has-session -t renamed1 2>&1 | Out-String
$ec = $LASTEXITCODE
Write-Host "  Exit code: $ec | Output: [$($out.Trim())]"
Add-Result 8 "has-session -t renamed1" $ec ($ec -eq 0) $out.Trim()
Start-Sleep -Seconds 2

# ========== TEST 9 ==========
Write-Host "=== TEST 9: has-session -t test1 (expect fail - was renamed) ==="
$out = & $PSMUX has-session -t test1 2>&1 | Out-String
$ec = $LASTEXITCODE
Write-Host "  Exit code: $ec | Output: [$($out.Trim())]"
Add-Result 9 "has-session -t test1 (expect fail after rename)" $ec ($ec -ne 0) $out.Trim()
Start-Sleep -Seconds 2

# ========== TEST 10 ==========
Write-Host "=== TEST 10: kill-session -t test2 ==="
$out = & $PSMUX kill-session -t test2 2>&1 | Out-String
$ec = $LASTEXITCODE
Write-Host "  Exit code: $ec | Output: [$($out.Trim())]"
Add-Result 10 "kill-session -t test2" $ec ($ec -eq 0) $out.Trim()
Start-Sleep -Seconds 2

# ========== TEST 11 ==========
Write-Host "=== TEST 11: has-session -t test2 (expect fail - was killed) ==="
$out = & $PSMUX has-session -t test2 2>&1 | Out-String
$ec = $LASTEXITCODE
Write-Host "  Exit code: $ec | Output: [$($out.Trim())]"
Add-Result 11 "has-session -t test2 (expect fail after kill)" $ec ($ec -ne 0) $out.Trim()
Start-Sleep -Seconds 2

# ========== TEST 12 ==========
Write-Host "=== TEST 12: kill-session -t renamed1 (cleanup) ==="
$out = & $PSMUX kill-session -t renamed1 2>&1 | Out-String
$ec = $LASTEXITCODE
Write-Host "  Exit code: $ec | Output: [$($out.Trim())]"
Add-Result 12 "kill-session -t renamed1" $ec ($ec -eq 0) $out.Trim()
Start-Sleep -Seconds 3

# Kill server so we start clean for test 13
taskkill /f /im psmux.exe 2>$null
Start-Sleep -Seconds 3

# ========== TEST 13: Create 3 sessions then kill-server ==========
Write-Host "=== TEST 13: Create 3 sessions + kill-server ==="
$out1 = & $PSMUX new-session -s multi1 -d 2>&1 | Out-String; $ec1 = $LASTEXITCODE
Start-Sleep -Seconds 3
$out2 = & $PSMUX new-session -s multi2 -d 2>&1 | Out-String; $ec2 = $LASTEXITCODE
Start-Sleep -Seconds 3
$out3 = & $PSMUX new-session -s multi3 -d 2>&1 | Out-String; $ec3 = $LASTEXITCODE
Start-Sleep -Seconds 3

# Verify all 3 exist
$lsOut = & $PSMUX list-sessions 2>&1 | Out-String; $lsEc = $LASTEXITCODE
$hasAll = ($lsOut -match "multi1") -and ($lsOut -match "multi2") -and ($lsOut -match "multi3")
Write-Host "  Created 3 sessions: ec1=$ec1 ec2=$ec2 ec3=$ec3 | list-sessions ec=$lsEc | hasAll=$hasAll"
Write-Host "  list-sessions output: [$($lsOut.Trim())]"

$createPass = ($ec1 -eq 0) -and ($ec2 -eq 0) -and ($ec3 -eq 0) -and ($lsEc -eq 0) -and $hasAll
Add-Result 13 "create 3 sessions (multi1,multi2,multi3)" $lsEc $createPass $lsOut.Trim()
Start-Sleep -Seconds 2

# Now kill-server
Write-Host "  Sending kill-server..."
$ksOut = & $PSMUX kill-server 2>&1 | Out-String; $ksEc = $LASTEXITCODE
Write-Host "  kill-server exit: $ksEc | Output: [$($ksOut.Trim())]"
Start-Sleep -Seconds 3

# ========== TEST 14: Verify all sessions gone after kill-server ==========
Write-Host "=== TEST 14: Verify all sessions gone after kill-server ==="
$out = & $PSMUX list-sessions 2>&1 | Out-String
$ec = $LASTEXITCODE
Write-Host "  Exit code: $ec | Output: [$($out.Trim())]"
# After kill-server, list-sessions should fail (no server) or return empty
$noSessions = ($ec -ne 0) -or (-not ($out -match "multi"))
Add-Result 14 "all sessions gone after kill-server" $ec $noSessions $out.Trim()

# ========== SUMMARY ==========
Write-Host ""
Write-Host "=" * 70
Write-Host "SESSION MANAGEMENT TEST RESULTS"
Write-Host "=" * 70
$results | Format-Table -Property Test, Pass, Name, ExitCode, Output -AutoSize -Wrap
Write-Host ""
$passCount = ($results | Where-Object { $_.Pass -eq "PASS" }).Count
$failCount = ($results | Where-Object { $_.Pass -eq "FAIL" }).Count
Write-Host "TOTAL: $($results.Count) tests | PASSED: $passCount | FAILED: $failCount"
if ($failCount -gt 0) {
    Write-Host ""
    Write-Host "FAILED TESTS:"
    $results | Where-Object { $_.Pass -eq "FAIL" } | ForEach-Object {
        Write-Host "  Test $($_.Test): $($_.Name) - ExitCode=$($_.ExitCode) Output=[$($_.Output)]"
    }
}

# Final cleanup
taskkill /f /im psmux.exe 2>$null
