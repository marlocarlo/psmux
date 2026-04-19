# escape=`
# psmux build environment: Windows + Rust (MSVC) + OpenSSH (key-only auth)
#
# Build:  docker build -t psmux-dev .
# Run:    pwsh -File docker\Run-PsmuxDev.ps1
# SSH:    (printed by Run-PsmuxDev.ps1 after container starts)
#
FROM mcr.microsoft.com/powershell:windowsservercore-ltsc2022

# NOTE: We keep the default cmd shell for RUN steps and invoke pwsh explicitly.
# Using SHELL ["pwsh",...] breaks Hyper-V isolated builds on Win11 25H2 hosts.

ENV VS_INSTALL_PATH="C:\BuildTools"
ENV RUSTUP_HOME="C:\rustup"
ENV CARGO_HOME="C:\cargo"
ENV PATH="C:\cargo\bin;C:\OpenSSH;C:\git\cmd;C:\Windows\System32;C:\Windows;C:\Program Files\PowerShell\latest;${PATH}"

# Create directories and copy all scripts first
RUN mkdir C:\Tools && mkdir C:\Profile && mkdir "C:\Users\ContainerAdministrator\Documents\PowerShell"
COPY Tools\  C:\Tools\
COPY Profile\ C:\Users\ContainerAdministrator\Documents\PowerShell\

# Single combined install step: Rust + VS Build Tools + OpenSSH + Git + cleanup
# (Combined to avoid Docker layer commit failures from VS Build Tools long paths)
RUN pwsh -NoLogo -NoProfile -ExecutionPolicy Bypass -File C:\Tools\InstallAll.ps1

EXPOSE 2222

CMD ["pwsh", "-NoLogo", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", "C:\\Tools\\StartContainer.ps1"]
