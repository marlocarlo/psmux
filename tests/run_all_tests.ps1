# psmux Comprehensive Test Runner
# Runs ALL test suites sequentially with proper cleanup, captures results,
# and produces a full report including performance metrics.
#
# Usage: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\run_all_tests.ps1

param(
    [switch]$SkipPerf,       # Skip long-running perf/stress tests
    [switch]$IncludeWSL,     # Include WSL-dependent tests
    [switch]$IncludeInteractive  # Include tests that need interactive TUI
)

$ErrorActionPreference = "Continue"
$startTime = Get-Date

# ── Logging setup ──────────────────────────────────────────────
# All logs go to $env:TEMP\psmux-test-logs\ (never inside the repo).
# Each run gets a timestamped folder with:
#   progress.log   – one-line-per-suite result, flushed immediately (crash-safe)
#   summary.log    – final report (written at end)
#   suites\<name>.log – full stdout/stderr captured from each test file
$script:LogRoot = Join-Path $env:TEMP "psmux-test-logs"
$script:RunId   = $startTime.ToString("yyyy-MM-dd_HH-mm-ss")
$script:RunDir  = Join-Path $script:LogRoot $script:RunId
$script:SuiteDir = Join-Path $script:RunDir "suites"
New-Item -ItemType Directory -Path $script:SuiteDir -Force | Out-Null

$script:ProgressLog = Join-Path $script:RunDir "progress.log"
$script:SummaryLog  = Join-Path $script:RunDir "summary.log"

# Also maintain a symlink-like "latest" pointer
$latestFile = Join-Path $script:LogRoot "latest_run.txt"
Set-Content -Path $latestFile -Value $script:RunId -Encoding UTF8

function Write-Log {
    param([string]$Message, [string]$File = $script:ProgressLog)
    $ts = Get-Date -Format "yyyy-MM-dd HH:mm:ss.fff"
    $line = "[$ts] $Message"
    # Append + flush immediately so partial results survive crashes/power loss
    [System.IO.File]::AppendAllText($File, "$line`r`n")
}

Write-Log "=== psmux test run started ==="
Write-Log "Run ID: $script:RunId"
Write-Log "Log directory: $script:RunDir"

# ── Binary discovery ──
$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { $PSMUX = (Get-Command psmux -ErrorAction SilentlyContinue).Source }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }

Write-Log "Binary: $PSMUX"
Write-Log "Params: SkipPerf=$SkipPerf IncludeWSL=$IncludeWSL IncludeInteractive=$IncludeInteractive"

Write-Host "Binary: $PSMUX" -ForegroundColor Cyan
Write-Host "Started: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" -ForegroundColor Cyan
Write-Host "Logs:    $script:RunDir" -ForegroundColor Cyan
Write-Host ""

# ── Categorize tests ──
# Tests requiring WSL
$wslTests = @(
    "test_wsl_in_pwsh_latency", "test_wsl_in_pwsh_latency2", "test_wsl_latency",
    "test_wsl_pwsh_latency3", "test_wsl_pwsh_latency4", "test_wsl_pwsh_latency5"
)
# Tests requiring interactive TUI / attached session / mouse
$interactiveTests = @(
    "test_claude_mouse", "test_conpty_mouse", "test_mouse_handling", "test_mouse_hover",
    "test_stress_attached", "test_tui_exit_cleanup", "test_claude_cursor_diag",
    "test_issue60_native_tui_mouse", "test_issue15_altgr", "test_cursor_fallback",
    "test_cursor_style", "test_issue52_cursor", "test_perf_vs_wt"
)
# Long-running stress/perf tests
$perfTests = @(
    "test_stress", "test_stress_50", "test_stress_aggressive", "test_extreme_perf",
    "test_e2e_latency", "test_pane_startup_perf", "test_startup_perf", "test_perf"
)

# Results tracking
$results = [System.Collections.ArrayList]::new()

# ── Live dashboard state ──
$script:LivePass = 0; $script:LiveFail = 0; $script:LiveSkip = 0
$script:LivePassTests = 0; $script:LiveFailTests = 0
$script:SuiteDurations = [System.Collections.ArrayList]::new()  # rolling avg for ETA

