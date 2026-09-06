#requires -Version 7.2
<#
.SYNOPSIS
Reproduce discovery defects against fake peers without creating psmux sessions.
.DESCRIPTION
Use ONLY a freshly built, reviewed upstream binary with the #510 reaper
attribution fix. Never pass the older installed executable. The mandatory hash
pins the reviewed build; this script does not establish its source provenance.

All psmux invocations are bounded list-sessions commands against a new
PSMUX_DATA_DIR below target/audit. Fake loopback peers run in owned PowerShell
workers. No psmux server, kill command, wildcard cleanup, or real registry write
is used. Scratch evidence is retained. All cases require the fixed behavior; exit 1 means a regression, harness failure,
or real-state change requiring investigation.
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory)][ValidateNotNullOrEmpty()][string]$PsmuxExe,
    [Parameter(Mandatory)][ValidatePattern('^[0-9a-fA-F]{64}$')][string]$ExpectedSha256,
    [ValidateRange(2, 30)][int]$CommandTimeoutSeconds = 8
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$repoRoot = Split-Path -Parent $PSScriptRoot
$allowedRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot 'target/audit'))
$runRoot = [IO.Path]::GetFullPath((Join-Path $allowedRoot ('discovery-' + [Guid]::NewGuid().ToString('N'))))
if (-not $runRoot.StartsWith($allowedRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Scratch directory escaped target/audit.'
}
$exePath = (Resolve-Path -LiteralPath $PsmuxExe).Path
$actualHash = (Get-FileHash -LiteralPath $exePath -Algorithm SHA256).Hash
if ($actualHash -ne $ExpectedSha256) { throw 'Executable SHA-256 differs from the reviewed build.' }
$pwshPath = (Get-Process -Id $PID).Path
if ([IO.Path]::GetFileNameWithoutExtension($pwshPath) -ne 'pwsh') {
    throw 'Run this audit from pwsh.exe, not an embedded PowerShell host.'
}

New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
$namespace = 'audit' + [Guid]::NewGuid().ToString('N')
$realHome = if ($env:USERPROFILE) { $env:USERPROFILE } else { $env:HOME }
$realRegistry = Join-Path $realHome '.psmux'
$savedEnvironment = @{}
$managedEnvironment = @('TMUX', 'TMUX_PANE', 'PSMUX_DATA_DIR', 'PSMUX_NO_WARM')
$managedEnvironment += @(Get-ChildItem Env: | Where-Object Name -Like 'PSMUX_*' | ForEach-Object Name)
$managedEnvironment = @($managedEnvironment | Sort-Object -Unique)
foreach ($name in $managedEnvironment) {
    $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
}
$children = [Collections.Generic.List[object]]::new()
$results = [Collections.Generic.List[object]]::new()
$failures = [Collections.Generic.List[string]]::new()

function Get-RealSnapshot {
    $files = [Collections.Generic.List[object]]::new()
    if (Test-Path -LiteralPath $realRegistry -PathType Container) {
        foreach ($entry in Get-ChildItem -LiteralPath $realRegistry -File -Force) {
            if ($entry.Extension -notin @('.port', '.key', '.pid', '.sid', '.act')) { continue }
            try {
                $digest = (Get-FileHash -LiteralPath $entry.FullName -Algorithm SHA256).Hash
                $files.Add([pscustomobject]@{
                    Name = $entry.Name; Length = $entry.Length
                    LastWriteUtc = $entry.LastWriteTimeUtc.ToString('o'); Sha256 = $digest
                })
            } catch {
                $files.Add([pscustomobject]@{ Name = $entry.Name; ReadError = $_.Exception.Message })
            }
        }
    }
    $processes = @(Get-Process | Where-Object ProcessName -In @('psmux', 'tmux', 'pmux') |
        ForEach-Object {
            $started = try { $_.StartTime.ToUniversalTime().ToString('o') } catch { $null }
            [pscustomobject]@{ Id = $_.Id; Name = $_.ProcessName; StartTimeUtc = $started }
        } | Sort-Object Id)
    [pscustomobject]@{
        CapturedUtc = [DateTime]::UtcNow.ToString('o'); Registry = $realRegistry
        Files = @($files | Sort-Object Name); Processes = $processes
    }
}

function Start-OwnedProcess([string]$File, [string[]]$Arguments) {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $File
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.WorkingDirectory = $runRoot
    foreach ($argValue in $Arguments) { $start.ArgumentList.Add($argValue) }
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    if (-not $process.Start()) { throw "Could not start owned child: $File" }
    $owned = [pscustomobject]@{
        Process = $process
        Stdout = $process.StandardOutput.ReadToEndAsync()
        Stderr = $process.StandardError.ReadToEndAsync()
    }
    $children.Add($owned)
    return $owned
}

function Stop-OwnedProcess($Owned) {
    if (-not $Owned.Process.HasExited) {
        # Only the exact Process object started by this script is eligible.
        $Owned.Process.Kill()
        if (-not $Owned.Process.WaitForExit(3000)) { throw 'Owned child did not exit after termination.' }
    }
}

function Invoke-Listing([string[]]$Arguments) {
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $owned = Start-OwnedProcess $exePath $Arguments
    if (-not $owned.Process.WaitForExit($CommandTimeoutSeconds * 1000)) {
        Stop-OwnedProcess $owned
        throw "Listing exceeded its $CommandTimeoutSeconds second deadline."
    }
    $timer.Stop()
    [pscustomobject]@{
        ExitCode = $owned.Process.ExitCode
        Stdout = $owned.Stdout.GetAwaiter().GetResult()
        Stderr = $owned.Stderr.GetAwaiter().GetResult()
        ElapsedMs = $timer.ElapsedMilliseconds
    }
}

$workerPath = Join-Path $runRoot 'fake-peer.ps1'
$workerSource = @'
#requires -Version 7.2
param([string]$Mode, [string]$ReadyPath, [string]$TranscriptPath)
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$events = [Collections.Generic.List[object]]::new()
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$client = $null
try {
    $listener.Start()
    [IO.File]::WriteAllText($ReadyPath, $listener.LocalEndpoint.Port.ToString())
    $deadline = [DateTime]::UtcNow.AddSeconds(15)
    for ($index = 1; $index -le 1; $index++) {
        while (-not $listener.Pending()) {
            if ([DateTime]::UtcNow -gt $deadline) { throw 'Fake peer accept deadline exceeded.' }
            Start-Sleep -Milliseconds 10
        }
        $client = $listener.AcceptTcpClient()
        $client.NoDelay = $true
        $stream = $client.GetStream()
        $stream.ReadTimeout = 2000
        $stream.WriteTimeout = 2000
        $reader = [IO.StreamReader]::new($stream)
        $writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false))
        $writer.NewLine = "`n"
        $writer.AutoFlush = $true
        $auth = $reader.ReadLine()
        if ($null -eq $auth -or -not $auth.StartsWith('AUTH ')) { throw 'Expected AUTH from audit client.' }
        $command = $reader.ReadLine()
        if ($command -ne 'session-info' -and -not $command.StartsWith('list-sessions -F ')) {
            throw 'Unexpected command from listing client.'
        }
        $events.Add([pscustomobject]@{ Connection = $index; Stage = 'listing'; Command = $command; Mode = $Mode })
        if ($Mode -eq 'InvalidAuth') {
            $writer.WriteLine('ERROR: Invalid session key')
        } else {
            $writer.WriteLine('OK')
            if ($Mode -eq 'SlowPayload') { Start-Sleep -Milliseconds 3000 }
            try { $writer.WriteLine('audit-peer: 1 windows') } catch {
                if ($Mode -ne 'SlowPayload') { throw }
                $events.Add([pscustomobject]@{ Stage = 'late-write'; ClientAlreadyClosed = $true })
            }
        }
        $client.Dispose()
        $client = $null
    }
} catch {
    $events.Add([pscustomobject]@{ Stage = 'worker-error'; Message = $_.Exception.Message })
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
} finally {
    if ($null -ne $client) { $client.Dispose() }
    $listener.Stop()
    $events | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath $TranscriptPath -Encoding utf8
}
'@
[IO.File]::WriteAllText($workerPath, $workerSource, [Text.UTF8Encoding]::new($false))

