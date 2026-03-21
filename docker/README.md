# psmux Docker Dev Environment

A Windows container with Rust (MSVC), Visual Studio Build Tools, and OpenSSH — ready to build and run psmux.

## What's inside

| Component | Details |
|-----------|---------|
| Base image | `mcr.microsoft.com/powershell:windowsservercore-ltsc2022` |
| Rust | stable-x86_64-pc-windows-msvc via rustup |
| MSVC | Visual Studio Build Tools 2022 (`cl.exe`, `link.exe`) |
| SSH | OpenSSH Server on port 2222 (key-only auth, no passwords) |
| Shell | PowerShell 7 with auto-loaded VS dev environment |
| Git | MinGit for cloning repos |

## Quick start

### One command

```powershell
pwsh -File docker\Run-PsmuxDev.ps1
```

This will:
1. Generate an SSH key at `~/.ssh/psmux_docker_key` (if not present)
2. Build the Docker image (first time only)
3. Start the container with your public key injected
4. Print the SSH command to connect

### Manual steps

#### 1. Build the image

```powershell
cd docker
docker build -t psmux-dev .
```

> **Note:** The build takes a while (~15-30 min) because it downloads and installs Visual Studio Build Tools. The resulting image is large (~15 GB). This is expected for Windows MSVC containers.

#### 2. Run the container

```powershell
# Generate SSH key (once)
ssh-keygen -t ed25519 -f ~/.ssh/psmux_docker_key -N "" -C "psmux-docker"

# Run with your public key
$pubkey = Get-Content ~/.ssh/psmux_docker_key.pub
docker run -d --name psmux-dev --isolation=hyperv `
    -e "SSH_PUBLIC_KEY=$pubkey" `
    psmux-dev
```

#### 3. SSH in

```powershell
$ip = docker inspect psmux-dev --format "{{range .NetworkSettings.Networks}}{{.IPAddress}}{{end}}"
ssh -i ~/.ssh/psmux_docker_key -p 2222 ContainerAdministrator@$ip
```

### 4. Build psmux

```powershell
git clone https://github.com/psmux/psmux.git
cd psmux
cargo install --path .
psmux --version
```

## SSH authentication

This container uses **key-only SSH** — no passwords. Your public key is passed in via the `SSH_PUBLIC_KEY` environment variable at container start. The `Run-PsmuxDev.ps1` script handles this automatically.

You can also mount a public key file:

```powershell
docker run -d --name psmux-dev --isolation=hyperv `
    -v "$HOME\.ssh\id_ed25519.pub:C:\ssh_public_key" `
    psmux-dev
```

## Verifying the toolchain

After SSH-ing in, these should all work:

```powershell
rustc --version
cargo --version
where cl
where link
```

## Safety notes

- No passwords are used — SSH key auth only
- Container runs with Hyper-V isolation (full VM separation from host)
- SSH listens on port 2222 to avoid conflicts with host sshd
- Key is stored at `~/.ssh/psmux_docker_key` (never inside the repo)

## File layout

```
docker/
  Dockerfile
  README.md
  Run-PsmuxDev.ps1               # Host-side: generates key, builds, runs, prints SSH command
  Tools/
    StartContainer.ps1            # Entrypoint: configures sshd with key auth, starts sshd
    InstallAll.ps1                # Build-time: installs Rust, VS Build Tools, OpenSSH, Git
    ImportVsDevEnv.ps1            # Loads VS dev environment (cl.exe, link.exe) into PowerShell
  Profile/
    Microsoft.PowerShell_profile.ps1   # Auto-loads Rust + MSVC env on every shell
```