function Get-Category {
    param([string]$Name)
    if ($wslTests -contains $Name) { return "WSL" }
    if ($interactiveTests -contains $Name) { return "Interactive" }
    if ($perfTests -contains $Name) { return "Perf/Stress" }
    if ($Name -match 'test_issue') { return "Issue Fixes" }
    if ($Name -match 'test_config|test_plugin|test_theme') { return "Config/Plugin" }
    if ($Name -match 'test_copy_mode|test_pane|test_layout|test_split|test_zoom') { return "UI/Layout" }
    if ($Name -match 'test_session|test_kill|test_warm') { return "Session Mgmt" }
    return "General"
}

function Show-ProgressDashboard {
    param([int]$Current, [int]$Total, [string]$SuiteName, [string]$Status)
    $pct = if ($Total -gt 0) { [math]::Round(($Current / $Total) * 100) } else { 0 }
    $elapsed = ((Get-Date) - $startTime).TotalSeconds

    # ETA calculation from rolling average
    $eta = "--:--"
    if ($script:SuiteDurations.Count -gt 0) {
        $avgTime = ($script:SuiteDurations | Measure-Object -Average).Average
        $remaining = ($Total - $Current) * $avgTime
        if ($remaining -gt 3600) {
            $eta = "{0:F0}h {1:F0}m" -f [math]::Floor($remaining/3600), [math]::Floor(($remaining%3600)/60)
        } elseif ($remaining -gt 60) {
            $eta = "{0:F0}m {1:F0}s" -f [math]::Floor($remaining/60), [math]::Floor($remaining%60)
        } else {
            $eta = "{0:F0}s" -f $remaining
        }
    }

    # Progress bar (40 chars wide)
    $barWidth = 40
    $filled = [math]::Max([math]::Round($pct / 100 * $barWidth), 0)
    $empty  = $barWidth - $filled
    $barFill  = [char]0x2588  # full block
    $barEmpty = [char]0x2591  # light shade
    $bar = ($barFill.ToString() * $filled) + ($barEmpty.ToString() * $empty)

    $barColor = if ($script:LiveFail -gt 0) { "Red" } elseif ($pct -ge 80) { "Green" } else { "Yellow" }

    # Status badge
    $badge = switch ($Status) {
        "PASS"  { "[PASS]" }
        "FAIL"  { "[FAIL]" }
        "SKIP"  { "[SKIP]" }
        "ERROR" { "[ERR!]" }
        default { "[....]" }
    }
    $badgeColor = switch ($Status) {
        "PASS"  { "Green" }
        "FAIL"  { "Red" }
        "SKIP"  { "Yellow" }
        "ERROR" { "Magenta" }
        default { "DarkGray" }
    }

    Write-Host ""
    Write-Host ("  {0} " -f $bar) -ForegroundColor $barColor -NoNewline
    Write-Host ("{0,3}%" -f $pct) -ForegroundColor White -NoNewline
    Write-Host ("  [{0}/{1}]" -f $Current, $Total) -ForegroundColor DarkGray -NoNewline
    Write-Host ("  ETA: {0}" -f $eta) -ForegroundColor Cyan

    # Live counters
    Write-Host "  " -NoNewline
    Write-Host ("Pass:{0}" -f $script:LivePass) -ForegroundColor Green -NoNewline
    Write-Host " | " -ForegroundColor DarkGray -NoNewline
    Write-Host ("Fail:{0}" -f $script:LiveFail) -ForegroundColor $(if ($script:LiveFail -gt 0) { "Red" } else { "Green" }) -NoNewline
    Write-Host " | " -ForegroundColor DarkGray -NoNewline
    Write-Host ("Skip:{0}" -f $script:LiveSkip) -ForegroundColor Yellow -NoNewline
    Write-Host " | " -ForegroundColor DarkGray -NoNewline
    Write-Host "Tests: " -ForegroundColor DarkGray -NoNewline
    Write-Host ("{0}" -f $script:LivePassTests) -ForegroundColor Green -NoNewline
    Write-Host "/" -ForegroundColor DarkGray -NoNewline
    $fColor = if ($script:LiveFailTests -gt 0) { "Red" } else { "Green" }
    Write-Host ("{0}" -f $script:LiveFailTests) -ForegroundColor $fColor -NoNewline
    $elapsedFmt = if ($elapsed -gt 3600) { "{0:F0}h{1:F0}m" -f [math]::Floor($elapsed/3600),[math]::Floor(($elapsed%3600)/60) } elseif ($elapsed -gt 60) { "{0:F0}m{1:F0}s" -f [math]::Floor($elapsed/60),[math]::Floor($elapsed%60) } else { "{0:F0}s" -f $elapsed }
    Write-Host ("  Elapsed: {0}" -f $elapsedFmt) -ForegroundColor DarkGray

    # Last suite result
    if ($SuiteName) {
        Write-Host "  " -NoNewline
        Write-Host $badge -ForegroundColor $badgeColor -NoNewline
        Write-Host (" {0}" -f $SuiteName) -ForegroundColor White
    }
}

