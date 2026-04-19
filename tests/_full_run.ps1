#!/usr/bin/env pwsh
# Full sequential test runner with result tracking
param(
    [int]$TimeoutSec = 300,
    [string]$Filter = "test_*",
    [switch]$SkipPerf,
    [switch]$SkipWSL,
    [switch]$SkipStress,
    [int]$StartFrom = 0
)

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$resultsFile = "$env:TEMP\psmux_full_run_results.csv"
$summaryFile = "$env:TEMP\psmux_full_run_summary.txt"

# Get all test files
$allTests = Get-ChildItem "$PSScriptRoot\$Filter.ps1" -EA Stop |
    Where-Object { $_.Name -ne '_full_run.ps1' -and $_.Name -ne '_run_batch3.ps1' -and $_.Name -ne 'run_all_tests.ps1' -and $_.Name -ne 'run_batch_fast.ps1' -and $_.Name -ne 'run_fmt_test.ps1' } |
    Sort-Object Name

# Apply filters
if ($SkipPerf) {
    $allTests = $allTests | Where-Object { $_.Name -notmatch 'perf|bench|latency|speed' }
}
if ($SkipWSL) {
    $allTests = $allTests | Where-Object { $_.Name -notmatch 'wsl' }
}
if ($SkipStress) {
    $allTests = $allTests | Where-Object { $_.Name -notmatch 'stress' }
}

# Always skip tests requiring external deps (Claude Code, NSIS, WSL-only)
$allTests = $allTests | Where-Object { 
    $_.Name -notmatch 'agent_teams|claude_agent|claude_compat|claude_cursor|claude_mouse|nsis_installer|destructive|battle_test|test_all$' 
}

$total = $allTests.Count
Write-Host "=" * 70 -ForegroundColor Cyan
Write-Host "PSMUX FULL TEST RUN: $total test suites" -ForegroundColor Cyan
Write-Host "Timeout per test: ${TimeoutSec}s" -ForegroundColor Cyan
Write-Host "Results: $resultsFile" -ForegroundColor Cyan
Write-Host "=" * 70 -ForegroundColor Cyan

# Initialize results
"Test,Status,Duration,ExitCode,Notes" | Set-Content $resultsFile -Encoding UTF8

$passed = 0
$failed = 0
$skipped = 0
$errors = @()
$startTime = Get-Date

