$ErrorActionPreference = "Continue"
$psmux = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $psmux)) { $psmux = "$PSScriptRoot\..\target\debug\psmux.exe" }

# Clean up first
taskkill /f /im psmux.exe 2>$null
Start-Sleep 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

Write-Host "=== Creating session ==="
& $psmux new-session -s shelltest -d 2>&1
$createExit = $LASTEXITCODE
Write-Host "new-session exit: $createExit"
Start-Sleep 3

$results = @()

# --- TEST 1: Write-Output ---
Write-Host "`n=== TEST 1: run-shell Write-Output ==="
$out1 = & $psmux run-shell -t shelltest "Write-Output 'hello from pwsh'" 2>&1 | Out-String
$exit1 = $LASTEXITCODE
Write-Host "run-shell output: $out1"
Write-Host "run-shell exit: $exit1"
Start-Sleep 2
$cap1 = & $psmux capture-pane -t shelltest -p 2>&1 | Out-String
Write-Host "capture-pane:`n$cap1"
$pass1 = ($exit1 -eq 0)
$results += [PSCustomObject]@{Test="TEST1: Write-Output"; Exit=$exit1; Pass=$pass1; Output=$out1.Trim(); Capture=$cap1.Trim()}

# --- TEST 2: Check if "hello from pwsh" was returned by run-shell ---
Write-Host "`n=== TEST 2: Verify 'hello from pwsh' in run-shell output ==="
$pass2 = $out1 -match "hello from pwsh"
$results += [PSCustomObject]@{Test="TEST2: hello from pwsh in output"; Exit="N/A"; Pass=$pass2; Output=$out1.Trim(); Capture=""}

# --- TEST 3: $env:USERNAME ---
Write-Host "`n=== TEST 3: run-shell `$env:USERNAME ==="
$out3 = & $psmux run-shell -t shelltest '$env:USERNAME' 2>&1 | Out-String
$exit3 = $LASTEXITCODE
Write-Host "run-shell output: $out3"
Write-Host "run-shell exit: $exit3"
Start-Sleep 2
$cap3 = & $psmux capture-pane -t shelltest -p 2>&1 | Out-String
Write-Host "capture-pane:`n$cap3"
$pass3 = ($exit3 -eq 0)
$results += [PSCustomObject]@{Test="TEST3: env:USERNAME"; Exit=$exit3; Pass=$pass3; Output=$out3.Trim(); Capture=$cap3.Trim()}

# --- TEST 4: Get-Date ---
Write-Host "`n=== TEST 4: run-shell Get-Date ==="
$out4 = & $psmux run-shell -t shelltest "Get-Date -Format 'yyyy'" 2>&1 | Out-String
$exit4 = $LASTEXITCODE
Write-Host "run-shell output: $out4"
Write-Host "run-shell exit: $exit4"
Start-Sleep 2
$cap4 = & $psmux capture-pane -t shelltest -p 2>&1 | Out-String
Write-Host "capture-pane:`n$cap4"
$pass4 = ($exit4 -eq 0)
$results += [PSCustomObject]@{Test="TEST4: Get-Date"; Exit=$exit4; Pass=$pass4; Output=$out4.Trim(); Capture=$cap4.Trim()}

# --- TEST 5: if-shell true condition (exit 0) ---
Write-Host "`n=== TEST 5: if-shell true condition ==="
$out5 = & $psmux if-shell -t shelltest "exit 0" "display-message 'true-branch'" "display-message 'false-branch'" 2>&1 | Out-String
$exit5 = $LASTEXITCODE
Write-Host "if-shell output: $out5"
Write-Host "if-shell exit: $exit5"
Start-Sleep 2
$pass5 = ($exit5 -eq 0)
$results += [PSCustomObject]@{Test="TEST5: if-shell true"; Exit=$exit5; Pass=$pass5; Output=$out5.Trim(); Capture=""}

# --- TEST 6: Verify exit code 0 from test 5 ---
Write-Host "`n=== TEST 6: Verify if-shell true exit code ==="
$pass6 = ($exit5 -eq 0)
$results += [PSCustomObject]@{Test="TEST6: if-shell true exit=0"; Exit=$exit5; Pass=$pass6; Output=""; Capture=""}