function Clean-Server {
    # Gracefully ask all servers to exit
    try { & $PSMUX kill-server 2>&1 | Out-Null } catch {}
    Start-Sleep -Milliseconds 500
    # Force-kill any lingering processes
    Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    # Wait for OS to release TCP ports and file handles
    Start-Sleep -Seconds 3
    # Remove stale port/key files
    Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
    Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue
    # Remove any test config files (tests should restore originals but may fail)
    Remove-Item "$env:USERPROFILE\.psmux.conf" -Force -ErrorAction SilentlyContinue
    Remove-Item "$env:USERPROFILE\.psmuxrc" -Force -ErrorAction SilentlyContinue
    # Verify no psmux processes remain
    $remaining = Get-Process psmux -ErrorAction SilentlyContinue
    if ($remaining) {
        Start-Sleep -Seconds 2
        Get-Process psmux -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
        Start-Sleep -Seconds 1
    }
}

function Run-TestFile {
    param([string]$FilePath)

    $name = [System.IO.Path]::GetFileNameWithoutExtension($FilePath)
    $baseName = $name
    $suiteLog = Join-Path $script:SuiteDir "$baseName.log"

    # Check skip categories
    if ($wslTests -contains $baseName -and -not $IncludeWSL) {
        Write-Log "SKIP  $baseName  (WSL required)"
        return @{ Name = $baseName; Status = "SKIP"; Reason = "WSL required"; Passed = 0; Failed = 0; Duration = 0 }
    }
    if ($interactiveTests -contains $baseName -and -not $IncludeInteractive) {
        Write-Log "SKIP  $baseName  (Interactive TUI required)"
        return @{ Name = $baseName; Status = "SKIP"; Reason = "Interactive TUI required"; Passed = 0; Failed = 0; Duration = 0 }
    }
    if ($perfTests -contains $baseName -and $SkipPerf) {
        Write-Log "SKIP  $baseName  (Perf test, -SkipPerf active)"
        return @{ Name = $baseName; Status = "SKIP"; Reason = "Perf test (use -SkipPerf to skip)"; Passed = 0; Failed = 0; Duration = 0 }
    }

    Clean-Server

    Write-Log "START $baseName"

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    Write-Host "`n$('=' * 60)" -ForegroundColor DarkGray
    Write-Host "  RUNNING: $baseName" -ForegroundColor White
    Write-Host "$('=' * 60)" -ForegroundColor DarkGray

    try {
        $output = & pwsh -NoProfile -ExecutionPolicy Bypass -File $FilePath 2>&1 | Out-String
        $exitCode = $LASTEXITCODE
        $sw.Stop()

        # Write full output to per-suite log file
        $suiteHeader = "Suite: $baseName`r`nFile:  $FilePath`r`nStart: $(($startTime + $sw.Elapsed - $sw.Elapsed).ToString('yyyy-MM-dd HH:mm:ss'))`r`nEnd:   $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')`r`nExit:  $exitCode`r`nDuration: $([math]::Round($sw.Elapsed.TotalSeconds,1))s`r`n$('=' * 70)`r`n"
        [System.IO.File]::WriteAllText($suiteLog, "$suiteHeader$output", [System.Text.Encoding]::UTF8)

        # Count PASS/FAIL from output (multiple patterns used by different test scripts)
        $passCount = ([regex]::Matches($output, '\[PASS\]')).Count
        $passCount += ([regex]::Matches($output, '(?m)^PASS\s')).Count
        $passCount += ([regex]::Matches($output, '=> PASS$', [System.Text.RegularExpressions.RegexOptions]::Multiline)).Count
        $failCount = ([regex]::Matches($output, '\[FAIL\]')).Count
        $failCount += ([regex]::Matches($output, '(?m)^FAIL\s')).Count
        $failCount += ([regex]::Matches($output, '=> FAIL$', [System.Text.RegularExpressions.RegexOptions]::Multiline)).Count
        $skipCount = ([regex]::Matches($output, '\[SKIP\]')).Count

        # Show output
        Write-Host $output

        $status = if ($exitCode -eq 0 -and $failCount -eq 0) { "PASS" } else { "FAIL" }

        Write-Log ("{0,-5} {1,-45} {2}P/{3}F  exit={4}  {5}s" -f $status, $baseName, $passCount, $failCount, $exitCode, [math]::Round($sw.Elapsed.TotalSeconds,1))

        return @{
            Name = $baseName
            Status = $status
            ExitCode = $exitCode
            Passed = $passCount
            Failed = $failCount
            Skipped = $skipCount
            Duration = [math]::Round($sw.Elapsed.TotalSeconds, 1)
            Output = $output
        }
    } catch {
        $sw.Stop()
        Write-Host "  ERROR: $_" -ForegroundColor Red
        [System.IO.File]::WriteAllText($suiteLog, "Suite: $baseName`r`nERROR: $_`r`n", [System.Text.Encoding]::UTF8)
        Write-Log "ERROR $baseName  $_"
        return @{
            Name = $baseName
            Status = "ERROR"
            Passed = 0
            Failed = 1
            Duration = [math]::Round($sw.Elapsed.TotalSeconds, 1)
            Output = $_.ToString()
        }
    }
}

