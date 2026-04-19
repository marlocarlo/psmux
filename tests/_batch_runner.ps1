#!/usr/bin/env pwsh
# Streamlined test runner - runs tests inline with timeout via job
param(
    [string[]]$Tests,
    [int]$Timeout = 120,
    [string]$ResultsFile = "$env:TEMP\psmux_batch_results.csv",
    [string]$TestList = ""
)

# Support comma-separated test list string
if ($TestList -ne "") {
    $Tests = $TestList -split ','
}

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$passed = 0; $failed = 0; $timedOut = 0; $errors = @()

if (-not (Test-Path $ResultsFile)) {
    "Test,Status,Duration,Passes,Fails" | Set-Content $ResultsFile -Encoding UTF8
}

foreach ($testName in $Tests) {
    $testFile = "tests\$testName.ps1"
    if (-not (Test-Path $testFile)) { Write-Host "  SKIP $testName (not found)" -ForegroundColor DarkGray; continue }
    
    Write-Host "$testName " -NoNewline -ForegroundColor Yellow
    
    # Cleanup between tests
    & $PSMUX kill-server 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    Get-Process psmux -EA SilentlyContinue | Stop-Process -Force -EA SilentlyContinue
    Start-Sleep -Milliseconds 300
    Remove-Item "$psmuxDir\*.port","$psmuxDir\*.key" -Force -EA SilentlyContinue
    
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $outFile = "$env:TEMP\psmux_out_$testName.txt"
    
    # Use System.Diagnostics.Process for reliable timeout
    $psi = [System.Diagnostics.ProcessStartInfo]::new()
    $psi.FileName = "pwsh"
    $psi.Arguments = "-NoProfile -ExecutionPolicy Bypass -File $testFile"
    $psi.WorkingDirectory = $PWD.Path
    $psi.UseShellExecute = $false
    $psi.RedirectStandardOutput = $true
    $psi.RedirectStandardError = $true
    $psi.CreateNoWindow = $true
    
    $proc = [System.Diagnostics.Process]::Start($psi)
    $stdout = $proc.StandardOutput.ReadToEndAsync()
    $stderr = $proc.StandardError.ReadToEndAsync()
    
    $exited = $proc.WaitForExit($Timeout * 1000)
    $sw.Stop()
    $durSec = [math]::Round($sw.Elapsed.TotalSeconds, 1)
    
    if (-not $exited) {
        try { $proc.Kill($true) } catch {}
        # Kill psmux children that inherited stdout handles
        Get-Process psmux -EA SilentlyContinue | Stop-Process -Force -EA SilentlyContinue
        Start-Sleep -Milliseconds 200
        Write-Host "TIMEOUT (${durSec}s)" -ForegroundColor DarkYellow
        "$testName,TIMEOUT,$durSec,0,0" | Add-Content $ResultsFile
        $timedOut++
        $proc.Dispose()
        continue
    }
    
    $exitCode = $proc.ExitCode
    # Kill psmux children that inherited stdout handles before reading output
    Get-Process psmux -EA SilentlyContinue | Stop-Process -Force -EA SilentlyContinue
    Start-Sleep -Milliseconds 200
    # Use timeout on ReadToEndAsync to avoid infinite blocking
    if (-not $stdout.Wait(5000)) { $stdout.Dispose() }
    $outputStr = if ($stdout.IsCompleted) { $stdout.Result } else { "" }
    $proc.Dispose()
    
    $passCount = ([regex]::Matches($outputStr, '\[PASS\]')).Count
    $failCount = ([regex]::Matches($outputStr, '\[FAIL\]')).Count
    $hasPanic = $outputStr -match 'panicked at'
    
    $isFail = ($failCount -gt 0) -or ($exitCode -ne 0 -and $passCount -gt 0) -or $hasPanic
    
    if ($isFail) {
        Write-Host "FAIL " -ForegroundColor Red -NoNewline
        Write-Host "(P:$passCount F:$failCount exit:$exitCode ${durSec}s)" -ForegroundColor Gray
        "$testName,FAIL,$durSec,$passCount,$failCount" | Add-Content $ResultsFile
        $failed++
        $errors += $testName
        # Show failures
        $outputStr -split "`n" | Where-Object { $_ -match '\[FAIL\]' } | Select-Object -First 5 | ForEach-Object {
            Write-Host "    $($_.Trim())" -ForegroundColor DarkRed
        }
    } else {
        Write-Host "PASS " -ForegroundColor Green -NoNewline
        Write-Host "(P:$passCount ${durSec}s)" -ForegroundColor Gray
        "$testName,PASS,$durSec,$passCount,0" | Add-Content $ResultsFile
        $passed++
    }
}

Write-Host "`n=== Batch Summary ===" -ForegroundColor Cyan
Write-Host "  Passed:  $passed" -ForegroundColor Green
Write-Host "  Failed:  $failed" -ForegroundColor $(if ($failed -gt 0) { "Red" } else { "Green" })
Write-Host "  Timeout: $timedOut" -ForegroundColor $(if ($timedOut -gt 0) { "Yellow" } else { "Green" })
if ($errors.Count -gt 0) {
    Write-Host "  Failures: $($errors -join ', ')" -ForegroundColor Red
}
