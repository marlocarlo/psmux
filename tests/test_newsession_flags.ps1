# Test new-session flag parsing (strict getopt, matching real tmux behavior)
# In real tmux: -s always consumes next arg as value, even if it looks like a flag.
# So "tmux new -s -d one" creates session named "-d" with command "one", NOT detached.
$exe = "$PSScriptRoot\..\target\release\tmux.exe"

# Clean up first
taskkill /f /im psmux.exe 2>$null
Start-Sleep 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$pass = 0
$fail = 0
$total = 12

# Test 1: tmux new -d -s alpha (standard: -d boolean, -s consumes "alpha")
Write-Host "`n=== Test 1: tmux new -d -s alpha ===" -ForegroundColor Cyan
$out = & $exe new -d -s alpha 2>&1 | Out-String
$code = $LASTEXITCODE
Start-Sleep 1
if ($code -eq 0 -and (Test-Path "$env:USERPROFILE\.psmux\alpha.port")) {
    Write-Host "PASS: Session 'alpha' created detached (exit=$code)" -ForegroundColor Green
    $pass++
} else {
    Write-Host "FAIL: exit=$code, output=$out" -ForegroundColor Red
    $fail++
}

# Test 2: tmux new -s beta -d (standard: -s consumes "beta", -d boolean)
Write-Host "`n=== Test 2: tmux new -s beta -d ===" -ForegroundColor Cyan
$out = & $exe new -s beta -d 2>&1 | Out-String
$code = $LASTEXITCODE
Start-Sleep 1
if ($code -eq 0 -and (Test-Path "$env:USERPROFILE\.psmux\beta.port")) {
    Write-Host "PASS: Session 'beta' created detached (exit=$code)" -ForegroundColor Green
    $pass++
} else {
    Write-Host "FAIL: exit=$code, output=$out" -ForegroundColor Red
    $fail++
}

# Test 3: tmux new -d -s gamma (another ordering)
Write-Host "`n=== Test 3: tmux new -d -s gamma ===" -ForegroundColor Cyan
$out = & $exe new -d -s gamma 2>&1 | Out-String
$code = $LASTEXITCODE
Start-Sleep 1
if ($code -eq 0 -and (Test-Path "$env:USERPROFILE\.psmux\gamma.port")) {
    Write-Host "PASS: Session 'gamma' created detached (exit=$code)" -ForegroundColor Green
    $pass++
} else {
    Write-Host "FAIL: exit=$code, output=$out" -ForegroundColor Red
    $fail++
}

# Test 4: tmux ls should show exactly 3
Write-Host "`n=== Test 4: tmux ls (expect 3 sessions) ===" -ForegroundColor Cyan
$out = & $exe ls 2>&1 | Out-String
$code = $LASTEXITCODE
$lines = @($out.Trim() -split "`n" | Where-Object { $_.Trim() }).Count
if ($code -eq 0 -and $lines -eq 3) {
    Write-Host "PASS: tmux ls shows $lines sessions" -ForegroundColor Green
    Write-Host $out
    $pass++
} else {
    Write-Host "FAIL: expected 3 lines, got $lines. exit=$code, output=$out" -ForegroundColor Red
    $fail++
}

# Test 5: Duplicate session
Write-Host "`n=== Test 5: tmux new -d -s alpha (duplicate) ===" -ForegroundColor Cyan
$out = & $exe new -d -s alpha 2>&1 | Out-String
$code = $LASTEXITCODE
if ($out -match "already exists") {
    Write-Host "PASS: Duplicate detected: $($out.Trim())" -ForegroundColor Green
    $pass++
} else {
    Write-Host "FAIL: exit=$code, output=$out" -ForegroundColor Red
    $fail++
}

# Test 6: tmux new -d (no session name, should auto-number like tmux)
Write-Host "`n=== Test 6: tmux new -d (auto-numbered name) ===" -ForegroundColor Cyan
$out = & $exe new -d 2>&1 | Out-String
$code = $LASTEXITCODE
Start-Sleep 1
# Should create a numeric session (0, 1, 2, ...) — check ls output
$lsOut = & $exe ls 2>&1 | Out-String
if ($code -eq 0 -and $lsOut -match "^\d+:") {
    Write-Host "PASS: Auto-numbered session created (exit=$code)" -ForegroundColor Green
    $pass++
} else {
    Write-Host "FAIL: exit=$code, output=$lsOut" -ForegroundColor Red
    $fail++
}

