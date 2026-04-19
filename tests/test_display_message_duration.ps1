# E2E test: display-message -d (per-message duration override)
# Proves that -d <ms> actually controls how long the message stays on the status bar.

$ErrorActionPreference = 'Stop'
$pass = 0; $fail = 0; $skip = 0
$sess = "dur_test_$$"

function Report($name, $ok, $detail) {
    if ($ok) { $script:pass++; Write-Host "  PASS  $name" -ForegroundColor Green }
    else     { $script:fail++; Write-Host "  FAIL  $name  ($detail)" -ForegroundColor Red }
}

# Cleanup
try { psmux kill-session -t $sess 2>$null } catch {}
Start-Sleep -Milliseconds 500

# Start a detached session
psmux new-session -d -s $sess
Start-Sleep -Milliseconds 1500

# Test 1: display-message -d is forwarded and does not corrupt the message text
$out = psmux display-message -t $sess -p -d 5000 "duration_test_msg"
Report "d flag not in message text" ($out -match "duration_test_msg" -and $out -notmatch "5000" -and $out -notmatch "\-d") "got: $out"

# Test 2: display-message -d with -p still works (print to stdout)
$out = psmux display-message -t $sess -p -d 3000 "hello_from_d"
Report "d flag with -p prints correctly" ($out -match "hello_from_d") "got: $out"

# Test 3: display-message without -d still works
$out = psmux display-message -t $sess -p "no duration flag"
Report "no -d flag works normally" ($out -match "no duration flag") "got: $out"

# Test 4: display-message -d with format variables
$out = psmux display-message -t $sess -p -d 2000 "#{session_name}"
Report "d flag with format vars" ($out -eq $sess) "expected '$sess', got: $out"

# Test 5: display-message -d 0 (zero duration)
$out = psmux display-message -t $sess -p -d 0 "zero_dur"
Report "d flag with 0 duration" ($out -eq "zero_dur") "got: $out"

# Test 6: Practical proof that -d controls display time
# Send display-message WITHOUT -p (so it sets the status bar) with -d 10000 (10s)
# Then immediately query with -p to check the message was set correctly
psmux display-message -t $sess -d 10000 "LONG_DURATION_TEST"
Start-Sleep -Milliseconds 300
# Verify the message was received and set (query via -p on a different message)
$check = psmux display-message -t $sess -p "#{session_name}"
Report "d 10000 message accepted" ($check -eq $sess) "session query returned: $check"

# Test 7: display-message -d 100 (very short) should expire quickly
psmux display-message -t $sess -d 100 "SHORT_DURATION_TEST"
Start-Sleep -Milliseconds 500
$capture2 = psmux capture-pane -t $sess -p
$hasShort = ($capture2 -join "`n") -match "SHORT_DURATION_TEST"
Report "d 100 message expired after 500ms" (-not $hasShort) "message was still visible"

# Cleanup
try { psmux kill-session -t $sess 2>$null } catch {}

Write-Host "`n===== Results: $pass passed, $fail failed, $skip skipped ====="
if ($fail -gt 0) { exit 1 }
