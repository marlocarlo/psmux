#requires -Version 7.0
<#
.SYNOPSIS
Bounded, private lifecycle audit against one explicitly identified new build.
.DESCRIPTION
Creates one disposable server in target/audit with its own data directory,
namespace, and config. USERPROFILE and HOME are unchanged. Never resolves an
installed psmux, runs a global kill command, or deletes evidence. A successful
audit requires rejected commands to preserve the server and its original panes.
Only the supplied executable is ever invoked.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][string]$PsmuxExe,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-fA-F]{64}$')][string]$ExpectedSha256
)
$ErrorActionPreference = 'Stop'
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$PsmuxExe = (Get-Item -LiteralPath $PsmuxExe -ErrorAction Stop).FullName
if ([IO.Path]::GetFileName($PsmuxExe) -ne 'psmux.exe') { throw 'Use the canonical new psmux.exe, not an installed alias or renamed executable.' }
$actualHash = (Get-FileHash -LiteralPath $PsmuxExe -Algorithm SHA256).Hash
if ($actualHash -ne $ExpectedSha256) { throw 'Executable SHA256 does not match the explicitly supplied new build.' }

# Hold native handles from identity capture through cleanup. Termination uses
# that SAME handle, never a fresh lookup of a potentially recycled process ID.
if (-not ('PsmuxLifecycleAuditHandleV1' -as [type])) {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public sealed class PsmuxLifecycleAuditHandleV1 : IDisposable {
    [DllImport("kernel32.dll", SetLastError=true)] static extern IntPtr OpenProcess(uint access, bool inherit, uint pid);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool GetProcessTimes(IntPtr h, out long created, out long exited, out long kernel, out long user);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool TerminateProcess(IntPtr h, uint exitCode);
    [DllImport("kernel32.dll")] static extern uint WaitForSingleObject(IntPtr h, uint milliseconds);
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr h);
    IntPtr handle;
    public uint Pid { get; private set; }
    public long CreatedFileTime { get; private set; }
    public bool IsAlive { get { return handle != IntPtr.Zero && WaitForSingleObject(handle, 0) == 258; } }
    public static PsmuxLifecycleAuditHandleV1 Open(uint pid) {
        // SYNCHRONIZE | QUERY_LIMITED_INFORMATION | TERMINATE, without inheritance.
        IntPtr h = OpenProcess(0x00101001, false, pid);
        if (h == IntPtr.Zero) return null;
        long c, e, k, u;
        if (!GetProcessTimes(h, out c, out e, out k, out u)) { CloseHandle(h); return null; }
        return new PsmuxLifecycleAuditHandleV1 { handle = h, Pid = pid, CreatedFileTime = c };
    }
    public bool Stop() { return !IsAlive || TerminateProcess(handle, 233); }
    public bool WaitForExit(uint milliseconds) { return handle != IntPtr.Zero && WaitForSingleObject(handle, milliseconds) == 0; }
    public void Dispose() { if (handle != IntPtr.Zero) { CloseHandle(handle); handle = IntPtr.Zero; } }
}
'@
}

