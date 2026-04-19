$ErrorActionPreference = "Continue"

# ============================================
# 0. Download everything first (before heavy installs use up disk/memory)
# ============================================
Write-Host "=== Downloading all installers ==="
[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12

Write-Host "  Downloading Rust..."
Invoke-WebRequest https://win.rustup.rs -OutFile C:\rustup-init.exe -UseBasicParsing

Write-Host "  Downloading VS Build Tools..."
Invoke-WebRequest https://aka.ms/vs/17/release/vs_BuildTools.exe -OutFile C:\vs_BuildTools.exe -UseBasicParsing

Write-Host "  Downloading OpenSSH..."
try {
    Invoke-WebRequest "https://github.com/PowerShell/Win32-OpenSSH/releases/download/v9.8.3.0p2-Preview/OpenSSH-Win64.zip" -OutFile C:\openssh.zip -UseBasicParsing
    Write-Host "  OpenSSH downloaded: $((Get-Item C:\openssh.zip).Length) bytes"
} catch {
    Write-Host "  OpenSSH download error: $($_.Exception.Message)"
    exit 1
}

Write-Host "  Downloading Git..."
try {
    Invoke-WebRequest "https://github.com/git-for-windows/git/releases/download/v2.47.1.windows.2/MinGit-2.47.1.2-64-bit.zip" -OutFile C:\git.zip -UseBasicParsing
    Write-Host "  Git downloaded: $((Get-Item C:\git.zip).Length) bytes"
} catch {
    Write-Host "  Git download error: $($_.Exception.Message)"
    exit 1
}

Write-Host "All downloads complete."

# ============================================
# 1. Install Rust
# ============================================
Write-Host "=== Installing Rust ===" 
Start-Process -FilePath C:\rustup-init.exe -ArgumentList "-y" -Wait
Remove-Item C:\rustup-init.exe -Force
& C:\cargo\bin\rustup.exe default stable-x86_64-pc-windows-msvc
& C:\cargo\bin\rustc.exe --version
& C:\cargo\bin\cargo.exe --version

# ============================================
# 2. Install Visual Studio Build Tools
# ============================================
Write-Host "=== Installing Visual Studio Build Tools ==="
$proc = Start-Process -FilePath C:\vs_BuildTools.exe -ArgumentList @(
    "--quiet","--wait","--norestart","--nocache",
    "--installPath","C:\BuildTools",
    "--add","Microsoft.VisualStudio.Workload.VCTools",
    "--add","Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
    "--add","Microsoft.VisualStudio.Component.Windows10SDK.19041"
) -Wait -PassThru
Write-Host "VS Build Tools exit code: $($proc.ExitCode)"
Remove-Item C:\vs_BuildTools.exe -Force -ErrorAction SilentlyContinue

if (-not (Test-Path "C:\BuildTools\VC\Tools\MSVC")) {
    throw "MSVC toolset missing after install"
}
Write-Host "VS Build Tools installed."

# Clean up VS installer temp immediately to free disk space
Remove-Item "C:\Users\ContainerAdministrator\AppData\Local\Temp\*" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item "C:\ProgramData\Package Cache" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item "C:\BuildTools\Installer" -Recurse -Force -ErrorAction SilentlyContinue

# ============================================
# 3. Install OpenSSH Server
# ============================================
Write-Host "=== Installing OpenSSH Server ==="
Add-Type -AssemblyName System.IO.Compression.FileSystem
[System.IO.Compression.ZipFile]::ExtractToDirectory("C:\openssh.zip", "C:\sshtmp")
Remove-Item C:\openssh.zip -Force

# Move extracted dir (OpenSSH-Win64) to C:\OpenSSH
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

# ============================================
# 4. Install Git
# ============================================
Write-Host "=== Installing Git ==="
Expand-Archive C:\git.zip -DestinationPath C:\git -Force
Remove-Item C:\git.zip -Force
[Environment]::SetEnvironmentVariable("PATH", "C:\git\cmd;" + [Environment]::GetEnvironmentVariable("PATH","Machine"), "Machine")
Write-Host "Git installed."

# ============================================
# 5. Aggressive cleanup to avoid Docker layer commit failures
# ============================================
Write-Host "=== Cleanup ==="
# VS Build Tools installer caches and temp files with very long paths
Remove-Item "C:\Users\ContainerAdministrator\AppData\Local\Temp\*" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item "$env:TEMP\*" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item "C:\ProgramData\Package Cache" -Recurse -Force -ErrorAction SilentlyContinue
# Remove VS installer metadata (keeps long path dirs)
Remove-Item "C:\BuildTools\Installer" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item "C:\ProgramData\Microsoft\VisualStudio" -Recurse -Force -ErrorAction SilentlyContinue
# Remove NuGet cache
Remove-Item "C:\Users\ContainerAdministrator\.nuget" -Recurse -Force -ErrorAction SilentlyContinue

Write-Host "=== All installations complete ==="
