# Shared, non-destructive helpers for the psmux PowerShell test suite.
#
# Dot-source:  . "$PSScriptRoot\psmux_test_helpers.ps1"
#
# Isolation model — safe to run on a machine that also runs psmux for real:
#   * New-PsmuxTestEnv points USERPROFILE *and* HOME at a throwaway temp dir, so
#     the data dir (~/.psmux: sockets, .port/.key files, warm pool), config, and
#     everything home-relative live under that dir. The real ~/.psmux is never
#     touched, and the prior USERPROFILE/HOME are saved for restore.
#   * It also scrubs the inherited session/target vars (PSMUX_SESSION,
#     PSMUX_ACTIVE, ...). Run from inside a psmux session those would otherwise
#     trip the nesting guard, so `new-session` never spawns a real server and
#     the test silently no-ops. Saved and restored alongside USERPROFILE/HOME.
#   * Remove-PsmuxTestEnv shuts servers down namespace-scoped (`-L <ns>
#     kill-server`, NEVER a bare kill-server, which has a nuclear kill-all-by-
#     image-name fallback), force-nets only processes whose ExecutablePath is the
#     binary under test (so an installed psmux at another path is never killed),
#     restores USERPROFILE/HOME, then deletes the temp dir.
#
# What this does NOT cover: image-name process kills against an installed psmux
# that happens to live at the SAME path as the binary under test, and TUI input
# injection (keybd_event / SetForegroundWindow). Those need separate handling.

# Vars a live psmux session leaks into a pane shell that a spawned psmux would
# then misread. This is exactly the set observed in an interactive session's env;
# each affects real behavior, so New-PsmuxTestEnv scrubs and later restores them:
#   PSMUX_SESSION        -> non-empty trips the nesting guard (main.rs:769-771):
#                           a spawned psmux refuses, so new-session never starts a
#                           real server.
#   PSMUX_TARGET_SESSION -> retargets commands at the caller's session.
#   TMUX                 -> targeting fallback when no -t / no PSMUX_TARGET_SESSION
#                           (main.rs:267); also the tmux-compat nesting signal that
#                           child tools read.
#   TMUX_PANE            -> tmux-compat pane identity.
# Deliberately NOT scrubbed:
#   PSMUX_ACTIVE         -> the guard's other half, but it is an in-process marker
#                           set on the attaching client (main.rs:3789) and never
#                           put on a pane shell, so it cannot reach a test.
#   PSMUX_SESSION_NAME / PSMUX_TARGET_FULL -> psmux sets these only in its own
#                           process for its children; they do not leak to the shell.
$script:PsmuxIsolatedVars = @(
    'PSMUX_SESSION', 'PSMUX_TARGET_SESSION', 'TMUX', 'TMUX_PANE'
)

# Resolve the psmux binary under test: PSMUX_EXE override, then the repo's
# target\release, then target\debug. Throws if none is found.
function Get-PsmuxExe {
    param([string]$TestsRoot = $PSScriptRoot)
    $candidates = @()
    if ($env:PSMUX_EXE) { $candidates += $env:PSMUX_EXE }
    $candidates += (Join-Path $TestsRoot '..\target\release\psmux.exe')
    $candidates += (Join-Path $TestsRoot '..\target\debug\psmux.exe')
    foreach ($c in $candidates) {
        if ($c -and (Test-Path $c)) { return (Resolve-Path $c).Path }
    }
    throw "psmux executable not found (set PSMUX_EXE, or build target\release|debug\psmux.exe)"
}

