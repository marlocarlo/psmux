$ErrorActionPreference = "Stop"

$installPath = "C:\BuildTools"
$logFile     = "C:\vsbuildtools-install.log"

Write-Host "Downloading Visual Studio Build Tools..."
Invoke-WebRequest https://aka.ms/vs/17/release/vs_BuildTools.exe -OutFile C:\vs_BuildTools.exe

Write-Host "Installing Visual Studio Build Tools (this takes a while)..."
$proc = Start-Process -FilePath C:\vs_BuildTools.exe -ArgumentList @(
    "--quiet","--wait","--norestart","--nocache",
    "--installPath", $installPath,
    "--add","Microsoft.VisualStudio.Workload.VCTools",
    "--add","Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
    "--add","Microsoft.VisualStudio.Component.Windows10SDK.19041"
) -Wait -PassThru

Write-Host "Installer exit code: $($proc.ExitCode)"

Remove-Item C:\vs_BuildTools.exe -Force -ErrorAction SilentlyContinue

if (-not (Test-Path "$installPath\VC\Tools\MSVC")) {
    Write-Host "Build Tools install FAILED."
    # Try to find log files
    Get-ChildItem "C:\Users\ContainerAdministrator\AppData\Local\Temp" -Filter "dd_*.log" -ErrorAction SilentlyContinue | ForEach-Object {
        Write-Host "=== $($_.Name) (tail) ==="
        Get-Content $_.FullName -Tail 50
    }
    throw "MSVC toolset missing after install (exit code: $($proc.ExitCode))"
}

Write-Host "Visual Studio Build Tools installed successfully."

# Clean up installer temp files (extremely long paths break Docker layer commit)
Write-Host "Cleaning up temp files..."
Remove-Item "C:\Users\ContainerAdministrator\AppData\Local\Temp\*" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item "$env:TEMP\*" -Recurse -Force -ErrorAction SilentlyContinue
# Remove installer cache & logs
Remove-Item "C:\ProgramData\Package Cache\*" -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item "C:\BuildTools\Installer" -Recurse -Force -ErrorAction SilentlyContinue
Write-Host "Cleanup done."
