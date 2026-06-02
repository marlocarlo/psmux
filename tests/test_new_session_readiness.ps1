# Regression test: `new-session -d` must not return until its initial window
# is listable (tmux's "command finished server-side" semantics).
#
# ROOT CAUSE: psmux's server writes its .port file and starts
# accepting connections BEFORE it creates the initial window. The client's
# readiness gate for `new-session -d` waited only for the port file + a TCP
# connect, so it could return exit 0 while `list-windows` was still empty —
# a window-less session that callers and scripts then trip over.
#
# DETERMINISTIC RED via latency injection: the server sleeps
# PSMUX_TEST_WINDOW_DELAY_MS right before create_window (after .port + accept
# thread are up). We set it LONGER than the client read timeout (2000ms), so:
#   - pre-fix : new-session returns immediately; an immediate list-windows
#               blocks on the not-yet-running main loop and times out EMPTY.
#   - post-fix: the client gate polls until the window is listable, so by the
#               time new-session returns, list-windows is non-empty.
# Removal recipe: when the PSMUX_TEST_WINDOW_DELAY_MS hook is deleted from
# server/mod.rs, delete this test (or drop it to a best-effort statistical loop).

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

$psmuxDir = "$env:USERPROFILE\.psmux"
$session  = "rdy_$($PID)_$(Get-Random -Maximum 99999)"
$pass = 0
$fail = 0

function Write-Result($name, $ok, $msg) {
    if ($ok) { Write-Host "  [PASS] $name" -ForegroundColor Green; $script:pass++ }
    else     { Write-Host "  [FAIL] $name : $msg" -ForegroundColor Red; $script:fail++ }
}

# Query a session's own server directly over TCP (independent of CLI target
# resolution). ReadTimeout mirrors the real client's 2000ms RPC timeout.
function Get-ListWindows($sess) {
    $portFile = "$psmuxDir\$sess.port"
    $keyFile  = "$psmuxDir\$sess.key"
    if (-not (Test-Path $portFile)) { return $null }
    $port = (Get-Content $portFile -Raw).Trim()
    $key  = if (Test-Path $keyFile) { (Get-Content $keyFile -Raw).Trim() } else { "" }
    try {
        $tcp = [System.Net.Sockets.TcpClient]::new()
        $tcp.Connect("127.0.0.1", [int]$port)
        $tcp.NoDelay = $true
        $stream = $tcp.GetStream()
        $stream.ReadTimeout = 2000
        $writer = [System.IO.StreamWriter]::new($stream); $writer.AutoFlush = $true
        $reader = [System.IO.StreamReader]::new($stream)
        $writer.WriteLine("AUTH $key")
        $authResp = $reader.ReadLine()   # "OK"
        $writer.WriteLine("list-windows")
        $sb = [System.Text.StringBuilder]::new()
        try { while ($null -ne ($l = $reader.ReadLine())) { [void]$sb.AppendLine($l) } } catch {}
        $tcp.Close()
        return $sb.ToString().Trim()
    } catch {
        return $null
    }
}

function Cleanup {
    & $PSMUX kill-session -t $session 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    Remove-Item "$psmuxDir\$session.*" -Force -EA SilentlyContinue
}

Write-Host ""
Write-Host "=== new-session readiness gate (window listable before return) ===" -ForegroundColor Cyan
Write-Host "  psmux: $PSMUX" -ForegroundColor DarkGray
Write-Host "  session: $session" -ForegroundColor DarkGray

Cleanup

# Force the readiness race wide open: server sleeps 3000ms before creating the
# window; the client read timeout is 2000ms, so an un-gated client loses.
$env:PSMUX_TEST_WINDOW_DELAY_MS = "3000"
$env:PSMUX_NO_WARM = "1"

try {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX new-session -d -s $session 2>&1 | Out-Null
    $code = $LASTEXITCODE
    $sw.Stop()
    $elapsed = $sw.ElapsedMilliseconds

    Write-Result "new-session -d exited 0" ($code -eq 0) "exit=$code"
    Write-Host "  (new-session returned after ${elapsed}ms)" -ForegroundColor DarkGray

    # Guard against a falsely-green pass: the readiness gate is only exercised if
    # the injected 3000ms delay actually fired. PSMUX_TEST_WINDOW_DELAY_MS exists
    # only in debug_assertions builds, so against a release/installed binary it is
    # a no-op, new-session returns fast, and the window is listable anyway. The
    # fixed client must have waited the delay out. thread::sleep is a floor (never
    # short) and the sleep starts after the stopwatch, so elapsed is always >=
    # 3000ms when injection fired (and ~tens of ms when it didn't).
    Write-Result "latency injection active (new-session waited out the injected delay)" `
        ($elapsed -ge 3000) "elapsed=${elapsed}ms < injected 3000ms - latency injection not compiled in; use a debug build"

    # THE assertion: the moment new-session returns, the initial window must be
    # listable. Pre-fix this is empty (client returned too early); post-fix it
    # is non-empty (client gated on window existence).
    $windows = Get-ListWindows $session
    $hasWindow = ($null -ne $windows -and $windows.Length -gt 0)
    Write-Result "initial window listable immediately after new-session returns" `
        $hasWindow "list-windows returned empty/null right after new-session (got: '$windows')"
}
finally {
    Remove-Item Env:\PSMUX_TEST_WINDOW_DELAY_MS -EA SilentlyContinue
    Remove-Item Env:\PSMUX_NO_WARM -EA SilentlyContinue
    Cleanup
}

Write-Host ""
Write-Host "=== Results: $pass passed, $fail failed ===" -ForegroundColor Cyan
if ($fail -gt 0) { exit 1 } else { exit 0 }
