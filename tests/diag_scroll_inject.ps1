# Test script to inject scroll events to psmux server via TCP (persistent mode)
param(
    [int]$Count = 10,
    [string]$Direction = "scroll-down",
    [int]$X = 10,
    [int]$Y = 10
)

$ErrorActionPreference = 'Stop'

$portFile = "$env:USERPROFILE\.psmux\mtest.port"
$keyFile = "$env:USERPROFILE\.psmux\mtest.key"

if (!(Test-Path $portFile)) { Write-Error "No port file: $portFile"; exit 1 }
if (!(Test-Path $keyFile)) { Write-Error "No key file: $keyFile"; exit 1 }

$port = [int](Get-Content $portFile)
$key = (Get-Content $keyFile).Trim()

Write-Host "Connecting to 127.0.0.1:$port..."
$tcp = [System.Net.Sockets.TcpClient]::new()
$tcp.NoDelay = $true
$tcp.Connect("127.0.0.1", $port)

$stream = $tcp.GetStream()
$writer = [System.IO.StreamWriter]::new($stream)
$writer.AutoFlush = $true

# Send AUTH + PERSISTENT immediately (no delay!)
$writer.WriteLine("AUTH $key")
$writer.WriteLine("PERSISTENT")

# Small delay for server to process auth
Start-Sleep -Milliseconds 100

# Send scroll events
for ($i = 0; $i -lt $Count; $i++) {
    $cmd = "$Direction $X $Y"
    $writer.WriteLine($cmd)
    Write-Host "Sent: $cmd"
    Start-Sleep -Milliseconds 50
}

Start-Sleep -Milliseconds 500
$tcp.Close()
Write-Host "Done. Sent $Count $Direction events."
