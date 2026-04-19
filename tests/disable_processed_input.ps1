Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public class ConMode {
    [DllImport("kernel32.dll")] public static extern IntPtr GetStdHandle(int h);
    [DllImport("kernel32.dll")] public static extern bool GetConsoleMode(IntPtr h, out uint m);
    [DllImport("kernel32.dll")] public static extern bool SetConsoleMode(IntPtr h, uint m);
}
"@

$h = [ConMode]::GetStdHandle(-10)
$m = [uint32]0
[ConMode]::GetConsoleMode($h, [ref]$m)
Write-Host "Mode_Before: $m (0x$($m.ToString('X4')))"

# Disable ENABLE_PROCESSED_INPUT (bit 0)
$newMode = $m -band (-bnot 1)
[ConMode]::SetConsoleMode($h, $newMode)

[ConMode]::GetConsoleMode($h, [ref]$m)
Write-Host "Mode_After: $m (0x$($m.ToString('X4')))"
Write-Host "ENABLE_PROCESSED_INPUT is now OFF - simulating TUI app exit"