# Create an isolated, throwaway environment and switch USERPROFILE/HOME to it.
# Returns a context object to pass to the other helpers and to Remove-PsmuxTestEnv.
function New-PsmuxTestEnv {
    param(
        [string]$Tag = 'psmux',
        [string]$Exe = (Get-PsmuxExe)
    )
    $root = Join-Path $env:TEMP ("psmux_${Tag}_" + [guid]::NewGuid().ToString('N').Substring(0, 8))
    $dataDir = Join-Path $root '.psmux'
    New-Item -ItemType Directory -Path $dataDir -Force | Out-Null

    $saved = @{ USERPROFILE = $env:USERPROFILE; HOME = $env:HOME }
    foreach ($v in $script:PsmuxIsolatedVars) {
        $saved[$v] = (Get-Item "Env:\$v" -ErrorAction SilentlyContinue).Value
    }
    $ctx = [pscustomobject]@{
        Home       = $root
        PsmuxDir   = $dataDir
        PsmuxExe   = $Exe
        Namespaces = [System.Collections.Generic.HashSet[string]]::new()
        SavedEnv   = $saved
    }
    $env:USERPROFILE = $root
    $env:HOME = $root
    foreach ($v in $script:PsmuxIsolatedVars) { Remove-Item "Env:\$v" -ErrorAction SilentlyContinue }
    return $ctx
}

# Record a `-L` namespace the test used, so teardown can shut its server down
# scoped. Returns the namespace for convenient inline use.
function Register-PsmuxNamespace {
    param([Parameter(Mandatory)]$Ctx, [Parameter(Mandatory)][string]$Namespace)
    [void]$Ctx.Namespaces.Add($Namespace)
    return $Namespace
}

# Tear the environment down non-destructively: scoped server shutdown, a
# build-binary-scoped force-net for any orphans, env restore, temp dir removal.
# Safe to call from a finally block. Extra -Namespace values are merged in.
function Remove-PsmuxTestEnv {
    param(
        [Parameter(Mandatory)]$Ctx,
        [string[]]$Namespace = @()
    )
    foreach ($n in $Namespace) { [void]$Ctx.Namespaces.Add($n) }

    # 1. Graceful, per-namespace shutdown. Never a bare kill-server.
    foreach ($n in $Ctx.Namespaces) {
        & $Ctx.PsmuxExe -L $n kill-server 2>&1 | Out-Null
    }

    # 2. Force-net any orphan: only processes running THIS binary and carrying
    #    one of our namespaces on the command line. An installed psmux at a
    #    different path never matches.
    if ($Ctx.Namespaces.Count -gt 0) {
        $nsPattern = ($Ctx.Namespaces | ForEach-Object { [regex]::Escape($_) }) -join '|'
        Get-CimInstance Win32_Process -Filter "Name='psmux.exe'" -ErrorAction SilentlyContinue |
            Where-Object {
                $_.ExecutablePath -and ($_.ExecutablePath -ieq $Ctx.PsmuxExe) -and
                $_.CommandLine -and ($_.CommandLine -match $nsPattern)
            } |
            ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }
        Start-Sleep -Milliseconds 300
    }

    # 3. Restore every managed env var (USERPROFILE/HOME + scrubbed session vars)
    #    to its prior value, then remove the throwaway home.
    foreach ($k in $Ctx.SavedEnv.Keys) {
        $val = $Ctx.SavedEnv[$k]
        if ($null -eq $val) { Remove-Item "Env:\$k" -ErrorAction SilentlyContinue }
        else { Set-Item "Env:\$k" -Value $val }
    }
    Remove-Item -Recurse -Force $Ctx.Home -ErrorAction SilentlyContinue
}

# Connect to a session's server over the control-mode TCP port (discovered from
# the isolated data dir) and return the `list-windows` response, or $null if the
# port file is missing or the server does not answer.
function Invoke-PsmuxListWindows {
    param(
        [Parameter(Mandatory)]$Ctx,
        [Parameter(Mandatory)][string]$Base,
        [int]$ReadTimeoutMs = 2000
    )
    $portFile = Join-Path $Ctx.PsmuxDir "$Base.port"
    $keyFile = Join-Path $Ctx.PsmuxDir "$Base.key"
    if (-not (Test-Path $portFile)) { return $null }
    $port = (Get-Content $portFile -Raw).Trim()
    $key = if (Test-Path $keyFile) { (Get-Content $keyFile -Raw).Trim() } else { "" }
    try {
        $tcp = [System.Net.Sockets.TcpClient]::new()
        $tcp.Connect("127.0.0.1", [int]$port); $tcp.NoDelay = $true
        $stream = $tcp.GetStream(); $stream.ReadTimeout = $ReadTimeoutMs
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
