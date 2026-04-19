$ErrorActionPreference = "Stop"

Write-Host "Downloading rustup..."
Invoke-WebRequest https://win.rustup.rs -OutFile C:\rustup-init.exe

Write-Host "Installing Rust..."
Start-Process -FilePath C:\rustup-init.exe -ArgumentList "-y" -Wait
Remove-Item C:\rustup-init.exe -Force

& C:\cargo\bin\rustup.exe default stable-x86_64-pc-windows-msvc
& C:\cargo\bin\rustc.exe --version
& C:\cargo\bin\cargo.exe --version
Write-Host "Rust installed."
