# Regression coverage for control-mode liveness under the Windows lifecycle
# failures that matter here:
#   1. runtime warm-pane replenishment is deliberately delayed, while unrelated
#      display-message and capture-pane commands must stay responsive;
#   2. input EOF drains already-completed response blocks before socket close;
#   3. a control client that stops reading is disconnected after the bounded
#      socket write timeout, without stalling a second control client.
#
# The warm-pane delay hook is compiled only into debug builds. A release build
# therefore skips this test rather than producing a false green.

$ErrorActionPreference = 'Stop'
. "$PSScriptRoot\psmux_test_helpers.ps1"

$ctx = New-PsmuxTestEnv -Tag 'ctrl_live'
$PSMUX = $ctx.PsmuxExe
if ($PSMUX -notmatch '\\debug\\') {
    Write-Host "[SKIP] control liveness fault injection requires a debug build: $PSMUX" -ForegroundColor Yellow
    Remove-PsmuxTestEnv -Ctx $ctx
    exit 0
}

$namespace = Register-PsmuxNamespace -Ctx $ctx -Namespace 'ctrl_live'
$session = 'ctrl_live'
$base = "${namespace}__${session}"
$delayMs = 8000
$commandLimitMs = 2500
$delayStartedFile = Join-Path $ctx.Home 'warm-delay-started.txt'
$pressureStartedFile = Join-Path $ctx.Home 'pressure-started.txt'
$pass = 0
$fail = 0

function Write-Result([string]$Name, [bool]$Ok, [string]$Detail = '') {
    if ($Ok) {
        Write-Host "  [PASS] $Name" -ForegroundColor Green
        $script:pass++
    } else {
        Write-Host "  [FAIL] $Name$(if ($Detail) { ": $Detail" })" -ForegroundColor Red
        $script:fail++
    }
}

function Read-ExactBytes {
    param(
        [Parameter(Mandatory)]$Stream,
        [Parameter(Mandatory)][int]$Count
    )
    $buffer = New-Object byte[] $Count
    $offset = 0
    while ($offset -lt $Count) {
        $read = $Stream.Read($buffer, $offset, $Count - $offset)
        if ($read -le 0) { throw "control socket closed after $offset/$Count greeting bytes" }
        $offset += $read
    }
    return ,$buffer
}

function Drain-ControlClient {
    param(
        [Parameter(Mandatory)]$Client,
        [int]$TimeoutMs = 250
    )
    $Client.Stream.ReadTimeout = $TimeoutMs
    try {
        while ($null -ne $Client.Reader.ReadLine()) {}
    } catch [System.IO.IOException] {}
}

function Open-ControlClient {
    param(
        [Parameter(Mandatory)]$Context,
        [Parameter(Mandatory)][string]$SessionBase,
        [switch]$SlowReceive
    )
    $port = [int](Get-Content (Join-Path $Context.PsmuxDir "$SessionBase.port") -Raw).Trim()
    $key = (Get-Content (Join-Path $Context.PsmuxDir "$SessionBase.key") -Raw).Trim()
    $tcp = [System.Net.Sockets.TcpClient]::new()
    if ($SlowReceive) { $tcp.ReceiveBufferSize = 1024 }
    $tcp.NoDelay = $true
    $tcp.Connect('127.0.0.1', $port)
    $stream = $tcp.GetStream()
    $stream.ReadTimeout = 3000
    $stream.WriteTimeout = 3000
    $encoding = [System.Text.UTF8Encoding]::new($false)
    $reader = [System.IO.StreamReader]::new($stream, $encoding, $false, 4096, $true)
    $writer = [System.IO.StreamWriter]::new($stream, $encoding, 4096, $true)
    $writer.NewLine = "`n"
    $writer.AutoFlush = $true
    $writer.WriteLine("AUTH $key")
    $auth = $reader.ReadLine()
    if ($auth -notmatch '^OK') { throw "control AUTH failed: $auth" }
    $writer.WriteLine('CONTROL_NOECHO')
    $reader.Dispose()
    $greeting = Read-ExactBytes -Stream $stream -Count 7
    $expected = [byte[]](0x1b, 0x50, 0x31, 0x30, 0x30, 0x30, 0x70)
    if (Compare-Object $greeting $expected) {
        throw 'control DCS greeting did not match ESC P 1000 p'
    }
    $reader = [System.IO.StreamReader]::new($stream, $encoding, $false, 4096, $true)
    $client = [pscustomobject]@{
        Tcp = $tcp
        Stream = $stream
        Reader = $reader
        Writer = $writer
    }
    Drain-ControlClient -Client $client
    return $client
}

