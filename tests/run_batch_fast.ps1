# run_batch_fast.ps1 - Run ALL test suites, skip ONLY those requiring visible TUI window
param([switch]$SkipPerf)

$ErrorActionPreference = "Continue"
$startTime = Get-Date
$logFile = "$PSScriptRoot\..\test_batch_results.log"
"" | Out-File $logFile -Encoding utf8

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Get-Command psmux -ErrorAction SilentlyContinue).Source }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }

# Skip ONLY tests that use Win32 keybd_event/P/Invoke requiring a visible attached TUI window
# or ConPTY console APIs that cannot work headlessly
$skip = @(
    'test_config_exhaustive_tui',    # keybd_event P/Invoke, needs visible window
    'test_tui_win32_proof',          # keybd_event P/Invoke, needs visible window
    'test_issue201_win32_tui_proof', # keybd_event P/Invoke, needs visible window
    'test_issue200_sendkeys_proof',  # keybd_event P/Invoke, needs visible window
    'test_issue211_win32_mouse',     # keybd_event P/Invoke, needs visible window
    'test_conpty_mouse'              # ConPTY raw console input, needs real console
)

# Skip diag/bench/debug prefixed files (diagnostic tools, not test suites)
$skipPrefixes = @('diag_', 'bench_', 'debug_', 'repro_', 'disable_', 'mouse_diag')

if ($SkipPerf) {
    $skip += @(
        'test_stress', 'test_stress_50', 'test_stress_aggressive',
        'test_extreme_perf', 'test_e2e_latency', 'test_pane_startup_perf',
        'test_startup_perf', 'test_perf', 'test_install_speed',
        'test_startup_exit_bench'
    )
}

$allTests = Get-ChildItem "$PSScriptRoot\test_*.ps1" | Sort-Object Name
$filtered = $allTests | Where-Object {
    $name = $_.BaseName
    if ($skip -contains $name) { return $false }
    foreach ($p in $skipPrefixes) { if ($name.StartsWith($p)) { return $false } }
    return $true
}

$totalSuites = @($filtered).Count
$totalPass = 0; $totalFail = 0; $totalTests = 0
$suitePass = 0; $suiteFail = 0; $suiteSkip = 0
$failedSuites = @()
$index = 0

Write-Host "Binary: $PSMUX" -ForegroundColor Cyan
Write-Host "Total suites to run: $totalSuites (skipping $($allTests.Count - $totalSuites) prereq/interactive)" -ForegroundColor Cyan
Write-Host ""
"Binary: $PSMUX" | Out-File $logFile -Append -Encoding utf8
"Total suites to run: $totalSuites (skipping $($allTests.Count - $totalSuites) prereq/interactive)" | Out-File $logFile -Append -Encoding utf8