$runToken = (Get-Date -Format 'yyyyMMdd-HHmmss') + '-' + [Guid]::NewGuid().ToString('N').Substring(0,12)
$auditParent = [IO.Path]::GetFullPath((Join-Path $repoRoot 'target/audit'))
$evidenceDir = [IO.Path]::GetFullPath((Join-Path $auditParent ('lifecycle-' + $runToken)))
if (-not $evidenceDir.StartsWith($auditParent + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Audit path escaped target/audit.' }
if (Test-Path -LiteralPath $evidenceDir) { throw 'Refusing to reuse an audit directory.' }
New-Item -ItemType Directory -Path $evidenceDir | Out-Null
$dataDir = Join-Path $evidenceDir 'data'
New-Item -ItemType Directory -Path $dataDir | Out-Null
$namespace = 'audit_lifecycle_' + [Guid]::NewGuid().ToString('N').Substring(0,12)
$sessionName = 'audit-session'
$configPath = Join-Path $evidenceDir 'audit.conf'
$cmdExe = Join-Path $env:SystemRoot 'System32/cmd.exe'
if (-not (Test-Path -LiteralPath $cmdExe -PathType Leaf)) { throw 'cmd.exe unavailable; refusing to substitute a profile-loading shell.' }
[IO.File]::WriteAllText($configPath, "set -g warm off`nset -g default-shell `"$($cmdExe.Replace('\','/'))`"`nset -g base-index 0`nset -g pane-base-index 0`n", [Text.UTF8Encoding]::new($false))
$events = [Collections.Generic.List[object]]::new()
$owned = [Collections.Generic.List[object]]::new()
$server = $null
$before = [pscustomobject]@{ Registries=@(); Processes=@() }
$status = 'not-started'
$auditError = $null
$serverExitCode = $null
$counter = 0
$realRoots = @((Join-Path $env:USERPROFILE '.psmux'))
if ($env:HOME) { $realRoots += Join-Path $env:HOME '.psmux' }
if ($env:PSMUX_DATA_DIR) { $realRoots += [IO.Path]::GetFullPath($env:PSMUX_DATA_DIR) }
$realRoots = @($realRoots | ForEach-Object { [IO.Path]::GetFullPath($_) } | Sort-Object -Unique)

function Save-Json([string]$Name, $Value) {
    $Value | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $evidenceDir $Name) -Encoding utf8NoBOM
}

function Get-ObservedSnapshot([string[]]$Roots) {
    $errors = [Collections.Generic.List[string]]::new()
    $registries = @(foreach ($root in $Roots) {
        try {
            $entries = if (Test-Path -LiteralPath $root) { @(Get-ChildItem -LiteralPath $root -File) } else { @() }
            $records = @(foreach ($entry in $entries) {
                if ($entry.Extension -notin '.port','.pid','.sid','.key','.act','.spawnlock','.instance' -and -not $entry.Name.EndsWith('.registry.json')) { continue }
                try {
                    [pscustomobject]@{ Name=$entry.Name; Length=$entry.Length; ModifiedUtc=$entry.LastWriteTimeUtc; SHA256=(Get-FileHash -LiteralPath $entry.FullName).Hash }
                } catch { $errors.Add("Snapshot file ($($entry.FullName)): " + $_.Exception.Message) }
            })
            [pscustomobject]@{ Root=$root; Files=@($records | Sort-Object Name) }
        } catch { $errors.Add("Snapshot root ($root): " + $_.Exception.Message) }
    })
    try {
        $processes = @(Get-CimInstance Win32_Process -OperationTimeoutSec 10 | Where-Object Name -match '^(psmux|tmux|pmux)\.exe$' |
            Select-Object ProcessId,ParentProcessId,Name,ExecutablePath,CreationDate | Sort-Object ProcessId)
    } catch { $processes=@(); $errors.Add('Snapshot processes: ' + $_.Exception.Message) }
    [pscustomobject]@{ AtUtc=[DateTime]::UtcNow.ToString('o'); Registries=$registries; Processes=$processes; Errors=$errors.ToArray() }
}

function Start-OwnedMux([string[]]$MuxArgs, [string]$Label, [switch]$InheritNamespace) {
    # Re-check before every launch so replacing the file mid-audit cannot invoke
    # an unexpected or old binary under the isolated directory.
    if ((Get-FileHash -LiteralPath $PsmuxExe).Hash -ne $ExpectedSha256) { throw 'Executable changed during audit.' }
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $PsmuxExe
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.WorkingDirectory = $evidenceDir
    foreach ($name in @($start.Environment.Keys)) {
        if ($name -like 'PSMUX_*' -or $name -eq 'TMUX' -or $name -eq 'TMUX_PANE') { [void]$start.Environment.Remove($name) }
    }
    $start.Environment['PSMUX_DATA_DIR'] = $dataDir
    $start.Environment['PSMUX_NO_WARM'] = '1'
    $start.Environment['PSMUX_CONFIG_FILE'] = $configPath
    $routing = @('-L',$namespace)
    if ($InheritNamespace) { $routing = @(); $start.Environment['PSMUX_SOCKET_NAME'] = $namespace }
    foreach ($arg in $routing + @('-f',$configPath) + $MuxArgs) { $start.ArgumentList.Add($arg) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    [void]$process.Start()
    $record = [pscustomobject]@{ Label=$Label; Process=$process; Stdout=$process.StandardOutput.ReadToEndAsync(); Stderr=$process.StandardError.ReadToEndAsync(); Handle=$null }
    $record.Handle = [PsmuxLifecycleAuditHandleV1]::Open([uint32]$process.Id)
    if (-not $record.Handle -and -not $process.HasExited) {
        # This object came directly from Process.Start, so its retained OS
        # handle still names this exact child. Do not leave it behind when
        # duplicate ownership capture fails.
        $process.Kill(); [void]$process.WaitForExit(2000); $process.Dispose()
        throw 'Could not capture a live newly started child; stopped that exact child.'
    }
    if ($record.Handle -and $record.Handle.CreatedFileTime -ne $process.StartTime.ToUniversalTime().ToFileTimeUtc()) {
        $record.Handle.Dispose(); $record.Handle=$null; throw 'New process identity changed before ownership capture.'
    }
    return $record
}

function Read-ProcessText($Task) {
    if ($Task.Wait(1000)) { return [string]$Task.Result }
    return '[output capture incomplete after deadline]'
}

function Invoke-BoundedMux([string]$Label, [string[]]$MuxArgs, [int]$TimeoutMs=7000, [switch]$InheritNamespace) {
    $script:counter++
    $record = Start-OwnedMux $MuxArgs $Label -InheritNamespace:$InheritNamespace
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $timedOut = $false
    try {
        if (-not $record.Process.WaitForExit($TimeoutMs)) {
            $timedOut=$true
            if ($record.Handle) { [void]$record.Handle.Stop() }
            else { throw 'Client timeout without an owned termination handle.' }
            [void]$record.Process.WaitForExit(2000)
        }
        $result = [pscustomobject]@{
            Label=$Label; Arguments=$MuxArgs; Pid=$record.Process.Id; TimedOut=$timedOut
            ExitCode=$(if ($record.Process.HasExited) { $record.Process.ExitCode } else { $null })
            Stdout=(Read-ProcessText $record.Stdout); Stderr=(Read-ProcessText $record.Stderr); ElapsedMs=$timer.ElapsedMilliseconds
        }
        $events.Add($result)
        Save-Json ('client-{0:D2}-{1}.json' -f $script:counter,$Label) $result
        return $result
    } finally {
        if ($record.Handle) { $record.Handle.Dispose() }
        $record.Process.Dispose()
    }
}

function Require-Healthy($Result, [string]$Contains='') {
    if ($Result.TimedOut -or $Result.ExitCode -ne 0 -or ($Contains -and -not $Result.Stdout.Contains($Contains))) {
        throw "Healthy precondition failed: $($Result.Label); inspect client evidence."
    }
    if ($server.Process.HasExited) { throw "Server exited during healthy precondition: $($Result.Label)" }
}

function Test-Ready {
    $portPath = Join-Path $dataDir ($namespace + '__' + $sessionName + '.port')
    $keyPath = [IO.Path]::ChangeExtension($portPath,'.key')
    $tcp=$null
    try {
        $port=[int][IO.File]::ReadAllText($portPath).Trim()
        $key=[IO.File]::ReadAllText($keyPath).Trim()
        if (-not $key -or $key -match '[\r\n\x00]') { return $false }
        $tcp=[Net.Sockets.TcpClient]::new()
        if (-not $tcp.ConnectAsync('127.0.0.1',$port).Wait(500)) { return $false }
        $stream=$tcp.GetStream(); $stream.ReadTimeout=500; $stream.WriteTimeout=500
        $bytes=[Text.Encoding]::UTF8.GetBytes("AUTH $key`nsession-info`n")
        $stream.Write($bytes,0,$bytes.Length)
        $reader=[IO.StreamReader]::new($stream)
        if ($reader.ReadLine() -ne 'OK') { return $false }
        $line=$reader.ReadLine()
        return ($line -and $line.StartsWith($sessionName + ': '))
    } catch { return $false }
    finally { $key=$null; if ($tcp) { $tcp.Dispose() } }
}

function Capture-OwnedDescendants {
    # Once the server has exited, its numeric PID can be reused. Keep the
    # handles captured before the fault; never discover a new tree from that
    # stale parent ID during cleanup.
    if (-not $server -or -not $server.Handle -or -not $server.Handle.IsAlive) { return }
    $snapshot=@(Get-CimInstance Win32_Process -OperationTimeoutSec 10)
    $queue=[Collections.Generic.Queue[uint32]]::new(); $queue.Enqueue([uint32]$server.Process.Id)
    $visited=[Collections.Generic.HashSet[uint32]]::new()
    while ($queue.Count -gt 0) {
        $parent=$queue.Dequeue()
        if (-not $visited.Add($parent)) { continue }
        foreach ($child in @($snapshot | Where-Object ParentProcessId -eq $parent)) {
            $childPid=[uint32]$child.ProcessId
            if ($childPid -eq $PID -or $child.CreationDate.ToUniversalTime().ToFileTimeUtc() -lt $server.Handle.CreatedFileTime) { continue }
            if (@($owned | Where-Object Pid -eq $childPid).Count -gt 0) { $queue.Enqueue($childPid); continue }
            $handle=[PsmuxLifecycleAuditHandleV1]::Open($childPid)
            if (-not $handle) { continue }
            # CIM timestamps have microsecond precision, FILETIME has 100ns.
            $expected=$child.CreationDate.ToUniversalTime().ToFileTimeUtc()
            if ([Math]::Abs($handle.CreatedFileTime-$expected) -ge 10) { $handle.Dispose(); continue }
            $owned.Add([pscustomobject]@{ Pid=$childPid; ParentPid=$parent; Name=$child.Name; CreatedFileTime=$handle.CreatedFileTime; Handle=$handle })
            $queue.Enqueue($childPid)
        }
    }
    Save-Json 'owned-descendants.json' @($owned | Select-Object Pid,ParentPid,Name,CreatedFileTime)
}

try {
    Write-Output "Audit evidence: $evidenceDir"
    Save-Json 'invocation.json' ([ordered]@{ Executable=$PsmuxExe; SHA256=$actualHash; DataDir=$dataDir; Namespace=$namespace; Config=$configPath; UserProfile=$env:USERPROFILE; Home=$env:HOME; Command='Direct isolated server; healthy queries; respawn-pane without -k'; StartedUtc=[DateTime]::UtcNow.ToString('o') })
    $before=Get-ObservedSnapshot $realRoots
    Save-Json 'real-before.json' $before
    $server=Start-OwnedMux @('server','-s',$sessionName,'-x','80','-y','24') 'server'
    if (-not $server.Handle) { throw 'Could not retain the exact server process handle.' }
    Save-Json 'server-identity.json' ([ordered]@{ Pid=$server.Process.Id; CreatedFileTime=$server.Handle.CreatedFileTime; Executable=$PsmuxExe; SHA256=$actualHash })
    $readyTimer=[Diagnostics.Stopwatch]::StartNew()
    $ready=$false
    while ($readyTimer.ElapsedMilliseconds -lt 20000 -and -not $server.Process.HasExited) {
        if (Test-Ready) { $ready=$true; break }
        Start-Sleep -Milliseconds 100
    }
    if (-not $ready) { throw 'Direct server did not authenticate and report its session within 20 seconds.' }
    Capture-OwnedDescendants
    Require-Healthy (Invoke-BoundedMux 'list-sessions' @('list-sessions')) ($sessionName + ': ')
    Require-Healthy (Invoke-BoundedMux 'has-session' @('has-session','-t',$sessionName))
    Require-Healthy (Invoke-BoundedMux 'new-window' @('new-window','-d','-t',$sessionName,'-n','audit-second'))
    Require-Healthy (Invoke-BoundedMux 'list-windows' @('list-windows','-t',$sessionName,'-F','#{window_index}|#{window_name}|#{window_panes}')) '1|audit-second|1'
    Require-Healthy (Invoke-BoundedMux 'rename-window' @('rename-window','-t',($sessionName + ':1'),'audit-renamed'))
    Require-Healthy (Invoke-BoundedMux 'list-renamed-window' @('list-windows','-t',$sessionName,'-F','#{window_index}|#{window_name}|#{window_panes}')) '1|audit-renamed|1'
    Capture-OwnedDescendants
    Save-Json 'isolated-before-fault.json' (Get-ObservedSnapshot @($dataDir))
    $fault=Invoke-BoundedMux 'respawn-live-pane' @('respawn-pane','-t',($sessionName + ':0'))
    $serverDied=$server.Process.WaitForExit(5000)
    if ($serverDied) {
        $serverExitCode=$server.Process.ExitCode
        $stderr=Read-ProcessText $server.Stderr
        if ($stderr.Contains('pane still active')) { $status='regression-server-died'; throw 'Invalid respawn terminated the server.' }
        else { $status='unexpected-server-exit'; throw 'Server exited without expected pane-still-active evidence.' }
    } else {
        Require-Healthy (Invoke-BoundedMux 'survivor-has-session' @('has-session','-t',$sessionName))
        Require-Healthy (Invoke-BoundedMux 'survivor-list-windows' @('list-windows','-t',$sessionName,'-F','#{window_index}|#{window_name}|#{window_panes}')) '1|audit-renamed|1'
        if (-not $fault.TimedOut -and $fault.ExitCode -ne 0) { $status='fixed-rejection-survivor' }
        else { $status='unexpected-success-survivor'; throw 'Server survived, but the invalid command did not return a bounded nonzero rejection.' }
    }
    # A failed replacement must not destroy the original child, even with -k.
    $originalPanes = Invoke-BoundedMux 'original-pane-pids' @('list-panes','-t',($sessionName + ':0'),'-F','#{pane_id}|#{pane_pid}')
    Require-Healthy $originalPanes
    $badDirectory = Join-Path $evidenceDir 'directory-that-does-not-exist'
    foreach ($verb in @('respawn-pane','respawn-window')) {
        $rejected = Invoke-BoundedMux ($verb + '-live-rejected') @($verb,'-t',($sessionName + ':0'))
        if ($rejected.TimedOut -or $rejected.ExitCode -eq 0 -or -not $rejected.Stderr) { throw "$verb did not reject an active target" }
        $rejected = Invoke-BoundedMux ($verb + '-replacement-failed') @($verb,'-k','-t',($sessionName + ':0'),'-c',$badDirectory)
        if ($rejected.TimedOut -or $rejected.ExitCode -eq 0 -or -not $rejected.Stderr) { throw "$verb accepted an invalid replacement directory" }
        $survivors = Invoke-BoundedMux ($verb + '-original-still-live') @('list-panes','-t',($sessionName + ':0'),'-F','#{pane_id}|#{pane_pid}')
        Require-Healthy $survivors
        if ($survivors.Stdout -ne $originalPanes.Stdout) { throw "$verb changed the original pane after failed replacement" }
    }

    # A foreign live PID is deliberately not a canonical psmux image. A name
    # collision must still preserve that registration byte-for-byte.
    $foreignBase = $namespace + '__foreign'
    $selfIdentity = [PsmuxLifecycleAuditHandleV1]::Open([uint32]$PID)
    try {
        $foreign = [ordered]@{ pid=$PID; creation_time=$selfIdentity.CreatedFileTime; generation='synthetic-live-owner'; port=65531; key='synthetic-key'; sid=999999 }
    } finally { $selfIdentity.Dispose() }
    $records = [ordered]@{ 'port'='65531'; 'key'='synthetic-key'; 'sid'='999999'; 'pid'=([string]$foreign.pid + ':' + [string]$foreign.creation_time); 'registry.json'=($foreign | ConvertTo-Json -Compress) }
    foreach ($extension in $records.Keys) { [IO.File]::WriteAllText((Join-Path $dataDir ($foreignBase + '.' + $extension)), $records[$extension]) }
    $rejected = Invoke-BoundedMux 'rename-live-collision' @('rename-session','-t',$sessionName,'foreign')
    if ($rejected.TimedOut -or $rejected.ExitCode -eq 0) { throw 'Rename overwrote or hung on a live destination' }
    foreach ($extension in $records.Keys) {
        $path = Join-Path $dataDir ($foreignBase + '.' + $extension)
        if ([IO.File]::ReadAllText($path) -ne $records[$extension]) { throw 'Rename modified foreign registry bytes' }
        Remove-Item -LiteralPath $path
    }
    Require-Healthy (Invoke-BoundedMux 'after-collision-has' @('has-session','-t',$sessionName))
    Require-Healthy (Invoke-BoundedMux 'inherited-namespace-list' @('list-sessions') -InheritNamespace) ($sessionName + ': ')

    # The new encoding must be usable on the real command route, not only by
    # standalone filename helper tests.
    $newName = 'audit__renamed'
    Require-Healthy (Invoke-BoundedMux 'rename-delimited-name' @('rename-session','-t',$sessionName,$newName))
    $sessionName = $newName
    Require-Healthy (Invoke-BoundedMux 'renamed-has' @('has-session','-t',$sessionName))
    Require-Healthy (Invoke-BoundedMux 'renamed-list' @('list-sessions','-F','#{session_name}')) $sessionName
    Require-Healthy (Invoke-BoundedMux 'renamed-new-window' @('new-window','-d','-t',$sessionName,'-n','post-rename'))
    Require-Healthy (Invoke-BoundedMux 'renamed-list-windows' @('list-windows','-t',$sessionName,'-F','#{window_name}')) 'post-rename'
    Require-Healthy (Invoke-BoundedMux 'enable-warm-panes' @('set-option','-g','warm','on'))
    for ($i=0; $i -lt 4; $i++) {
        $created = Invoke-BoundedMux ('warm-window-' + $i) @('new-window','-d','-t',$sessionName,'-n',('warm-' + $i))
        Require-Healthy $created
        if ($created.ElapsedMs -gt 2500) { throw 'Warm replenishment blocked window creation' }
        Require-Healthy (Invoke-BoundedMux ('warm-probe-' + $i) @('has-session','-t',$sessionName))
    }
    Require-Healthy (Invoke-BoundedMux 'disable-warm-panes' @('set-option','-g','warm','off'))
    Capture-OwnedDescendants

    # A sink that never reads stdin must not hold the control loop hostage.
    $sinkCommand = 'pwsh -NoLogo -NoProfile -NonInteractive -Command "Start-Sleep -Seconds 20"'
    Require-Healthy (Invoke-BoundedMux 'start-nonreading-sink' @('pipe-pane','-O','-t',($sessionName + ':1'),$sinkCommand))
    Capture-OwnedDescendants
    $flood = 'for /L %i in (1,1,60000) do @echo psmux-audit-output-012345678901234567890123456789012345678901234567890123456789'
    Require-Healthy (Invoke-BoundedMux 'start-output-flood' @('send-keys','-t',($sessionName + ':1'),$flood,'Enter'))
    $rssBefore = $server.Process.WorkingSet64
    $peakRss = $rssBefore
    for ($i=0; $i -lt 15; $i++) {
        $probe = Invoke-BoundedMux ('load-has-' + $i) @('has-session','-t',$sessionName)
        Require-Healthy $probe
        if ($probe.ElapsedMs -gt 2500) { throw 'Control request exceeded 2.5 seconds during output load' }
        $server.Process.Refresh()
        $peakRss = [Math]::Max($peakRss,$server.Process.WorkingSet64)
        Start-Sleep -Milliseconds 100
    }
    Require-Healthy (Invoke-BoundedMux 'cancel-nonreading-sink' @('pipe-pane','-t',($sessionName + ':1')))
    Require-Healthy (Invoke-BoundedMux 'stop-output-flood' @('send-keys','-t',($sessionName + ':1'),'C-c'))
    $floodCapture = Invoke-BoundedMux 'verify-output-flood' @('capture-pane','-p','-t',($sessionName + ':1'))
    Require-Healthy $floodCapture 'psmux-audit-output-'
    if ([regex]::Matches($floodCapture.Stdout, 'psmux-audit-output-').Count -lt 3) { throw 'Output flood did not produce the expected repeated pane output' }
    Require-Healthy (Invoke-BoundedMux 'post-load-list' @('list-sessions')) $sessionName
    Save-Json 'load-memory.json' ([ordered]@{ BeforeBytes=$rssBefore; PeakBytes=$peakRss; IncreaseBytes=($peakRss-$rssBefore); Samples=15 })
    if ($peakRss -gt $rssBefore + 128MB) { throw 'Resident memory grew more than 128 MiB during the bounded load probe' }
    Capture-OwnedDescendants
    $status = 'fixed-rejections-transactions-namespace-and-load-survivor'
    Save-Json 'isolated-after-fault.json' (Get-ObservedSnapshot @($dataDir))
} catch {
    $auditError=$_.Exception.Message
    if ($status -eq 'not-started') { $status='precondition-failed' }
} finally {
    $cleanup=[Collections.Generic.List[object]]::new()
    if ($server) {
        try { Capture-OwnedDescendants } catch { $cleanup.Add([pscustomobject]@{ Stage='capture'; Error=$_.Exception.Message }) }
        if ($server.Handle) {
            $cleanup.Add([pscustomobject]@{ Pid=$server.Handle.Pid; CreatedFileTime=$server.Handle.CreatedFileTime; Kind='server'; WasAlive=$server.Handle.IsAlive; StopSucceeded=$server.Handle.Stop() })
        }
        foreach ($entry in $owned) {
            $cleanup.Add([pscustomobject]@{ Pid=$entry.Pid; CreatedFileTime=$entry.CreatedFileTime; Kind=$entry.Name; WasAlive=$entry.Handle.IsAlive; StopSucceeded=$entry.Handle.Stop() })
        }
        foreach ($entry in $owned) {
            if (-not $entry.Handle.WaitForExit(2000)) {
                $cleanup.Add([pscustomobject]@{ Pid=$entry.Pid; Kind=$entry.Name; Error='Owned process remained after cleanup deadline' })
                $auditError='An owned process did not stop within the cleanup deadline; inspect cleanup.json.'
            }
        }
        if (-not $server.Process.WaitForExit(3000)) {
            $cleanup.Add([pscustomobject]@{ Pid=$server.Process.Id; Kind='server'; Error='Owned server remained after cleanup deadline' })
            $auditError='Owned server did not stop within the cleanup deadline.'
        }
        [IO.File]::WriteAllText((Join-Path $evidenceDir 'server.stdout.txt'),(Read-ProcessText $server.Stdout))
        [IO.File]::WriteAllText((Join-Path $evidenceDir 'server.stderr.txt'),(Read-ProcessText $server.Stderr))
        foreach ($entry in $owned) { $entry.Handle.Dispose() }
        if ($server.Handle) { $server.Handle.Dispose() }
        $server.Process.Dispose()
    }
    Save-Json 'cleanup.json' $cleanup.ToArray()
    $after=Get-ObservedSnapshot $realRoots
    Save-Json 'real-after.json' $after
    $realChanges=@(foreach ($oldRoot in @($before.Registries)) {
        $newRoot=@($after.Registries | Where-Object Root -eq $oldRoot.Root) | Select-Object -First 1
        $oldRecords=@($oldRoot.Files | ForEach-Object { $_.Name + '|' + $_.SHA256 })
        $newRecords=@($newRoot.Files | ForEach-Object { $_.Name + '|' + $_.SHA256 })
        foreach ($oldRecord in $oldRecords) { if ($newRecords -notcontains $oldRecord) { [pscustomobject]@{ Root=$oldRoot.Root; Record=$oldRecord; Side='<=' } } }
        foreach ($newRecord in $newRecords) { if ($oldRecords -notcontains $newRecord) { [pscustomobject]@{ Root=$oldRoot.Root; Record=$newRecord; Side='=>' } } }
    })
    Save-Json 'real-registry-changes.json' $realChanges
    $lostProcesses=@(foreach ($prior in @($before.Processes)) {
        if (@($after.Processes | Where-Object { $_.ProcessId -eq $prior.ProcessId -and $_.CreationDate -eq $prior.CreationDate }).Count -eq 0) { $prior }
    })
    if ($realChanges.Count -gt 0 -or $lostProcesses.Count -gt 0) { $auditError='Real registry/process baseline changed; treat this run as failed and inspect before/after evidence.' }
    if (@($cleanup | Where-Object { $_.Error -or ($_.PSObject.Properties.Name -contains 'StopSucceeded' -and -not $_.StopSucceeded) }).Count -gt 0) { $auditError='Owned-process cleanup reported a failure; inspect cleanup.json.' }
    $summary=[ordered]@{ Status=$status; Error=$auditError; EvidenceDirectory=$evidenceDir; ServerFaultExitCode=$serverExitCode; RealRegistryChangeCount=$realChanges.Count; BaselineMuxProcessesAbsentAfter=$lostProcesses; Commands=$events.ToArray(); Cleanup=$cleanup.ToArray(); CompletedUtc=[DateTime]::UtcNow.ToString('o'); Note='Real-state changes may be concurrent user activity; inspect snapshots. No installed executable or global kill command was invoked.' }
    Save-Json 'summary.json' $summary
    Write-Output ($summary | ConvertTo-Json -Depth 7)
}
if ($auditError) { throw $auditError }
exit 0
