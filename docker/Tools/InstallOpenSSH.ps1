$ErrorActionPreference = "Stop"

$url = "https://github.com/PowerShell/Win32-OpenSSH/releases/download/v9.8.3.0p2-Preview/OpenSSH-Win64.zip"

Write-Host "Downloading Win32-OpenSSH..."
Invoke-WebRequest $url -OutFile C:\openssh.zip -UseBasicParsing

Write-Host "Extracting..."
Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::ExtractToDirectory("C:\openssh.zip", "C:\sshtmp")
Remove-Item C:\openssh.zip -Force

$extracted = Get-ChildItem "C:\sshtmp" -Directory | Select-Object -First 1
if ($extracted) {
    Move-Item $extracted.FullName "C:\OpenSSH" -Force
} else {
    Move-Item "C:\sshtmp" "C:\OpenSSH" -Force
}
Remove-Item "C:\sshtmp" -Recurse -Force -ErrorAction SilentlyContinue

if (Test-Path "C:\OpenSSH\sshd.exe") {
    Write-Host "OpenSSH Server installed to C:\OpenSSH"
} else {
    throw "sshd.exe not found after OpenSSH install"
}