foreach ($testFile in $filtered) {
    $index++
    $name = $testFile.BaseName
    
    # Cleanup between suites
    try { & $PSMUX kill-server 2>&1 | Out-Null } catch {}
    Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 1500
    Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
    Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue
    
    $pct = [math]::Round(($index / $totalSuites) * 100)
    Write-Host ("[{0,3}/{1}] {2,3}% " -f $index, $totalSuites, $pct) -NoNewline -ForegroundColor DarkGray
    Write-Host "$name " -NoNewline -ForegroundColor White
    
    # Longer timeout for heavy tests (Claude Code e2e, stress, perf)
    $heavyTests = @('test_agent_teams_e2e', 'test_stress', 'test_stress_50', 'test_stress_aggressive', 'test_extreme_perf')
    $timeout = if ($heavyTests -contains $name) { 600 } else { 300 }
    
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    
    try {
        $testJob = Start-Job -ScriptBlock {
            param($f)
            $out = & pwsh -NoProfile -ExecutionPolicy Bypass -File $f 2>&1 | Out-String
            @{ Output = $out; ExitCode = $LASTEXITCODE }
        } -ArgumentList $testFile.FullName
        
        $done = Wait-Job $testJob -Timeout $timeout
        if ($done) {
            $r = Receive-Job $testJob
            $output = $r.Output
            $exitCode = $r.ExitCode
        } else {
            Stop-Job $testJob
            $output = "[TIMEOUT]"
            $exitCode = -2
        }
        Remove-Job $testJob -Force
    } catch {
        $output = "[ERROR] $_"
        $exitCode = -1
    }
    $sw.Stop()
    
    # Count PASS/FAIL
    $passCount = ([regex]::Matches($output, '\[PASS\]')).Count
    $passCount += ([regex]::Matches($output, '(?m)^PASS\s')).Count
    $passCount += ([regex]::Matches($output, '=> PASS$', [System.Text.RegularExpressions.RegexOptions]::Multiline)).Count
    $failCount = ([regex]::Matches($output, '\[FAIL\]')).Count
    $failCount += ([regex]::Matches($output, '(?m)^FAIL\s')).Count
    $failCount += ([regex]::Matches($output, '=> FAIL$', [System.Text.RegularExpressions.RegexOptions]::Multiline)).Count
    
    $totalTests += ($passCount + $failCount)
    $totalPass += $passCount
    $totalFail += $failCount
    
    $status = if ($exitCode -eq -2) { "TIMEOUT" } 
              elseif ($exitCode -eq 0 -and $failCount -eq 0) { "PASS" } 
              else { "FAIL" }
    
    $dur = [math]::Round($sw.Elapsed.TotalSeconds, 1)
    
    switch ($status) {
        "PASS" { 
            $suitePass++
            Write-Host ("{0}P/{1}F " -f $passCount, $failCount) -NoNewline -ForegroundColor Green
            Write-Host ("{0}s" -f $dur) -ForegroundColor DarkGray
            "[{0,3}/{1}] PASS {2} {3}P/{4}F {5}s" -f $index, $totalSuites, $name, $passCount, $failCount, $dur | Out-File $logFile -Append -Encoding utf8
        }
        "TIMEOUT" {
            $suiteFail++
            $failedSuites += "$name (TIMEOUT)"
            Write-Host "TIMEOUT " -NoNewline -ForegroundColor Yellow
            Write-Host ("{0}s" -f $dur) -ForegroundColor DarkGray
            "[{0,3}/{1}] TIMEOUT {2} {3}s" -f $index, $totalSuites, $name, $dur | Out-File $logFile -Append -Encoding utf8
        }
        "FAIL" {
            $suiteFail++
            $failedSuites += "$name ($passCount P/$failCount F, exit=$exitCode)"
            Write-Host ("{0}P/{1}F " -f $passCount, $failCount) -NoNewline -ForegroundColor Red
            Write-Host ("exit={0} {1}s" -f $exitCode, $dur) -ForegroundColor DarkGray
            "[{0,3}/{1}] FAIL {2} {3}P/{4}F exit={5} {6}s" -f $index, $totalSuites, $name, $passCount, $failCount, $exitCode, $dur | Out-File $logFile -Append -Encoding utf8
            # Save detailed output for failed suites
            $failDir = "$PSScriptRoot\..\test_failures"
            if (-not (Test-Path $failDir)) { New-Item -ItemType Directory -Path $failDir -Force | Out-Null }
            $output | Out-File "$failDir\${name}.txt" -Encoding utf8
        }
    }
}

# Final cleanup
try { & $PSMUX kill-server 2>&1 | Out-Null } catch {}
Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue

$elapsed = ((Get-Date) - $startTime).TotalSeconds
Write-Host ""
Write-Host ("=" * 70) -ForegroundColor White
Write-Host "  FINAL RESULTS" -ForegroundColor White
Write-Host ("=" * 70) -ForegroundColor White
Write-Host ""
Write-Host "  Suites: " -NoNewline
Write-Host "$suitePass PASS" -ForegroundColor Green -NoNewline
Write-Host " / " -NoNewline
Write-Host "$suiteFail FAIL" -ForegroundColor $(if ($suiteFail -gt 0) { "Red" } else { "Green" }) -NoNewline
Write-Host " / $suiteSkip SKIP" -ForegroundColor Yellow
Write-Host "  Individual tests: " -NoNewline
Write-Host "$totalPass PASS" -ForegroundColor Green -NoNewline
Write-Host " / " -NoNewline
Write-Host "$totalFail FAIL" -ForegroundColor $(if ($totalFail -gt 0) { "Red" } else { "Green" })
Write-Host "  Total duration: $([math]::Round($elapsed/60, 1)) minutes"
Write-Host ""

if ($failedSuites.Count -gt 0) {
    Write-Host "  FAILED SUITES:" -ForegroundColor Red
    foreach ($f in $failedSuites) { Write-Host "    $f" -ForegroundColor Red }
    Write-Host ""
}

$skippedCount = $allTests.Count - $totalSuites
Write-Host "  Skipped $skippedCount suites (WSL/Claude/Interactive TUI prerequisites)" -ForegroundColor Yellow
Write-Host ""

# Write final summary to log
"" | Out-File $logFile -Append -Encoding utf8
"======================================================================" | Out-File $logFile -Append -Encoding utf8
"FINAL RESULTS" | Out-File $logFile -Append -Encoding utf8
"======================================================================" | Out-File $logFile -Append -Encoding utf8
"Suites: $suitePass PASS / $suiteFail FAIL / $suiteSkip SKIP" | Out-File $logFile -Append -Encoding utf8
"Individual tests: $totalPass PASS / $totalFail FAIL" | Out-File $logFile -Append -Encoding utf8
"Total duration: $([math]::Round($elapsed/60, 1)) minutes" | Out-File $logFile -Append -Encoding utf8
if ($failedSuites.Count -gt 0) {
    "FAILED SUITES:" | Out-File $logFile -Append -Encoding utf8
    foreach ($f in $failedSuites) { "  $f" | Out-File $logFile -Append -Encoding utf8 }
}
"Skipped $skippedCount suites" | Out-File $logFile -Append -Encoding utf8
