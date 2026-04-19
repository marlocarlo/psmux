#!/usr/bin/env pwsh
# Test for issue #205: new-session -e environment variable support
# Verifies that -e KEY=VALUE sets session environment and pane inheritance.

$ErrorActionPreference = "Continue"
$results = @()

function Add-Result($name, $pass, $detail="") {
    $script:results += [PSCustomObject]@{
        Test=$name
        Result=if($pass){"PASS"}else{"FAIL"}
        Detail=$detail
    }
}

$SESSION = "test205_$$"

try {
    # Clean up any leftover session
    psmux kill-session -t $SESSION 2>$null
    Start-Sleep -Milliseconds 500

    # ---- Test 1: Single -e flag creates session with env var ----
    psmux new-session -d -s $SESSION -e "TEST205_VAR=hello_world"
    Start-Sleep -Seconds 3
    $env_out = psmux show-environment -t $SESSION 2>&1 | Out-String
    $pass = $env_out -match "TEST205_VAR=hello_world"
    Add-Result "single -e in show-environment" $pass "Output: $($env_out.Trim())"
    psmux kill-session -t $SESSION 2>$null
    Start-Sleep -Seconds 1

    # ---- Test 2: Multiple -e flags ----
    $SESSION2 = "${SESSION}_multi"
    psmux new-session -d -s $SESSION2 -e "VAR_A=alpha" -e "VAR_B=beta" -e "VAR_C=gamma"
    Start-Sleep -Seconds 3
    $env_out = psmux show-environment -t $SESSION2 2>&1 | Out-String
    $pass_a = $env_out -match "VAR_A=alpha"
    $pass_b = $env_out -match "VAR_B=beta"
    $pass_c = $env_out -match "VAR_C=gamma"
    Add-Result "multiple -e: VAR_A" $pass_a "Found VAR_A=alpha: $pass_a"
    Add-Result "multiple -e: VAR_B" $pass_b "Found VAR_B=beta: $pass_b"
    Add-Result "multiple -e: VAR_C" $pass_c "Found VAR_C=gamma: $pass_c"

    # ---- Test 3: Pane inherits env vars ----
    psmux send-keys -t $SESSION2 'echo "INHERITED=$env:VAR_A"' Enter
    Start-Sleep -Seconds 2
    $pane_out = psmux capture-pane -t $SESSION2 -p 2>&1 | Out-String
    $pass = $pane_out -match "INHERITED=alpha"
    Add-Result "pane inherits -e vars" $pass "Pane output contains INHERITED=alpha: $pass"
    psmux kill-session -t $SESSION2 2>$null
    Start-Sleep -Seconds 1

    # ---- Test 4: Value with equals sign ----
    $SESSION3 = "${SESSION}_eq"
    psmux new-session -d -s $SESSION3 -e "COMPLEX=a=b=c"
    Start-Sleep -Seconds 3
    $env_out = psmux show-environment -t $SESSION3 2>&1 | Out-String
    $pass = $env_out -match "COMPLEX=a=b=c"
    Add-Result "value with equals sign" $pass "Found COMPLEX=a=b=c: $pass"
    psmux kill-session -t $SESSION3 2>$null
    Start-Sleep -Seconds 1

    # ---- Test 5: Empty value ----
    $SESSION4 = "${SESSION}_empty"
    psmux new-session -d -s $SESSION4 -e "EMPTY_VAR="
    Start-Sleep -Seconds 3
    $env_out = psmux show-environment -t $SESSION4 2>&1 | Out-String
    $pass = $env_out -match "EMPTY_VAR="
    Add-Result "empty value" $pass "Found EMPTY_VAR=: $pass"
    psmux kill-session -t $SESSION4 2>$null
    Start-Sleep -Seconds 1

    # ---- Test 6: Invalid env var name rejected ----
    $err = psmux new-session -d -s "${SESSION}_bad" -e "123BAD=x" 2>&1 | Out-String
    $pass = $err -match "invalid" -and $err -match "environment variable"
    Add-Result "rejects invalid var name" $pass "Error: $($err.Trim())"
    psmux kill-session -t "${SESSION}_bad" 2>$null

    # ---- Test 7: Missing value rejected ----
    $err = psmux new-session -d -s "${SESSION}_noval" -e "NOEQUALS" 2>&1 | Out-String
    $pass = $err -match "expected VARIABLE=value" -or $err -match "invalid"
    Add-Result "rejects missing equals" $pass "Error: $($err.Trim())"
    psmux kill-session -t "${SESSION}_noval" 2>$null

    # ---- Test 8: Duplicate key last wins ----
    $SESSION5 = "${SESSION}_dup"
    psmux new-session -d -s $SESSION5 -e "DUP_KEY=first" -e "DUP_KEY=last"
    Start-Sleep -Seconds 3
    $env_out = psmux show-environment -t $SESSION5 2>&1 | Out-String
    $pass = $env_out -match "DUP_KEY=last" -and $env_out -notmatch "DUP_KEY=first"
    Add-Result "duplicate key last wins" $pass "Output: $($env_out.Trim())"
    psmux kill-session -t $SESSION5 2>$null

} finally {
    # Cleanup all test sessions
    psmux kill-session -t $SESSION 2>$null
    psmux kill-session -t "${SESSION}_multi" 2>$null
    psmux kill-session -t "${SESSION}_eq" 2>$null
    psmux kill-session -t "${SESSION}_empty" 2>$null
    psmux kill-session -t "${SESSION}_bad" 2>$null
    psmux kill-session -t "${SESSION}_noval" 2>$null
    psmux kill-session -t "${SESSION}_dup" 2>$null
}

# Summary
Write-Host "`n=== Issue #205: new-session -e Test Results ===" -ForegroundColor Cyan
$results | Format-Table -AutoSize
$failed = ($results | Where-Object { $_.Result -eq "FAIL" }).Count
$total = $results.Count
$passed = $total - $failed
Write-Host "Total: $total | Passed: $passed | Failed: $failed" -ForegroundColor $(if($failed -gt 0){"Red"}else{"Green"})
if ($failed -gt 0) { exit 1 }
