$ErrorActionPreference = "Continue"

param(
    [string]$PsmuxExe = $env:PSMUX_EXE
)

function Resolve-PsmuxExe {
    param([string]$PreferredPath)

    if ($PreferredPath -and (Test-Path $PreferredPath)) {
        return (Resolve-Path $PreferredPath).Path
    }

    $candidates = @(
        "$PSScriptRoot\..\target\x86_64-pc-windows-msvc\release\psmux.exe",
        "$PSScriptRoot\..\target\release\psmux.exe",
        "$PSScriptRoot\..\target\debug\psmux.exe"
    )

    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) {
            return (Resolve-Path $candidate).Path
        }
    }

    $cmd = Get-Command psmux -ErrorAction SilentlyContinue
    if ($cmd) {
        if ($cmd.Path) { return $cmd.Path }
        if ($cmd.Source) { return $cmd.Source }
    }

    return $null
}

$resolvedPsmuxExe = Resolve-PsmuxExe -PreferredPath $PsmuxExe
if (-not $resolvedPsmuxExe) {
    Write-Host "FATAL: could not locate psmux executable for smoke integrations" -ForegroundColor Red
    exit 1
}

$env:PSMUX_EXE = $resolvedPsmuxExe
$env:Path = "$(Split-Path -Parent $resolvedPsmuxExe);$env:Path"

$tests = @(
    "test_smoke_pr.ps1",
    "test_session_mgmt.ps1",
    "test_run_shell.ps1",
    "test_issue209_e2e_verify.ps1",
    "test_swapw_revert.ps1"
)

$suitePass = 0
$suiteFail = 0

Write-Host "`n=== Smoke Integration Test Suite ===" -ForegroundColor Cyan
Write-Host "psmux: $resolvedPsmuxExe" -ForegroundColor DarkGray
Write-Host "tests: $($tests.Count)" -ForegroundColor DarkGray

foreach ($test in $tests) {
    $testPath = Join-Path $PSScriptRoot $test
    if (-not (Test-Path $testPath)) {
        Write-Host "`n[FAIL] $test (file missing)" -ForegroundColor Red
        $suiteFail++
        continue
    }

    Write-Host "`n--- Running $test ---" -ForegroundColor Yellow
    & pwsh -NoProfile -ExecutionPolicy Bypass -File $testPath
    $exitCode = $LASTEXITCODE

    if ($exitCode -eq 0) {
        Write-Host "[PASS] $test" -ForegroundColor Green
        $suitePass++
    } else {
        Write-Host "[FAIL] $test (exit=$exitCode)" -ForegroundColor Red
        $suiteFail++
    }
}

Write-Host "`n=== Smoke Integration Summary ===" -ForegroundColor Cyan
Write-Host "  Passed suites: $suitePass" -ForegroundColor Green
Write-Host "  Failed suites: $suiteFail" -ForegroundColor $(if ($suiteFail -gt 0) { "Red" } else { "Green" })

if ($suiteFail -gt 0) {
    exit 1
}

exit 0
