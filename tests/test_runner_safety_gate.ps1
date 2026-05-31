#!/usr/bin/env pwsh
# Regression guard for the run_all_tests.ps1 safety gate.
#
# run_all_tests.ps1 is DESTRUCTIVE: between every test it kills all psmux
# processes and wipes ~/.psmux. It must refuse to run unless PSMUX_TEST_SANDBOX=1,
# and that gate must come BEFORE any destructive operation.
#
# This test is purely STATIC — it reads run_all_tests.ps1 and never executes it
# — so it is safe to run anywhere, including on a machine with a live psmux.

$ErrorActionPreference = "Continue"
$runner = Join-Path $PSScriptRoot "run_all_tests.ps1"
$pass = 0; $fail = 0
function Check($name, $cond, $detail = "") {
    if ($cond) { Write-Host "[PASS] $name" -ForegroundColor Green; $script:pass++ }
    else { Write-Host "[FAIL] $name $detail" -ForegroundColor Red; $script:fail++ }
}

if (-not (Test-Path $runner)) {
    Write-Host "[FAIL] run_all_tests.ps1 not found at $runner" -ForegroundColor Red
    exit 1
}
$src = Get-Content -LiteralPath $runner -Raw

# 1. The gate references PSMUX_TEST_SANDBOX and aborts (exit) shortly after.
$gateIdx = $src.IndexOf('PSMUX_TEST_SANDBOX')
Check "run_all_tests.ps1 references the PSMUX_TEST_SANDBOX gate" ($gateIdx -ge 0)
Check "gate aborts with exit when not in a sandbox" ($src -match 'PSMUX_TEST_SANDBOX[\s\S]{0,1200}?exit\s+\d')

# 2. The gate must appear BEFORE every destructive operation, so the runner can
#    never reach them without the opt-in.
$destructive = @('Stop-Process', 'taskkill', 'Remove-Item\s+"\$env:USERPROFILE\\\.psmux')
foreach ($pat in $destructive) {
    $m = [regex]::Match($src, $pat)
    if ($m.Success) {
        Check "gate precedes destructive '$pat'" (($gateIdx -ge 0) -and ($gateIdx -lt $m.Index)) "gate@$gateIdx match@$($m.Index)"
    }
}

Write-Host "`n=== run_all_tests.ps1 safety gate (static guard) ===" -ForegroundColor Cyan
Write-Host "Passed: $pass  Failed: $fail" -ForegroundColor $(if ($fail -gt 0) { "Red" } else { "Green" })
if ($fail -gt 0) { exit 1 }