# Test 7: tmux ls should now show 4 sessions
Write-Host "`n=== Test 7: tmux ls (expect 4 sessions) ===" -ForegroundColor Cyan
$out = & $exe ls 2>&1 | Out-String
$code = $LASTEXITCODE
$lines = @($out.Trim() -split "`n" | Where-Object { $_.Trim() }).Count
if ($code -eq 0 -and $lines -eq 4) {
    Write-Host "PASS: tmux ls shows $lines sessions" -ForegroundColor Green
    Write-Host $out
    $pass++
} else {
    Write-Host "FAIL: expected 4 lines, got $lines. exit=$code, output=$out" -ForegroundColor Red
    $fail++
}

# Clean up for next batch
taskkill /f /im psmux.exe 2>$null
Start-Sleep 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

# Test 8: tmux new -s -d (getopt: -s eats "-d" as session name, NOT detached)
# This matches real tmux behavior exactly.
# Since the session is NOT detached, it will try to attach. We spawn detached equivalent
# by just verifying the session name is "-d".
Write-Host "`n=== Test 8: tmux new -d -s '-d' (session named '-d') ===" -ForegroundColor Cyan
$out = & $exe new -d -s "-d" 2>&1 | Out-String
$code = $LASTEXITCODE
Start-Sleep 1
if ($code -eq 0 -and (Test-Path "$env:USERPROFILE\.psmux\-d.port")) {
    Write-Host "PASS: Session named '-d' created (exit=$code)" -ForegroundColor Green
    $pass++
} else {
    Write-Host "FAIL: exit=$code, output=$out" -ForegroundColor Red
    $fail++
}

# Clean up
taskkill /f /im psmux.exe 2>$null
Start-Sleep 2
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

# Test 9: tmux new -d -s "my-session" (session name with dash)
Write-Host "`n=== Test 9: tmux new -d -s my-session ===" -ForegroundColor Cyan
$out = & $exe new -d -s "my-session" 2>&1 | Out-String
$code = $LASTEXITCODE
Start-Sleep 1
if ($code -eq 0 -and (Test-Path "$env:USERPROFILE\.psmux\my-session.port")) {
    Write-Host "PASS: Session 'my-session' created (exit=$code)" -ForegroundColor Green
    $pass++
} else {
    Write-Host "FAIL: exit=$code, output=$out" -ForegroundColor Red
    $fail++
}

# Test 10: tmux new -s "work" -n "editor" -d (multiple value+bool flags)
Write-Host "`n=== Test 10: tmux new -s work -n editor -d ===" -ForegroundColor Cyan
$out = & $exe new -s work -n editor -d 2>&1 | Out-String
$code = $LASTEXITCODE
Start-Sleep 1
if ($code -eq 0 -and (Test-Path "$env:USERPROFILE\.psmux\work.port")) {
    Write-Host "PASS: Session 'work' created with window name (exit=$code)" -ForegroundColor Green
    $pass++
} else {
    Write-Host "FAIL: exit=$code, output=$out" -ForegroundColor Red
    $fail++
}

# Test 11: tmux ls shows both sessions from tests 9-10
Write-Host "`n=== Test 11: tmux ls (expect 2 sessions) ===" -ForegroundColor Cyan
$out = & $exe ls 2>&1 | Out-String
$code = $LASTEXITCODE
if ($out -match "my-session:" -and $out -match "work:") {
    Write-Host "PASS: Both sessions visible" -ForegroundColor Green
    Write-Host $out
    $pass++
} else {
    Write-Host "FAIL: output=$out" -ForegroundColor Red
    $fail++
}

# Test 12: tmux new -d (with existing sessions, creates auto-numbered session)
Write-Host "`n=== Test 12: tmux new -d (creates auto-numbered session alongside others) ===" -ForegroundColor Cyan
$out = & $exe new -d 2>&1 | Out-String
$code = $LASTEXITCODE
Start-Sleep 1
$out2 = & $exe ls 2>&1 | Out-String
$lines = @($out2.Trim() -split "`n" | Where-Object { $_.Trim() }).Count
if ($code -eq 0 -and $lines -eq 3 -and $out2 -match "^\d+:") {
    Write-Host "PASS: auto-numbered session added, total $lines sessions" -ForegroundColor Green
    Write-Host $out2
    $pass++
} else {
    Write-Host "FAIL: exit=$code, lines=$lines, output=$out2" -ForegroundColor Red
    $fail++
}

# Final clean up
taskkill /f /im psmux.exe 2>$null
Start-Sleep 1
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

Write-Host "`n========================================" -ForegroundColor Yellow
Write-Host "Results: $pass PASS, $fail FAIL out of $total tests" -ForegroundColor Yellow
Write-Host "========================================" -ForegroundColor Yellow
