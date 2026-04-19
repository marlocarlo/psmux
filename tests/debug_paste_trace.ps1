#!/usr/bin/env pwsh
# Quick paste debug trace: start psmux, send Ctrl+V via keybd_event, dump log
$ErrorActionPreference = 'Stop'

Add-Type @"
using System;
using System.Runtime.InteropServices;
using System.Collections.Generic;
using System.Text;
public class PD {
    [DllImport("user32.dll")] public static extern void keybd_event(byte bVk,byte bScan,uint dwFlags,IntPtr dwExtra);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern bool EnumWindows(CallBack cb, IntPtr p);
    [DllImport("user32.dll")] static extern bool IsWindowVisible(IntPtr h);
    [DllImport("user32.dll")] static extern int GetWindowTextLength(IntPtr h);
    [DllImport("user32.dll")] static extern int GetWindowText(IntPtr h, StringBuilder sb, int max);
    delegate bool CallBack(IntPtr h, IntPtr p);
    public static List<IntPtr> Find(string sub) {
        var r = new List<IntPtr>();
        EnumWindows((h,p)=>{
            if(!IsWindowVisible(h)) return true;
            int n=GetWindowTextLength(h); if(n==0) return true;
            var sb=new StringBuilder(n+1); GetWindowText(h,sb,sb.Capacity);
            if(sb.ToString().IndexOf(sub,StringComparison.OrdinalIgnoreCase)>=0) r.Add(h);
            return true;
        }, IntPtr.Zero);
        return r;
    }
}
"@

# Clean up
Stop-Process -Name psmux -Force -EA SilentlyContinue
Start-Sleep -Milliseconds 500
Remove-Item "$env:USERPROFILE\.psmux\input_debug.log" -EA SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\client_paste.log" -EA SilentlyContinue

# Set clipboard
Set-Clipboard "PASTETEST1"

# Launch psmux with debug env
$psi = New-Object System.Diagnostics.ProcessStartInfo
$psi.FileName = "$PSScriptRoot\_launch_debug.bat"
$psi.UseShellExecute = $true
$psi.WindowStyle = 'Normal'
$p = [System.Diagnostics.Process]::Start($psi)
Write-Host "Started psmux via cmd, PID=$($p.Id)"
Start-Sleep -Seconds 4

# Find psmux window
$wins = [PD]::Find("psmux")
if ($wins.Count -eq 0) { $wins = [PD]::Find("cmd") }
Write-Host "Found $($wins.Count) windows"
if ($wins.Count -gt 0) {
    [PD]::SetForegroundWindow($wins[0]) | Out-Null
    Start-Sleep -Milliseconds 500
}

# Ctrl+V
Write-Host "Sending Ctrl+V..."
[PD]::keybd_event(0x11, 0, 0, [IntPtr]::Zero)   # Ctrl down
[PD]::keybd_event(0x56, 0, 0, [IntPtr]::Zero)   # V down  
[PD]::keybd_event(0x56, 0, 2, [IntPtr]::Zero)   # V up
[PD]::keybd_event(0x11, 0, 2, [IntPtr]::Zero)   # Ctrl up
Start-Sleep -Seconds 3

# Read log
Write-Host "`n===== CLIENT PASTE LOG ====="
$clientLog = "$env:USERPROFILE\.psmux\client_paste.log"
if (Test-Path $clientLog) {
    Get-Content $clientLog
} else {
    Write-Host "NO CLIENT PASTE LOG"
}

Write-Host "`n===== DEBUG LOG ====="
$logPath = "$env:USERPROFILE\.psmux\input_debug.log"
if (Test-Path $logPath) {
    Get-Content $logPath
} else {
    Write-Host "NO LOG FILE at $logPath"
}

# Cleanup
Stop-Process -Name psmux -Force -EA SilentlyContinue
