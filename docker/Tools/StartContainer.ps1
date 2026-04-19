$ErrorActionPreference = "Stop"

$opensshDir   = "C:\OpenSSH"
$sshdConfig   = "C:\ProgramData\ssh\sshd_config"
$adminKeysFile = "C:\ProgramData\ssh\administrators_authorized_keys"

# ── Ensure critical system paths and OpenSSH are on PATH ──
# (docker commit can lose System32 from PATH depending on how ENV was set)
# Use exact-match check to avoid substring false positives (e.g. System32\OpenSSH)
$pathEntries = $env:PATH -split ';' | ForEach-Object { $_.TrimEnd('\') }
foreach ($p in @("C:\Windows\System32", "C:\Windows", $opensshDir, "C:\git\cmd")) {
    if ($p.TrimEnd('\') -notin $pathEntries) { $env:PATH = "$p;$env:PATH" }
}
# Also persist to machine-level so SSH sessions inherit the full PATH
[Environment]::SetEnvironmentVariable("PATH", $env:PATH, "Machine")

# ── Set default shell to PowerShell ──
New-Item -Path "HKLM:\SOFTWARE\OpenSSH" -Force | Out-Null
$pwshPath = (Get-Command pwsh -ErrorAction SilentlyContinue).Source
if (-not $pwshPath) { $pwshPath = "C:\Program Files\PowerShell\7\pwsh.exe" }
New-ItemProperty -Path "HKLM:\SOFTWARE\OpenSSH" -Name "DefaultShell" -Value $pwshPath -PropertyType String -Force | Out-Null
New-ItemProperty -Path "HKLM:\SOFTWARE\OpenSSH" -Name "DefaultShellCommandOption" -Value "-c" -PropertyType String -Force | Out-Null

# ── Generate host keys if missing ──
New-Item -ItemType Directory -Path "C:\ProgramData\ssh" -Force | Out-Null
if (-not (Test-Path "C:\ProgramData\ssh\ssh_host_ed25519_key")) {
    & "$opensshDir\ssh-keygen.exe" -A 2>$null
}

# ── Write sshd_config: key-only auth, port 2222 ──
@"
Port 2222
ListenAddress 0.0.0.0
HostKey C:/ProgramData/ssh/ssh_host_rsa_key
HostKey C:/ProgramData/ssh/ssh_host_ecdsa_key
HostKey C:/ProgramData/ssh/ssh_host_ed25519_key
PasswordAuthentication no
PubkeyAuthentication yes
StrictModes no
Subsystem sftp C:/OpenSSH/sftp-server.exe

Match Group administrators
       AuthorizedKeysFile __PROGRAMDATA__/ssh/administrators_authorized_keys
"@ | Set-Content -Path $sshdConfig -Encoding ascii

# ── Install authorized public key ──
# Accepts key via: SSH_PUBLIC_KEY env var, or mounted file at C:\ssh_public_key
$pubkey = $env:SSH_PUBLIC_KEY
if (-not $pubkey -and (Test-Path "C:\ssh_public_key")) {
    $pubkey = (Get-Content "C:\ssh_public_key" -Raw).Trim()
}
if (-not $pubkey) {
    Write-Host ""
    Write-Host "ERROR: No SSH public key provided." -ForegroundColor Red
    Write-Host "Pass your public key via one of:"
    Write-Host '  -e SSH_PUBLIC_KEY="ssh-ed25519 AAAA..."'
    Write-Host '  -v C:\Users\you\.ssh\id_ed25519.pub:C:\ssh_public_key'
    Write-Host ""
    exit 1
}

Set-Content -Path $adminKeysFile -Value $pubkey -Encoding ascii
# Note: StrictModes is disabled in sshd_config, so we don't need to set
# restrictive ACLs on administrators_authorized_keys. Setting ACLs via
# icacls or Set-Acl crashes Hyper-V isolated containers.

# ── Start sshd directly (not as a service) ──
$sshdProc = Start-Process -FilePath "$opensshDir\sshd.exe" `
    -ArgumentList "-f", $sshdConfig `
    -PassThru -WindowStyle Hidden

Start-Sleep 2
if ($sshdProc.HasExited) {
    Write-Host "ERROR: sshd failed to start (exit code $($sshdProc.ExitCode))" -ForegroundColor Red
    & "$opensshDir\sshd.exe" -f $sshdConfig -d -d 2>&1 | Select-Object -First 20
    exit 1
}

# ── Print connection info ──
# Get-NetIPAddress isn't available in windowsservercore containers; use .NET instead
$ip = ([System.Net.Dns]::GetHostAddresses($env:COMPUTERNAME) |
    Where-Object { $_.AddressFamily -eq 'InterNetwork' -and $_.ToString() -ne '127.0.0.1' } |
    Select-Object -First 1).ToString()
Write-Host ""
Write-Host "============================================"
Write-Host " psmux dev container ready" -ForegroundColor Green
Write-Host "============================================"
Write-Host " SSH  : ssh -i ~/.ssh/psmux_docker_key -p 2222 ContainerAdministrator@$ip"
Write-Host " Rust : $(& rustc --version 2>$null)"
Write-Host " Cargo: $(& cargo --version 2>$null)"
Write-Host "============================================"
Write-Host ""
Write-Host "Quick start:"
Write-Host "  git clone https://github.com/psmux/psmux.git"
Write-Host "  cd psmux"
Write-Host "  cargo install --path ."
Write-Host ""

# Keep container alive + restart sshd if it crashes
while ($true) {
    Start-Sleep -Seconds 30
    if ($sshdProc.HasExited) {
        Write-Host "sshd exited, restarting..."
        $sshdProc = Start-Process -FilePath "$opensshDir\sshd.exe" `
            -ArgumentList "-f", $sshdConfig `
            -PassThru -WindowStyle Hidden
    }
}