function Close-ControlClient {
    param($Client)
    if ($null -eq $Client) { return }
    try { $Client.Tcp.Close() } catch {}
}

function Invoke-ControlCommand {
    param(
        [Parameter(Mandatory)]$Client,
        [Parameter(Mandatory)][string]$Command,
        [int]$TimeoutMs = 5000
    )
    $Client.Stream.ReadTimeout = $TimeoutMs
    $lines = [System.Collections.Generic.List[string]]::new()
    $number = $null
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    $Client.Writer.WriteLine($Command)
    try {
        while ($true) {
            $line = $Client.Reader.ReadLine()
            if ($null -eq $line) { throw 'control socket closed before command footer' }
            $lines.Add($line)
            if ($line -match '^%begin \d+ (\d+) 1$') {
                $number = $Matches[1]
                continue
            }
            if ($null -ne $number -and $line -match "^%(end|error) \d+ $number 1$") {
                break
            }
        }
    } finally {
        $stopwatch.Stop()
    }
    $text = $lines -join "`n"
    return [pscustomobject]@{
        Text = $text
        ElapsedMs = [int]$stopwatch.ElapsedMilliseconds
        BeginCount = ([regex]::Matches($text, '(?m)^%begin ')).Count
        EndCount = ([regex]::Matches($text, '(?m)^%end ')).Count
        ErrorCount = ([regex]::Matches($text, '(?m)^%error ')).Count
    }
}

function Invoke-ControlBatchAndHalfClose {
    param(
        [Parameter(Mandatory)]$Client,
        [Parameter(Mandatory)][string[]]$Commands,
        [int]$TimeoutMs = 5000
    )
    $Client.Stream.ReadTimeout = $TimeoutMs
    foreach ($command in $Commands) {
        $Client.Writer.WriteLine($command)
    }
    $Client.Writer.Flush()
    $Client.Tcp.Client.Shutdown([System.Net.Sockets.SocketShutdown]::Send)

    $lines = [System.Collections.Generic.List[string]]::new()
    $stopwatch = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        while ($true) {
            $line = $Client.Reader.ReadLine()
            if ($null -eq $line) { break }
            $lines.Add($line)
        }
    } finally {
        $stopwatch.Stop()
    }
    $text = $lines -join "`n"
    return [pscustomobject]@{
        Text = $text
        ElapsedMs = [int]$stopwatch.ElapsedMilliseconds
        BeginCount = ([regex]::Matches($text, '(?m)^%begin ')).Count
        EndCount = ([regex]::Matches($text, '(?m)^%end ')).Count
        ErrorCount = ([regex]::Matches($text, '(?m)^%error ')).Count
    }
}

function Wait-ForFile {
    param(
        [Parameter(Mandatory)][string]$Path,
        [int]$TimeoutMs = 5000
    )
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (Test-Path $Path -PathType Leaf) { return $true }
        Start-Sleep -Milliseconds 25
    }
    return $false
}

function Wait-ControlClientClosed {
    param(
        [Parameter(Mandatory)]$Client,
        [int]$TimeoutMs = 5000
    )
    $Client.Stream.ReadTimeout = 250
    $deadline = [DateTime]::UtcNow.AddMilliseconds($TimeoutMs)
    $characters = 0L
    while ([DateTime]::UtcNow -lt $deadline) {
        try {
            $line = $Client.Reader.ReadLine()
            if ($null -eq $line) {
                return [pscustomobject]@{ Closed = $true; Characters = $characters }
            }
            $characters += $line.Length
        } catch [System.IO.IOException] {
            $socket = $_.Exception.InnerException
            if ($socket -is [System.Net.Sockets.SocketException] -and
                $socket.SocketErrorCode -eq [System.Net.Sockets.SocketError]::TimedOut) {
                continue
            }
            return [pscustomobject]@{ Closed = $true; Characters = $characters }
        }
    }
    return [pscustomobject]@{ Closed = $false; Characters = $characters }
}

