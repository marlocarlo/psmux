$ErrorActionPreference = "Stop"

$url = "https://github.com/git-for-windows/git/releases/download/v2.47.1.windows.2/MinGit-2.47.1.2-64-bit.zip"

Write-Host "Downloading MinGit..."
Invoke-WebRequest $url -OutFile C:\git.zip

Write-Host "Extracting..."
Expand-Archive C:\git.zip -DestinationPath C:\git -Force
Remove-Item C:\git.zip -Force

[Environment]::SetEnvironmentVariable("PATH", "C:\git\cmd;" + [Environment]::GetEnvironmentVariable("PATH", "Machine"), "Machine")
Write-Host "Git installed."