function Invoke-PeerCase([string]$Case, [string]$Mode, [bool]$Formatted) {
    $dataDir = Join-Path $runRoot $Case
    New-Item -ItemType Directory -Path $dataDir | Out-Null
    [Environment]::SetEnvironmentVariable('PSMUX_DATA_DIR', $dataDir, 'Process')
    $readyPath = Join-Path $dataDir 'listener.ready'
    $transcriptPath = Join-Path $dataDir 'peer.json'
    $worker = Start-OwnedProcess $pwshPath @(
        '-NoLogo', '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', $workerPath,
        '-Mode', $Mode, '-ReadyPath', $readyPath, '-TranscriptPath', $transcriptPath
    )
    try {
        $readyDeadline = [DateTime]::UtcNow.AddSeconds(5)
        $parsedPort = 0
        while ($parsedPort -eq 0) {
            if ($worker.Process.HasExited) { throw 'Fake peer exited before readiness.' }
            if ([DateTime]::UtcNow -gt $readyDeadline) { throw 'Fake peer readiness deadline exceeded.' }
            if (Test-Path -LiteralPath $readyPath) {
                $portText = [IO.File]::ReadAllText($readyPath)
                $candidatePort = 0
                if ([int]::TryParse($portText, [ref]$candidatePort) -and $candidatePort -ge 1 -and $candidatePort -le 65535) {
                    $parsedPort = $candidatePort
                    break
                }
            }
            Start-Sleep -Milliseconds 10
        }
        $base = $namespace + '__peer'
        # No PID or ownership marker: this is a synthetic registry, not a server.
        [IO.File]::WriteAllText((Join-Path $dataDir ($base + '.key')), 'audit-synthetic-key')
        [IO.File]::WriteAllText((Join-Path $dataDir ($base + '.port')), $parsedPort.ToString())
        $cliArguments = @('-L', $namespace, 'list-sessions')
        if ($Formatted) { $cliArguments += @('-F', '#{session_name}') }
        $observed = Invoke-Listing $cliArguments
        $observed | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath (Join-Path $dataDir 'observed.json') -Encoding utf8
        if (-not $worker.Process.WaitForExit(4000)) { throw 'Fake peer did not finish its bounded protocol.' }
        if ($worker.Process.ExitCode -ne 0) { throw ('Fake peer failed: ' + $worker.Stderr.GetAwaiter().GetResult()) }
        $trace = @(Get-Content -LiteralPath $transcriptPath -Raw | ConvertFrom-Json)
        if (@($trace | Where-Object Stage -EQ 'listing').Count -ne 1) { throw 'Listing was not observed by the fake peer.' }
        $registryRetained = (Test-Path -LiteralPath (Join-Path $dataDir ($base + '.port'))) -and
            (Test-Path -LiteralPath (Join-Path $dataDir ($base + '.key')))
        if (-not $registryRetained) { throw 'The synthetic live registry was unexpectedly removed.' }
        $classification = 'unexpected-result'
        switch ($Mode) {
            'Healthy' {
                if ($observed.ExitCode -eq 0 -and $observed.Stdout.Trim() -eq 'audit-peer: 1 windows' -and
                    [string]::IsNullOrWhiteSpace($observed.Stderr)) { $classification = 'correct-behavior' }
            }
            'InvalidAuth' {
                if ($observed.ExitCode -eq 0 -and $observed.Stdout.Contains('ERROR: Invalid session key')) {
                    $classification = 'known-defect-observed'
                } elseif ($observed.ExitCode -ne 0 -and [string]::IsNullOrWhiteSpace($observed.Stdout)) {
                    $classification = 'defect-not-observed-error-reported'
                }
            }
            'SlowPayload' {
                if ($observed.ExitCode -eq 0 -and $observed.Stdout.Length -gt 0 -and
                    [string]::IsNullOrWhiteSpace($observed.Stdout)) {
                    $classification = 'known-defect-observed'
                } elseif ($observed.ExitCode -ne 0 -and [string]::IsNullOrWhiteSpace($observed.Stdout)) {
                    $classification = 'defect-not-observed-timeout-reported'
                } elseif ($observed.ExitCode -eq 0 -and $observed.Stdout.Trim() -eq 'audit-peer: 1 windows') {
                    $classification = 'defect-not-observed-response-awaited'
                }
            }
        }
        $results.Add([pscustomobject]@{
            Case = $Case; Classification = $classification; Observed = $observed
            RegistryRetained = $registryRetained; PeerTranscript = $transcriptPath
        })
        if ($classification -in @('unexpected-result','known-defect-observed')) { throw "Unexpected protocol result in $Case; inspect results.json." }
    } finally {
        Stop-OwnedProcess $worker
    }
}