# ── Collect all test files ──
$allTests = Get-ChildItem "$PSScriptRoot\test_*.ps1" | Sort-Object Name
$totalSuites = $allTests.Count
Write-Host ""
Write-Host ("  {0} test suites discovered" -f $totalSuites) -ForegroundColor Cyan
Write-Log "Found $totalSuites test files"

# Category header
$catGroups = @{}
foreach ($t in $allTests) {
    $cat = Get-Category $t.BaseName
    if (-not $catGroups.ContainsKey($cat)) { $catGroups[$cat] = 0 }
    $catGroups[$cat]++
}
Write-Host "  Categories: " -ForegroundColor DarkGray -NoNewline
$catNames = ($catGroups.GetEnumerator() | Sort-Object Value -Descending | ForEach-Object { "{0}({1})" -f $_.Key,$_.Value })
Write-Host ($catNames -join "  ") -ForegroundColor DarkGray
Write-Host ""

# ── Run each test ──
$suiteIndex = 0
foreach ($testFile in $allTests) {
    $suiteIndex++
    Write-Log "--- [$suiteIndex/$totalSuites] Queuing $($testFile.BaseName) ---"

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $result = Run-TestFile -FilePath $testFile.FullName
    $sw.Stop()
    [void]$results.Add($result)
    [void]$script:SuiteDurations.Add($sw.Elapsed.TotalSeconds)

    # Update live counters
    switch ($result.Status) {
        "PASS"  { $script:LivePass++ }
        "FAIL"  { $script:LiveFail++ }
        "ERROR" { $script:LiveFail++ }
        "SKIP"  { $script:LiveSkip++ }
    }
    $script:LivePassTests += $result.Passed
    $script:LiveFailTests += $result.Failed

    Show-ProgressDashboard -Current $suiteIndex -Total $totalSuites -SuiteName $testFile.BaseName -Status $result.Status
}

# ── Final cleanup ──
Clean-Server

# ── Generate Report ──
$endTime = Get-Date
$totalDuration = ($endTime - $startTime).TotalSeconds

$bullet = [char]0x25CF  # ●

Write-Host "`n"
Write-Host ("=" * 80) -ForegroundColor White
Write-Host "  COMPREHENSIVE TEST REPORT" -ForegroundColor White
Write-Host "  $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" -ForegroundColor DarkGray
Write-Host ("=" * 80) -ForegroundColor White

$passed = @($results | Where-Object { $_.Status -eq "PASS" })
$failed = @($results | Where-Object { $_.Status -eq "FAIL" -or $_.Status -eq "ERROR" })
$skipped = @($results | Where-Object { $_.Status -eq "SKIP" })

$totalTests = 0; $totalPassed = 0; $totalFailed = 0
foreach ($r in $results) { $totalTests += ($r.Passed + $r.Failed); $totalPassed += $r.Passed; $totalFailed += $r.Failed }