# --- TEST 7: if-shell false condition (exit 1) ---
Write-Host "`n=== TEST 7: if-shell false condition ==="
$out7 = & $psmux if-shell -t shelltest "exit 1" "display-message 'true-branch'" "display-message 'false-branch'" 2>&1 | Out-String
$exit7 = $LASTEXITCODE
Write-Host "if-shell output: $out7"
Write-Host "if-shell exit: $exit7"
Start-Sleep 2
$pass7 = ($exit7 -eq 0)
$results += [PSCustomObject]@{Test="TEST7: if-shell false"; Exit=$exit7; Pass=$pass7; Output=$out7.Trim(); Capture=""}

# --- TEST 8: Verify exit code 0 from test 7 ---
Write-Host "`n=== TEST 8: Verify if-shell false exit code ==="
$pass8 = ($exit7 -eq 0)
$results += [PSCustomObject]@{Test="TEST8: if-shell false exit=0"; Exit=$exit7; Pass=$pass8; Output=""; Capture=""}

# --- TEST 9: PowerShell math 1+1 ---
Write-Host "`n=== TEST 9: run-shell 1+1 ==="
$out9 = & $psmux run-shell -t shelltest "1 + 1" 2>&1 | Out-String
$exit9 = $LASTEXITCODE
Write-Host "run-shell output: $out9"
Write-Host "run-shell exit: $exit9"
Start-Sleep 2
$cap9 = & $psmux capture-pane -t shelltest -p 2>&1 | Out-String
Write-Host "capture-pane:`n$cap9"
$pass9 = ($exit9 -eq 0)
$results += [PSCustomObject]@{Test="TEST9: 1+1 math"; Exit=$exit9; Pass=$pass9; Output=$out9.Trim(); Capture=$cap9.Trim()}

# --- TEST 10: Complex pipeline ---
Write-Host "`n=== TEST 10: run-shell Get-Process pipeline ==="
$out10 = & $psmux run-shell -t shelltest "Get-Process | Select-Object -First 1 | Format-Table Name" 2>&1 | Out-String
$exit10 = $LASTEXITCODE
Write-Host "run-shell output: $out10"
Write-Host "run-shell exit: $exit10"
Start-Sleep 2
$cap10 = & $psmux capture-pane -t shelltest -p 2>&1 | Out-String
Write-Host "capture-pane:`n$cap10"
$pass10 = ($exit10 -eq 0)
$results += [PSCustomObject]@{Test="TEST10: Get-Process pipeline"; Exit=$exit10; Pass=$pass10; Output=$out10.Trim(); Capture=$cap10.Trim()}

# --- TEST 11: Test-Path (PowerShell-only cmdlet) ---
Write-Host "`n=== TEST 11: run-shell Test-Path ==="
$out11 = & $psmux run-shell -t shelltest "Test-Path ." 2>&1 | Out-String
$exit11 = $LASTEXITCODE
Write-Host "run-shell output: $out11"
Write-Host "run-shell exit: $exit11"
Start-Sleep 2
$cap11 = & $psmux capture-pane -t shelltest -p 2>&1 | Out-String
Write-Host "capture-pane:`n$cap11"
$pass11 = ($exit11 -eq 0)
$results += [PSCustomObject]@{Test="TEST11: Test-Path (pwsh-only)"; Exit=$exit11; Pass=$pass11; Output=$out11.Trim(); Capture=$cap11.Trim()}

# --- CLEANUP ---
Write-Host "`n=== CLEANUP ==="
& $psmux kill-session -t shelltest 2>&1
Write-Host "kill-session exit: $LASTEXITCODE"

# --- SUMMARY ---
Write-Host "`n`n=========================================="
Write-Host "           TEST RESULTS SUMMARY"
Write-Host "=========================================="
$passCount = 0
$failCount = 0
foreach ($r in $results) {
    $status = if ($r.Pass) { "PASS" } else { "FAIL" }
    if ($r.Pass) { $passCount++ } else { $failCount++ }
    Write-Host "$status - $($r.Test) (exit=$($r.Exit))"
    if ($r.Output) { Write-Host "  run-shell output: $($r.Output)" }
    if ($r.Capture) {
        $capLines = $r.Capture -split "`n" | Where-Object { $_.Trim() -ne "" } | Select-Object -Last 5
        Write-Host "  capture-pane (last 5 non-empty lines):"
        foreach ($l in $capLines) { Write-Host "    $l" }
    }
}
Write-Host "=========================================="
Write-Host "TOTAL: $passCount PASS, $failCount FAIL out of $($results.Count) tests"
Write-Host "=========================================="
