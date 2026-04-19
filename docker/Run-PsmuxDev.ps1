<#
.SYNOPSIS
    Build (if needed) and run the psmux-dev Docker container with SSH key auth.

.DESCRIPTION
    - Generates an SSH keypair in ~/.ssh/psmux_docker_key (if not present)
    - Builds the psmux-dev image (if not present)
    - Starts the container with your public key injected
    - Prints the SSH command to connect

.EXAMPLE
    pwsh -File docker\Run-PsmuxDev.ps1
    pwsh -File docker\Run-PsmuxDev.ps1 -Rebuild
#>
param(
    [switch]$Rebuild
)

$ErrorActionPreference = "Stop"

$imageName     = "psmux-dev"
$containerName = "psmux-dev"
$keyPath       = Join-Path $env:USERPROFILE ".ssh\psmux_docker_key"
$pubKeyPath    = "$keyPath.pub"
$dockerDir     = $PSScriptRoot  # docker/ folder

# ── 1. Generate SSH key if missing ──
if (-not (Test-Path $keyPath)) {
    Write-Host "Generating SSH key at $keyPath ..."
    $sshDir = Split-Path $keyPath
    if (-not (Test-Path $sshDir)) { New-Item -ItemType Directory -Path $sshDir -Force | Out-Null }
    ssh-keygen -t ed25519 -f $keyPath -N "" -C "psmux-docker" -q
    Write-Host "  Key generated."
} else {
    Write-Host "Using existing SSH key: $keyPath"
}

$pubKey = (Get-Content $pubKeyPath -Raw).Trim()
Write-Host "  Public key: $($pubKey.Substring(0, [Math]::Min(60, $pubKey.Length)))..."

# ── 2. Build image if needed ──
$imageExists = docker images $imageName -q 2>$null
if (-not $imageExists -or $Rebuild) {
    Write-Host ""
    Write-Host "Building $imageName image (this takes a while on first run)..."
    docker build -t $imageName $dockerDir
    if ($LASTEXITCODE -ne 0) { throw "Docker build failed" }
}

# ── 3. Remove old container if exists ──
$existing = docker ps -aq -f "name=$containerName" 2>$null
if ($existing) {
    Write-Host "Removing existing container..."
    docker rm -f $containerName 2>$null | Out-Null
}

# ── 4. Run container with public key ──
Write-Host "Starting container..."
docker run -d `
    --name $containerName `
    --isolation=hyperv `
    -e "SSH_PUBLIC_KEY=$pubKey" `
    $imageName | Out-Null

if ($LASTEXITCODE -ne 0) { throw "Docker run failed" }

# Wait for container to initialize
Write-Host "Waiting for sshd to start..."
Start-Sleep 5

# ── 5. Get container IP ──
$containerIP = docker inspect $containerName --format "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}"
if (-not $containerIP) { throw "Could not get container IP" }

# ── 6. Print connection info ──
Write-Host ""
Write-Host "============================================" -ForegroundColor Green
Write-Host " psmux dev container is running" -ForegroundColor Green
Write-Host "============================================" -ForegroundColor Green
Write-Host ""
Write-Host " Connect:" -ForegroundColor Cyan
Write-Host "   ssh -i ~/.ssh/psmux_docker_key -p 2222 ContainerAdministrator@$containerIP"
Write-Host ""
Write-Host " Quick build:" -ForegroundColor Cyan
Write-Host "   git clone https://github.com/psmux/psmux.git"
Write-Host "   cd psmux && cargo install --path ."
Write-Host ""
Write-Host " Stop:" -ForegroundColor Cyan
Write-Host "   docker stop $containerName"
Write-Host ""
Write-Host " Restart:" -ForegroundColor Cyan
Write-Host "   docker start $containerName"
Write-Host ""
Write-Host "============================================" -ForegroundColor Green

# ── 7. Verify SSH connectivity ──
Write-Host ""
Write-Host "Testing SSH connection..."
$result = ssh -i $keyPath -o StrictHostKeyChecking=no -o UserKnownHostsFile=NUL -o ConnectTimeout=10 -p 2222 ContainerAdministrator@$containerIP "hostname" 2>$null
if ($result) {
    Write-Host "  Connected to: $result" -ForegroundColor Green
} else {
    Write-Host "  SSH not ready yet — container may still be initializing." -ForegroundColor Yellow
    Write-Host "  Try manually: ssh -i ~/.ssh/psmux_docker_key -p 2222 ContainerAdministrator@$containerIP"
}