# ── Suite & Test Counters ──
Write-Host ""
Write-Host "  SUITE SUMMARY" -ForegroundColor Cyan
Write-Host "  -------------------------------------------------------"
Write-Host ("  $bullet Suites PASSED:  {0}" -f $passed.Count) -ForegroundColor Green
Write-Host ("  $bullet Suites FAILED:  {0}" -f $failed.Count) -ForegroundColor $(if ($failed.Count -gt 0) { "Red" } else { "Green" })
Write-Host ("  $bullet Suites SKIPPED: {0}" -f $skipped.Count) -ForegroundColor Yellow
Write-Host ""
Write-Host "  INDIVIDUAL TEST SUMMARY" -ForegroundColor Cyan
Write-Host "  -------------------------------------------------------"
Write-Host ("  $bullet Tests PASSED:   {0}" -f $totalPassed) -ForegroundColor Green
Write-Host ("  $bullet Tests FAILED:   {0}" -f $totalFailed) -ForegroundColor $(if ($totalFailed -gt 0) { "Red" } else { "Green" })
Write-Host ("  $bullet Total Duration: {0:F1}s ({1:F1} min)" -f $totalDuration, ($totalDuration / 60))

# ── Category-Wise Breakdown ──
Write-Host ""
Write-Host ("=" * 80) -ForegroundColor White
Write-Host "  CATEGORY BREAKDOWN" -ForegroundColor White
Write-Host ("=" * 80) -ForegroundColor White
Write-Host ""
Write-Host ("  {0,-16} {1,6} {2,6} {3,6} {4,10}" -f "Category", "Pass", "Fail", "Skip", "Time") -ForegroundColor White
Write-Host ("  " + ("-" * 50)) -ForegroundColor DarkGray

$catStats = @{}
foreach ($r in $results) {
    $cat = Get-Category $r.Name
    if (-not $catStats.ContainsKey($cat)) {
        $catStats[$cat] = @{ Pass=0; Fail=0; Skip=0; Time=[double]0 }
    }
    switch ($r.Status) {
        "PASS"  { $catStats[$cat].Pass++ }
        "FAIL"  { $catStats[$cat].Fail++ }
        "ERROR" { $catStats[$cat].Fail++ }
        "SKIP"  { $catStats[$cat].Skip++ }
    }
    $catStats[$cat].Time += $r.Duration
}

foreach ($kv in ($catStats.GetEnumerator() | Sort-Object { $_.Value.Fail } -Descending)) {
    $c = $kv.Value
    $catColor = if ($c.Fail -gt 0) { "Red" } elseif ($c.Skip -gt 0 -and $c.Pass -eq 0) { "Yellow" } else { "Green" }
    $timeFmt = if ($c.Time -ge 60) { "{0:F0}m{1:F0}s" -f [math]::Floor($c.Time/60),[math]::Floor($c.Time%60) } else { "{0:F1}s" -f $c.Time }
    Write-Host ("  {0,-16} {1,6} {2,6} {3,6} {4,10}" -f $kv.Key, $c.Pass, $c.Fail, $c.Skip, $timeFmt) -ForegroundColor $catColor
}

# ── Failures first, then passed, then skipped ──
if ($failed.Count -gt 0) {
    Write-Host ""
    Write-Host ("  " + ("-" * 55)) -ForegroundColor Red
    Write-Host "  FAILED SUITES" -ForegroundColor Red
    foreach ($r in $failed) {
        Write-Host ("    $bullet [FAIL] {0,-42} {1,3}P/{2}F  ({3}s)" -f $r.Name, $r.Passed, $r.Failed, $r.Duration) -ForegroundColor Red
    }
}

if ($passed.Count -gt 0) {
    Write-Host ""
    Write-Host "  PASSED SUITES" -ForegroundColor Green
    foreach ($r in $passed) {
        Write-Host ("    $bullet [PASS] {0,-42} {1,3}P/{2}F  ({3}s)" -f $r.Name, $r.Passed, $r.Failed, $r.Duration) -ForegroundColor Green
    }
}

if ($skipped.Count -gt 0) {
    Write-Host ""
    Write-Host "  SKIPPED SUITES" -ForegroundColor Yellow
    foreach ($r in $skipped) {
        Write-Host ("    $bullet [SKIP] {0,-42} {1}" -f $r.Name, $r.Reason) -ForegroundColor Yellow
    }
}