$control = $null
$drainControl = $null
$slowControl = $null
$healthyControl = $null
$savedConfig = $env:PSMUX_CONFIG_FILE
$savedDelay = $env:PSMUX_TEST_WARM_PANE_DELAY_MS
$savedMarker = $env:PSMUX_TEST_WARM_PANE_DELAY_STARTED_FILE
try {
    $env:PSMUX_CONFIG_FILE = 'NUL'
    $env:PSMUX_TEST_WARM_PANE_DELAY_MS = "$delayMs"
    $env:PSMUX_TEST_WARM_PANE_DELAY_STARTED_FILE = $delayStartedFile

    & $PSMUX -L $namespace new-session -d -s $session -x 100 -y 30 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "new-session failed with exit code $LASTEXITCODE" }

    Write-Host "`n=== Runtime warm-pane spawn does not block control commands ===" -ForegroundColor Cyan
    $control = Open-ControlClient -Context $ctx -SessionBase $base
    $trigger = Invoke-ControlCommand -Client $control `
        -Command 'new-window -d -P -F "#{pane_id}" -n worker-trigger'
    Write-Result 'new-window completed before the injected worker delay' `
        ($trigger.EndCount -eq 1 -and $trigger.ErrorCount -eq 0) $trigger.Text

    $injectionStarted = Wait-ForFile -Path $delayStartedFile -TimeoutMs 2500
    Write-Result 'runtime warm-pane delay hook started' $injectionStarted `
        "marker did not appear at $delayStartedFile"

    $display = Invoke-ControlCommand -Client $control `
        -Command 'display-message -p "control-alive"' `
        -TimeoutMs $commandLimitMs
    Write-Result 'display-message stays responsive during warm ConPTY creation' `
        ($display.Text -match '(?m)^control-alive$' -and
            $display.ElapsedMs -lt $commandLimitMs -and
            $display.BeginCount -eq 1 -and
            $display.EndCount -eq 1 -and
            $display.ErrorCount -eq 0) `
        "elapsed=$($display.ElapsedMs)ms output=$($display.Text)"

    $capture = Invoke-ControlCommand -Client $control `
        -Command 'capture-pane -p -e -J -N -t :0' `
        -TimeoutMs $commandLimitMs
    Write-Result 'capture-pane stays responsive during warm ConPTY creation' `
        ($capture.ElapsedMs -lt $commandLimitMs -and
            $capture.BeginCount -eq 1 -and
            $capture.EndCount -eq 1 -and
            $capture.ErrorCount -eq 0) `
        "elapsed=$($capture.ElapsedMs)ms output=$($capture.Text)"

    Write-Host "`n=== One failed command does not poison the control connection ===" -ForegroundColor Cyan
    $bad = Invoke-ControlCommand -Client $control -Command 'definitely-not-a-real-command'
    Write-Result 'unknown command has exactly one %begin/%error pair' `
        ($bad.BeginCount -eq 1 -and $bad.EndCount -eq 0 -and $bad.ErrorCount -eq 1) `
        $bad.Text
    $afterError = Invoke-ControlCommand -Client $control -Command 'display-message -p "after-error"'
    Write-Result 'next command succeeds on the same socket after %error' `
        ($afterError.Text -match '(?m)^after-error$' -and
            $afterError.BeginCount -eq 1 -and
            $afterError.EndCount -eq 1 -and
            $afterError.ErrorCount -eq 0) `
        $afterError.Text
    Close-ControlClient $control
    $control = $null

    Write-Host "`n=== Input EOF drains completed command blocks ===" -ForegroundColor Cyan
    $drainControl = Open-ControlClient -Context $ctx -SessionBase $base
    $drained = Invoke-ControlBatchAndHalfClose -Client $drainControl -Commands @(
        'display-message -p "eof-drain-one"',
        'display-message -p "eof-drain-two"'
    )
    Write-Result 'half-closed input preserves every response body' `
        ($drained.Text -match '(?m)^eof-drain-one$' -and
            $drained.Text -match '(?m)^eof-drain-two$') `
        $drained.Text
    Write-Result 'half-closed input drains every command guard before socket EOF' `
        ($drained.BeginCount -eq 2 -and
            $drained.EndCount -eq 2 -and
            $drained.ErrorCount -eq 0 -and
            $drained.ElapsedMs -lt 5000) `
        "elapsed=$($drained.ElapsedMs)ms output=$($drained.Text)"
    Close-ControlClient $drainControl
    $drainControl = $null

    Write-Host "`n=== A non-reading control client is bounded ===" -ForegroundColor Cyan
    $slowControl = Open-ControlClient -Context $ctx -SessionBase $base -SlowReceive
    $escapedStarted = $pressureStartedFile.Replace("'", "''")
    $payload = "[IO.File]::WriteAllText('$escapedStarted','started');`$chunk='Z' * 65536;for(`$i=0;`$i -lt 512;`$i++){[Console]::Out.Write(`$chunk);[Console]::Out.Flush();[Threading.Thread]::Sleep(10)}"
    $pressureWatch = [System.Diagnostics.Stopwatch]::StartNew()
    & $PSMUX -L $namespace send-keys -t "${session}:0" -l $payload 2>&1 | Out-Null
    & $PSMUX -L $namespace send-keys -t "${session}:0" Enter 2>&1 | Out-Null
    $outputStarted = Wait-ForFile -Path $pressureStartedFile -TimeoutMs 5000
    Write-Result 'pane started the socket-pressure payload' $outputStarted `
        "start file did not appear at $pressureStartedFile"

    $healthyControl = Open-ControlClient -Context $ctx -SessionBase $base
    $healthy = Invoke-ControlCommand -Client $healthyControl `
        -Command 'display-message -p "healthy-client"' `
        -TimeoutMs $commandLimitMs
    Write-Result 'a second control client remains responsive' `
        ($healthy.Text -match '(?m)^healthy-client$' -and
            $healthy.ElapsedMs -lt $commandLimitMs -and
            $healthy.EndCount -eq 1) `
        "elapsed=$($healthy.ElapsedMs)ms output=$($healthy.Text)"
    Close-ControlClient $healthyControl
    $healthyControl = $null

    $remainingMs = 10000 - [int]$pressureWatch.ElapsedMilliseconds
    if ($remainingMs -gt 0) { Start-Sleep -Milliseconds $remainingMs }
    $closed = Wait-ControlClientClosed -Client $slowControl -TimeoutMs 5000
    Write-Result 'non-reading client is disconnected after the write timeout' `
        $closed.Closed "socket stayed open after draining $($closed.Characters) characters"
} catch {
    Write-Result 'test setup and protocol execution completed' $false $_.Exception.ToString()
} finally {
    Close-ControlClient $drainControl
    Close-ControlClient $healthyControl
    Close-ControlClient $slowControl
    Close-ControlClient $control
    if ($null -eq $savedConfig) { Remove-Item Env:\PSMUX_CONFIG_FILE -ErrorAction SilentlyContinue }
    else { $env:PSMUX_CONFIG_FILE = $savedConfig }
    if ($null -eq $savedDelay) { Remove-Item Env:\PSMUX_TEST_WARM_PANE_DELAY_MS -ErrorAction SilentlyContinue }
    else { $env:PSMUX_TEST_WARM_PANE_DELAY_MS = $savedDelay }
    if ($null -eq $savedMarker) { Remove-Item Env:\PSMUX_TEST_WARM_PANE_DELAY_STARTED_FILE -ErrorAction SilentlyContinue }
    else { $env:PSMUX_TEST_WARM_PANE_DELAY_STARTED_FILE = $savedMarker }
    Remove-PsmuxTestEnv -Ctx $ctx
}

Write-Host "`n=== Results: $pass passed, $fail failed ===" -ForegroundColor Cyan
if ($fail -gt 0) { exit 1 }
exit 0
