# Regression test: `new-session -d` must WAIT for a slow-but-healthy server
# instead of giving up (and orphaning it) on a tight fixed timeout.
#
# ROOT CAUSE: under load a healthy server can simply be slow to come up — slow
# to write its .port file, or slow to accept the first TCP connection. That
# slowness is normal, not a failure.
#
# THE BUG (pre-fix): the client's startup gate allowed only a tight, fixed budget
# — a 5s .port poll, then a single 100ms connect attempt — and mistook anything
# slower for death. It gave up with rc=1 ("failed to create session" / "exited
# immediately") on a server that was alive and about to be ready — and in the
# connect-miss branch it even deleted the .port file, orphaning the live server.
# Observed in 38 of 6000 sessions under load, every one leaving a live server
# behind. (Post-fix replaces both limits with one 15s bounded readiness wait.)
#
# DETERMINISTIC RED via latency injection: PSMUX_TEST_PORTFILE_DELAY_MS makes the
# server sleep before writing .port while otherwise healthy. Set LONGER than the
# old 5s port poll, so:
#   - pre-fix : the client's 5s poll expires -> rc=1, even though the server is
#               alive and writes .port shortly after (orphan).
#   - post-fix: the client's bounded readiness wait keeps waiting, the server
#               comes up, and new-session returns rc=0 with a listable window.
# Removal recipe: when PSMUX_TEST_PORTFILE_DELAY_MS is deleted from server/mod.rs,
# delete this test.

$ErrorActionPreference = "Stop"
# Prefer the local build under test (debug carries the latency-injection hooks);
# do NOT fall back to a psmux on PATH, which is typically an installed release
# binary where the injection is a no-op and the test would pass for free.
$PSMUX = $env:PSMUX_EXE
if (-not $PSMUX -or -not (Test-Path $PSMUX)) { $PSMUX = "$PSScriptRoot\..\target\debug\psmux.exe" }
if (-not (Test-Path $PSMUX)) { $PSMUX = "$PSScriptRoot\..\target\release\psmux.exe" }
if (-not (Test-Path $PSMUX)) {
    Write-Host "FATAL: could not resolve psmux executable ($PSMUX)" -ForegroundColor Red
    exit 1
}

# Isolate into a throwaway home so we never touch the real ~/.psmux and any
# orphan is contained. The spawned server inherits this via the env (CreateProcessW).
$tmpHome = Join-Path $env:TEMP ("psmux_noorphan_" + [guid]::NewGuid().ToString("N").Substring(0,8))
New-Item -ItemType Directory -Path $tmpHome -Force | Out-Null
$ns = "noorphan_" + [guid]::NewGuid().ToString("N").Substring(0,8)
$session = "d"
$sdir = Join-Path $tmpHome "SDIR"
New-Item -ItemType Directory -Path $sdir -Force | Out-Null

$pass = 0
$fail = 0
function Write-Result($name, $ok, $msg) {
    if ($ok) { Write-Host "  [PASS] $name" -ForegroundColor Green; $script:pass++ }
    else     { Write-Host "  [FAIL] $name : $msg" -ForegroundColor Red; $script:fail++ }
}

function Get-ListWindows($homeRoot, $portBase) {
    # With `-L <ns>` psmux prefixes the discovery files as <ns>__<session>.port
    $portFile = "$homeRoot\.psmux\$portBase.port"
    $keyFile  = "$homeRoot\.psmux\$portBase.key"
    if (-not (Test-Path $portFile)) { return $null }
    $port = (Get-Content $portFile -Raw).Trim()
    $key  = if (Test-Path $keyFile) { (Get-Content $keyFile -Raw).Trim() } else { "" }
    try {
        $tcp = [System.Net.Sockets.TcpClient]::new()
        $tcp.Connect("127.0.0.1", [int]$port)
        $stream = $tcp.GetStream(); $stream.ReadTimeout = 2000
        $w = [System.IO.StreamWriter]::new($stream); $w.AutoFlush = $true
        $r = [System.IO.StreamReader]::new($stream)
        $w.WriteLine("AUTH $key"); $r.ReadLine() | Out-Null
        $w.WriteLine("list-windows")
        $sb = [System.Text.StringBuilder]::new()
        try { while ($null -ne ($l = $r.ReadLine())) { [void]$sb.AppendLine($l) } } catch {}
        $tcp.Close()
        return $sb.ToString().Trim()
    } catch { return $null }
}

Write-Host ""
Write-Host "=== new-session waits for a slow server (no early give-up / orphan) ===" -ForegroundColor Cyan
Write-Host "  psmux: $PSMUX" -ForegroundColor DarkGray
Write-Host "  home : $tmpHome   ns: $ns" -ForegroundColor DarkGray

# Make the server slow to write its .port file (7s > the old 5s client poll).
$env:USERPROFILE = $tmpHome
$env:HOME = $tmpHome
$env:PSMUX_TEST_PORTFILE_DELAY_MS = "7000"
$env:PSMUX_NO_WARM = "1"
$env:PSMUX_CONFIG_FILE = "NUL"

try {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX -L $ns new-session -d -s $session -c $sdir 2>&1 | Out-Null
    $code = $LASTEXITCODE
    $sw.Stop()
    $elapsed = $sw.ElapsedMilliseconds
    Write-Host "  (new-session returned after ${elapsed}ms, rc=$code)" -ForegroundColor DarkGray

    # Guard against a falsely-green pass: the orphan race is only exercised if the
    # injected 7000ms .port delay actually fired. PSMUX_TEST_PORTFILE_DELAY_MS
    # exists only in debug_assertions builds, so against a release/installed binary
    # it is a no-op and new-session returns fast. thread::sleep is a floor and the
    # sleep starts after the stopwatch, so elapsed >= 7000ms when injection fired.
    Write-Result "latency injection active (new-session waited out the injected delay)" `
        ($elapsed -ge 7000) "elapsed=${elapsed}ms < injected 7000ms - latency injection not compiled in; use a debug build"

    Write-Result "new-session -d waited for the slow server (rc=0)" ($code -eq 0) `
        "rc=$code — client gave up on a slow-but-healthy server"

    $windows = Get-ListWindows $tmpHome "${ns}__${session}"
    $hasWindow = ($null -ne $windows -and $windows.Length -gt 0)
    Write-Result "initial window listable after new-session returns" $hasWindow `
        "list-windows empty/null (got: '$windows')"
}
finally {
    Remove-Item Env:\PSMUX_TEST_PORTFILE_DELAY_MS -EA SilentlyContinue
    Remove-Item Env:\PSMUX_NO_WARM -EA SilentlyContinue
    Remove-Item Env:\PSMUX_CONFIG_FILE -EA SilentlyContinue
    & $PSMUX -L $ns kill-server 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    # Backstop: kill any server still bound to this ns home, then remove temp home.
    Get-CimInstance Win32_Process -Filter "Name='psmux.exe'" -EA SilentlyContinue |
        Where-Object { $_.CommandLine -match [regex]::Escape($ns) } |
        ForEach-Object { Stop-Process -Id $_.ProcessId -Force -EA SilentlyContinue }
    Remove-Item $tmpHome -Recurse -Force -EA SilentlyContinue
}

Write-Host ""
Write-Host "=== Results: $pass passed, $fail failed ===" -ForegroundColor Cyan
if ($fail -gt 0) { exit 1 } else { exit 0 }