# ── Performance chart (top 15 slowest, visual bar) ──
Write-Host "`n"
Write-Host ("=" * 80) -ForegroundColor White
Write-Host "  PERFORMANCE METRICS (top 15 slowest suites)" -ForegroundColor White
Write-Host ("=" * 80) -ForegroundColor White
Write-Host ""
$perfResults = $results | Where-Object { $_.Status -ne "SKIP" } | Sort-Object { $_.Duration } -Descending | Select-Object -First 15
$maxDur = ($perfResults | Measure-Object -Property Duration -Maximum).Maximum
if ($maxDur -lt 1) { $maxDur = 1 }
$barBlock = [char]0x2588
foreach ($r in $perfResults) {
    $barLen = [math]::Max([math]::Round(($r.Duration / $maxDur) * 30), 1)
    $bar = $barBlock.ToString() * $barLen
    $color = if ($r.Status -eq "PASS") { "Green" } elseif ($r.Status -eq "FAIL") { "Red" } else { "Yellow" }
    Write-Host ("  {0,-42} {1,7:F1}s " -f $r.Name, $r.Duration) -ForegroundColor DarkGray -NoNewline
    Write-Host $bar -ForegroundColor $color
}

Write-Host "`n"
Write-Host ("=" * 80) -ForegroundColor White
if ($totalFailed -gt 0) {
    Write-Host "  RESULT: FAILURES DETECTED ($totalFailed tests failed)" -ForegroundColor Red
    Write-Log "=== FINAL RESULT: FAILURES DETECTED ($totalFailed tests failed across $($failed.Count) suites) ==="
} else {
    Write-Host "  RESULT: ALL TESTS PASSED ($totalPassed tests across $($passed.Count) suites)" -ForegroundColor Green
    Write-Log "=== FINAL RESULT: ALL TESTS PASSED ($totalPassed tests across $($passed.Count) suites) ==="
}

# ── Write comprehensive summary.log ──────────────────────────────
$summaryLines = [System.Collections.ArrayList]::new()
[void]$summaryLines.Add("psmux Test Run Summary")
[void]$summaryLines.Add("Run ID:   $script:RunId")
[void]$summaryLines.Add("Started:  $($startTime.ToString('yyyy-MM-dd HH:mm:ss'))")
[void]$summaryLines.Add("Finished: $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')")
[void]$summaryLines.Add("Duration: $([math]::Round($totalDuration,1))s ($([math]::Round($totalDuration/60,1)) min)")
[void]$summaryLines.Add("Binary:   $PSMUX")
[void]$summaryLines.Add("Params:   SkipPerf=$SkipPerf IncludeWSL=$IncludeWSL IncludeInteractive=$IncludeInteractive")
[void]$summaryLines.Add("")
[void]$summaryLines.Add("Suites PASSED:  $($passed.Count)")
[void]$summaryLines.Add("Suites FAILED:  $($failed.Count)")
[void]$summaryLines.Add("Suites SKIPPED: $($skipped.Count)")
[void]$summaryLines.Add("Tests PASSED:   $totalPassed")
[void]$summaryLines.Add("Tests FAILED:   $totalFailed")
[void]$summaryLines.Add("")
[void]$summaryLines.Add("=" * 70)
foreach ($r in $results) {
    $line = "[{0,-5}] {1,-45} {2,3}P/{3}F  {4,7:F1}s" -f $r.Status, $r.Name, $r.Passed, $r.Failed, $r.Duration
    if ($r.Reason) { $line += "  ($($r.Reason))" }
    [void]$summaryLines.Add($line)
}
[void]$summaryLines.Add("=" * 70)
if ($totalFailed -gt 0) {
    [void]$summaryLines.Add("RESULT: FAILURES DETECTED")
} else {
    [void]$summaryLines.Add("RESULT: ALL TESTS PASSED")
}
[System.IO.File]::WriteAllText($script:SummaryLog, ($summaryLines -join "`r`n"), [System.Text.Encoding]::UTF8)

Write-Log "Summary written to: $script:SummaryLog"
Write-Log "Suite logs in:      $script:SuiteDir"
Write-Log "=== Run finished ==="

Write-Host ""
Write-Host "  Logs saved to: $script:RunDir" -ForegroundColor Cyan

if ($totalFailed -gt 0) {
    exit 1
} else {
    exit 0
}
