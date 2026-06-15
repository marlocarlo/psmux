# Zero-hang regression: `new-session -d` must NEVER block indefinitely on a
# doomed server, and must fail FAST when the server is provably dead.
#
# Two scenarios, both via debug-only fault injection (compiled out of release):
#
#  H1 DEAD SERVER, NO CLEANUP (PSMUX_TEST_DIE_AFTER_PORTFILE):
#     The server writes its .port file then hard-exits WITHOUT running its panic
#     hook, leaving a stale .port. The pre-fix client would loop until its 15s
#     deadline (.port never vanishes, connect never succeeds). The fixed client
#     polls the spawned server PID and fails fast the instant it dies.
#     ASSERT: rc != 0 AND elapsed well under the 15s deadline (< 5s).
#
#  H2 ALIVE BUT HUNG create_window (PSMUX_TEST_WINDOW_DELAY_MS = 60000):
#     The server is alive, .port written, accept loop up, but create_window is
#     stuck "forever" (60s >> 15s). list-windows stays empty and the PID stays
#     alive, so no fast-fail signal applies. The client must still return at its
#     bounded 15s deadline, never block for 60s or indefinitely.
#     ASSERT: rc != 0 AND elapsed in [12s, 20s] (bounded, not 60s, not infinite).
#
# Removal recipe: if the PSMUX_TEST_DIE_AFTER_PORTFILE / PSMUX_TEST_WINDOW_DELAY_MS
# hooks are deleted from server/mod.rs, delete this test.

$ErrorActionPreference = "Stop"
. "$PSScriptRoot\psmux_test_helpers.ps1"

$PSMUX = Get-PsmuxExe
$pass = 0; $fail = 0
function Write-Result($name, $ok, $msg) {
    if ($ok) { Write-Host "  [PASS] $name" -ForegroundColor Green; $script:pass++ }
    else     { Write-Host "  [FAIL] $name : $msg" -ForegroundColor Red; $script:fail++ }
}

function Run-Scenario {
    param([string]$Tag, [hashtable]$EnvVars)
    $ctx = New-PsmuxTestEnv -Tag "hang_$Tag" -Exe $PSMUX
    $ns = Register-PsmuxNamespace -Ctx $ctx -Namespace "hang_$Tag"; $sn = $ns
    $env:PSMUX_NO_WARM = "1"
    foreach ($k in $EnvVars.Keys) { Set-Item -Path "Env:\$k" -Value $EnvVars[$k] }
    try {
        $sw = [System.Diagnostics.Stopwatch]::StartNew()
        & $PSMUX -L $ns new-session -d -s $sn 2>&1 | Out-Null
        $rc = $LASTEXITCODE
        $sw.Stop()
    }
    finally {
        foreach ($k in $EnvVars.Keys) { Remove-Item -Path "Env:\$k" -EA SilentlyContinue }
        Remove-Item Env:\PSMUX_NO_WARM -EA SilentlyContinue
        Remove-PsmuxTestEnv -Ctx $ctx
    }
    return @{ rc = $rc; ms = $sw.ElapsedMilliseconds }
}

Write-Host ""
Write-Host "=== Zero-hang: new-session -d never blocks on a doomed server ===" -ForegroundColor Cyan
Write-Host "  psmux: $PSMUX" -ForegroundColor DarkGray

# --- H1: dead server, no cleanup -> must fail FAST (PID death detection) ---
Write-Host "`n[H1] server dies after writing .port (no panic cleanup)" -ForegroundColor Yellow
$h1 = Run-Scenario -Tag "die" -EnvVars @{ PSMUX_TEST_DIE_AFTER_PORTFILE = "1" }
Write-Host ("  new-session returned in {0}ms, rc={1}" -f $h1.ms, $h1.rc) -ForegroundColor DarkGray
Write-Result "H1 reports failure (rc != 0)" ($h1.rc -ne 0) "rc=$($h1.rc)"
Write-Result "H1 fails FAST (< 5000ms, not the 15s deadline)" ($h1.ms -lt 5000) "elapsed=$($h1.ms)ms - PID death not detected, fell through to deadline"

# --- H2: alive but create_window hangs forever -> must return at bounded deadline ---
Write-Host "`n[H2] server alive but create_window hangs 60s (>> 15s deadline)" -ForegroundColor Yellow
$h2 = Run-Scenario -Tag "stuck" -EnvVars @{ PSMUX_TEST_WINDOW_DELAY_MS = "60000" }
Write-Host ("  new-session returned in {0}ms, rc={1}" -f $h2.ms, $h2.rc) -ForegroundColor DarkGray
Write-Result "H2 reports failure (rc != 0)" ($h2.rc -ne 0) "rc=$($h2.rc)"
Write-Result "H2 is BOUNDED (returned in 12-20s, not 60s, not infinite)" (($h2.ms -ge 12000) -and ($h2.ms -le 20000)) "elapsed=$($h2.ms)ms"

Write-Host ""
Write-Host "=== Results: $pass passed, $fail failed ===" -ForegroundColor Cyan
if ($fail -gt 0) { exit 1 } else { exit 0 }