for ($i = $StartFrom; $i -lt $allTests.Count; $i++) {
    $test = $allTests[$i]
    $testName = $test.BaseName
    $num = $i + 1
    
    Write-Host "`n[$num/$total] $testName " -ForegroundColor Yellow -NoNewline
    
    # Clean up between tests
    & $PSMUX kill-server 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    Get-Process psmux -EA SilentlyContinue | Stop-Process -Force -EA SilentlyContinue
    Start-Sleep -Milliseconds 500
    Remove-Item "$psmuxDir\*.port" -Force -EA SilentlyContinue
    Remove-Item "$psmuxDir\*.key" -Force -EA SilentlyContinue
    
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $outFile = "$env:TEMP\psmux_test_out_$testName.txt"
    
    try {
        $proc = Start-Process -FilePath "pwsh" -ArgumentList "-NoProfile","-ExecutionPolicy","Bypass","-File",$test.FullName `
            -WorkingDirectory (Split-Path $test.FullName -Parent | Split-Path -Parent) `
            -RedirectStandardOutput $outFile -RedirectStandardError "$env:TEMP\psmux_test_err.txt" `
            -NoNewWindow -PassThru
        
        $exited = $proc.WaitForExit($TimeoutSec * 1000)
        $sw.Stop()
        $durSec = [math]::Round($sw.Elapsed.TotalSeconds, 1)
        
        if (-not $exited) {
            $proc.Kill()
            Write-Host "TIMEOUT (${durSec}s)" -ForegroundColor DarkYellow
            "$testName,TIMEOUT,$durSec,-1,Exceeded ${TimeoutSec}s" | Add-Content $resultsFile
            $skipped++
        } else {
            $outputStr = if (Test-Path $outFile) { Get-Content $outFile -Raw -EA SilentlyContinue } else { "" }
            if (-not $outputStr) { $outputStr = "" }
            $exitCode = $proc.ExitCode
            
            # Parse output for pass/fail counts
            $passCount = ([regex]::Matches($outputStr, '\[PASS\]')).Count
            $failCount = ([regex]::Matches($outputStr, '\[FAIL\]')).Count
            
            # Check for failure indicators
            $hasFails = $failCount -gt 0 -or $exitCode -ne 0 -or $outputStr -match 'panicked at'
            
            if ($hasFails -and $passCount -eq 0 -and $failCount -eq 0) {
                # No structured output, check exit code only
                if ($exitCode -eq 0) { $hasFails = $false }
            }
            
            if ($hasFails) {
                Write-Host "FAIL " -ForegroundColor Red -NoNewline
                Write-Host "(P:$passCount F:$failCount exit:$exitCode ${durSec}s)" -ForegroundColor Gray
                "$testName,FAIL,$durSec,$failCount,$passCount passed / $failCount failed / exit $exitCode" | Add-Content $resultsFile
                $failed++
                $errors += @{ Name=$testName; Output=$outputStr; Fails=$failCount; Passes=$passCount; Exit=$exitCode }
                
                # Show failure lines
                $outputStr -split "`n" | Where-Object { $_ -match '\[FAIL\]' } | Select-Object -First 5 | ForEach-Object {
                    Write-Host "    $($_.Trim())" -ForegroundColor DarkRed
                }
            } else {
                Write-Host "PASS " -ForegroundColor Green -NoNewline
                Write-Host "(P:$passCount ${durSec}s)" -ForegroundColor Gray
                "$testName,PASS,$durSec,0,$passCount passed" | Add-Content $resultsFile
                $passed++
            }
        }
        Remove-Item $outFile -Force -EA SilentlyContinue
        Remove-Item "$env:TEMP\psmux_test_err.txt" -Force -EA SilentlyContinue
    } catch {
        $sw.Stop()
        $durSec = [math]::Round($sw.Elapsed.TotalSeconds, 1)
        Write-Host "ERROR (${durSec}s): $_" -ForegroundColor Red
        "$testName,ERROR,$durSec,-1,$($_.ToString())" | Add-Content $resultsFile
        $failed++
        $errors += @{ Name=$testName; Output=$_.ToString(); Fails=1; Passes=0 }
    }
}

# Final cleanup
& $PSMUX kill-server 2>&1 | Out-Null
Get-Process psmux -EA SilentlyContinue | Stop-Process -Force -EA SilentlyContinue

$elapsed = (Get-Date) - $startTime
Write-Host "`n" + ("=" * 70) -ForegroundColor Cyan
Write-Host "FINAL RESULTS" -ForegroundColor Cyan
Write-Host ("=" * 70) -ForegroundColor Cyan
Write-Host "  Total:   $total" 
Write-Host "  Passed:  $passed" -ForegroundColor Green
Write-Host "  Failed:  $failed" -ForegroundColor $(if ($failed -gt 0) { "Red" } else { "Green" })
Write-Host "  Timeout: $skipped" -ForegroundColor $(if ($skipped -gt 0) { "Yellow" } else { "Green" })
Write-Host "  Time:    $([math]::Round($elapsed.TotalMinutes, 1)) minutes"
Write-Host ""

if ($errors.Count -gt 0) {
    Write-Host "FAILED TESTS:" -ForegroundColor Red
    foreach ($e in $errors) {
        Write-Host "  - $($e.Name) (P:$($e.Passes) F:$($e.Fails))" -ForegroundColor Red
    }
}

# Write summary
@"
PSMUX Full Test Run Summary
Date: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')
Total: $total | Passed: $passed | Failed: $failed | Timeout: $skipped
Duration: $([math]::Round($elapsed.TotalMinutes, 1)) minutes

Failed Tests:
$($errors | ForEach-Object { "  - $($_.Name) (P:$($_.Passes) F:$($_.Fails))" } | Out-String)
"@ | Set-Content $summaryFile -Encoding UTF8

Write-Host "`nSummary saved to: $summaryFile" -ForegroundColor DarkGray
Write-Host "Full results CSV: $resultsFile" -ForegroundColor DarkGray
exit $failed