$before = Get-RealSnapshot
$before | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $runRoot 'real-before.json') -Encoding utf8
try {
    foreach ($name in $managedEnvironment) { [Environment]::SetEnvironmentVariable($name, $null, 'Process') }
    [Environment]::SetEnvironmentVariable('PSMUX_NO_WARM', '1', 'Process')
    $emptyDir = Join-Path $runRoot 'empty'
    New-Item -ItemType Directory -Path $emptyDir | Out-Null
    [Environment]::SetEnvironmentVariable('PSMUX_DATA_DIR', $emptyDir, 'Process')
    $empty = Invoke-Listing @('-L', $namespace, 'list-sessions')
    $emptyOk = $empty.ExitCode -eq 1 -and [string]::IsNullOrWhiteSpace($empty.Stdout) -and
        $empty.Stderr.Contains('no server running')
    $results.Add([pscustomobject]@{
        Case = 'empty-namespace'; Classification = $(if ($emptyOk) { 'correct-behavior' } else { 'unexpected-result' })
        Observed = $empty
    })
    if (-not $emptyOk) { throw 'Empty namespace did not produce the reviewed upstream error behavior.' }
    Invoke-PeerCase 'healthy-control' 'Healthy' $false
    Invoke-PeerCase 'invalid-auth' 'InvalidAuth' $false
    Invoke-PeerCase 'formatted-timeout' 'SlowPayload' $true
} catch {
    $failures.Add($_.Exception.Message)
} finally {
    foreach ($child in $children) {
        try { Stop-OwnedProcess $child } catch { $failures.Add($_.Exception.Message) }
    }
    foreach ($name in $managedEnvironment) {
        [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], 'Process')
    }
    $after = Get-RealSnapshot
    $after | ConvertTo-Json -Depth 6 | Set-Content -LiteralPath (Join-Path $runRoot 'real-after.json') -Encoding utf8
    $afterIdentities = @($after.Processes | ForEach-Object { "$($_.Id)/$($_.StartTimeUtc)" })
    $lostProcesses = @($before.Processes | Where-Object { "$($_.Id)/$($_.StartTimeUtc)" -notin $afterIdentities })
    $changedRegistry = [Collections.Generic.List[string]]::new()
    $afterFiles = @{}
    foreach ($file in $after.Files) { $afterFiles[$file.Name] = $file }
    foreach ($file in $before.Files) {
        # .act records typing and may change during normal concurrent use.
        if ([IO.Path]::GetExtension($file.Name) -eq '.act') { continue }
        if (-not $afterFiles.ContainsKey($file.Name)) { $changedRegistry.Add($file.Name); continue }
        $other = $afterFiles[$file.Name]
        if ($file.PSObject.Properties.Name -contains 'Sha256' -and $other.PSObject.Properties.Name -contains 'Sha256') {
            if ($file.Sha256 -ne $other.Sha256) { $changedRegistry.Add($file.Name) }
        } else { $changedRegistry.Add($file.Name) }
    }
    if ($lostProcesses.Count -gt 0 -or $changedRegistry.Count -gt 0) {
        $failures.Add('Real process/registry snapshots changed. This may be concurrent activity; inspect before/after evidence before attributing cause.')
    }
    $summary = [pscustomobject]@{
        Binary = $exePath; Sha256 = $actualHash; Namespace = $namespace; EvidenceDirectory = $runRoot
        NoPsmuxServersSpawned = $true; Cases = $results.ToArray()
        LostExistingProcesses = $lostProcesses; ChangedExistingRegistryFiles = $changedRegistry.ToArray()
        Failures = $failures.ToArray(); HarnessPassed = ($failures.Count -eq 0)
    }
    $summary | ConvertTo-Json -Depth 8 | Set-Content -LiteralPath (Join-Path $runRoot 'results.json') -Encoding utf8
    foreach ($result in $results) { Write-Host ("{0}: {1}" -f $result.Case, $result.Classification) }
    Write-Host "Evidence: $runRoot"
    foreach ($failure in $failures) { Write-Warning $failure }
    foreach ($child in $children) { $child.Process.Dispose() }
}
if ($failures.Count -gt 0) { exit 1 }
exit 0
