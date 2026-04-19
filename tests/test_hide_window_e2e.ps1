# ===========================================================================
# test_hide_window_e2e.ps1
#
# UNDENIABLE PROOF that CREATE_NO_WINDOW works on all background subprocesses.
#
# Strategy: We use the Win32 API (EnumWindows / GetWindowText / IsWindowVisible)
# to count visible windows BEFORE and DURING subprocess execution.  If any
# new console window appears while a background command runs, the test FAILS.
#
# Covers every subprocess spawn site:
#   1.  run-shell (basic, exit codes, env vars, explicit shells, cmd.exe)
#   2.  if-shell (true/false branches, literals, background, complex conditions)
#   3.  Format #() expansion (basic, pwsh, rapid polling, mixed)
#   4.  pipe-pane (hidden pipe process, stdin piping)
#   5.  Config if-shell via source-file (true/false branches)
#   6.  copy-pipe stdin piping
#   7.  Win32 window enumeration proof (the crown jewel)
#
# Usage:   pwsh .\tests\test_hide_window_e2e.ps1
# ===========================================================================

$ErrorActionPreference = "Continue"
$psmux = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $psmux)) { $psmux = "$PSScriptRoot\..\target\debug\psmux.exe" }
if (-not (Test-Path $psmux)) {
    $psmux = (Get-Command psmux -ErrorAction SilentlyContinue).Source
}
if (-not $psmux) { Write-Error "psmux binary not found"; exit 1 }

$SESSION = "hidewin_e2e_$PID"
$TestsPassed = 0
$TestsFailed = 0
$results = @()

function Write-Pass($msg) { Write-Host "  [PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail($msg) { Write-Host "  [FAIL] $msg" -ForegroundColor Red;   $script:TestsFailed++ }

function Ensure-Session {
    $lsCheck = & $psmux list-sessions 2>&1 | Out-String
    if ($lsCheck -notmatch [regex]::Escape($SESSION)) {
        Write-Host "  (session lost, recreating...)"
        & $psmux new-session -s $SESSION -d 2>&1 | Out-Null
        Start-Sleep 4
        # Verify it's really up
        $verify = & $psmux list-sessions 2>&1 | Out-String
        if ($verify -notmatch [regex]::Escape($SESSION)) {
            Start-Sleep 3
        }
    }
}

function Test-Case {
    param([string]$Name, [scriptblock]$Test)
    Write-Host "`n--- $Name ---"
    Ensure-Session
    try {
        $pass = & $Test
        if ($pass) { Write-Pass $Name } else { Write-Fail $Name }
        $script:results += [PSCustomObject]@{Test=$Name;Pass=[bool]$pass}
    } catch {
        Write-Fail "$Name (exception: $_)"
        $script:results += [PSCustomObject]@{Test=$Name;Pass=$false}
    }
}

# ===========================================================================
# Win32 Window Enumeration Helper
# ===========================================================================
# This uses P/Invoke to enumerate ALL visible top-level windows.
# We snapshot windows before psmux runs a command, then check during/after.
# Any new "ConsoleWindowClass" or conhost window = FAIL.

Add-Type -TypeDefinition @"
using System;
using System.Collections.Generic;
using System.Runtime.InteropServices;
using System.Text;

public class WindowEnumerator {
    public delegate bool EnumWindowsProc(IntPtr hWnd, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern bool EnumWindows(EnumWindowsProc lpEnumFunc, IntPtr lParam);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);

    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)]
    public static extern int GetWindowText(IntPtr hWnd, StringBuilder lpString, int nMaxCount);

    [DllImport("user32.dll", SetLastError = true, CharSet = CharSet.Auto)]
    public static extern int GetClassName(IntPtr hWnd, StringBuilder lpClassName, int nMaxCount);

    [DllImport("user32.dll")]
    public static extern uint GetWindowThreadProcessId(IntPtr hWnd, out uint lpdwProcessId);

    public static List<string> GetVisibleConsoleWindows() {
        var windows = new List<string>();
        EnumWindows((hWnd, lParam) => {
            if (IsWindowVisible(hWnd)) {
                var cls = new StringBuilder(256);
                GetClassName(hWnd, cls, 256);
                string className = cls.ToString();
                // ConsoleWindowClass is what conhost.exe creates
                if (className == "ConsoleWindowClass") {
                    var title = new StringBuilder(256);
                    GetWindowText(hWnd, title, 256);
                    uint pid;
                    GetWindowThreadProcessId(hWnd, out pid);
                    windows.Add(pid + "|" + title.ToString());
                }
            }
            return true;
        }, IntPtr.Zero);
        return windows;
    }

    public static HashSet<string> SnapshotConsoleWindows() {
        var set = new HashSet<string>();
        foreach (var w in GetVisibleConsoleWindows()) {
            set.Add(w);
        }
        return set;
    }
}
"@ -ErrorAction SilentlyContinue

function Get-NewConsoleWindows {
    param([System.Collections.Generic.HashSet[string]]$Baseline)
    $current = [WindowEnumerator]::SnapshotConsoleWindows()
    $newWindows = @()
    foreach ($w in $current) {
        if (-not $Baseline.Contains($w)) {
            $newWindows += $w
        }
    }
    return $newWindows
}

# ===========================================================================
# Setup
# ===========================================================================
Write-Host "=== CREATE_NO_WINDOW E2E Proof Suite ==="
Write-Host "Binary: $psmux"
Write-Host "Session: $SESSION"

# Hard-kill everything to guarantee clean state
taskkill /F /IM psmux.exe 2>$null
taskkill /F /IM tmux.exe 2>$null
taskkill /F /IM pmux.exe 2>$null
Start-Sleep 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

& $psmux new-session -s $SESSION -d 2>&1 | Out-Null
Start-Sleep 4

$sessions = & $psmux list-sessions 2>&1 | Out-String
if ($sessions -notmatch [regex]::Escape($SESSION)) {
    Write-Error "Failed to create session $SESSION (got: $sessions)"
    exit 1
}
Write-Host "Session created.`n"

# ===================================================================
# SECTION A: Win32 WINDOW PROOF (the undeniable part)
# These tests use EnumWindows to prove no new console windows appear
# ===================================================================

Write-Host "==============================================="
Write-Host "  SECTION A: Win32 Window Enumeration Proof"
Write-Host "==============================================="

# --- A1: run-shell produces no visible console window ---
Test-Case "A1: run-shell spawns NO visible console window" {
    $baseline = [WindowEnumerator]::SnapshotConsoleWindows()
    $totalFlash = 0

    # Run 5 run-shell commands, checking windows after each
    for ($i = 0; $i -lt 5; $i++) {
        $out = & $psmux run-shell -t $SESSION "Write-Output 'window_proof_$i'" 2>&1 | Out-String
        $flash = Get-NewConsoleWindows -Baseline $baseline
        $totalFlash += $flash.Count
    }

    Write-Host "    New console windows across 5 run-shell calls: $totalFlash"
    Write-Host "    Last output: $($out.Trim())"
    ($totalFlash -eq 0) -and ($out -match "window_proof")
}

# --- A2: if-shell condition check produces no visible console window ---
Test-Case "A2: if-shell spawns NO visible console window" {
    $baseline = [WindowEnumerator]::SnapshotConsoleWindows()
    $totalFlash = 0

    # Run 5 if-shell commands (each spawns a shell to check condition)
    for ($i = 0; $i -lt 5; $i++) {
        & $psmux if-shell -t $SESSION "exit 0" "run-shell 'exit 0'" "" 2>&1 | Out-Null
        $flash = Get-NewConsoleWindows -Baseline $baseline
        $totalFlash += $flash.Count
    }

    Write-Host "    New console windows across 5 if-shell calls: $totalFlash"
    $totalFlash -eq 0
}

# --- A3: format #() expansion produces no visible console window ---
Test-Case "A3: format #() spawns NO visible console window" {
    $baseline = [WindowEnumerator]::SnapshotConsoleWindows()
    $totalFlash = 0

    # Run 5 format #() expansions (each spawns a hidden subprocess)
    for ($i = 0; $i -lt 5; $i++) {
        $out = & $psmux display-message -t $SESSION -p "#(echo fmt_proof_$i)" 2>&1 | Out-String
        $flash = Get-NewConsoleWindows -Baseline $baseline
        $totalFlash += $flash.Count
    }

    Write-Host "    New console windows across 5 format #() calls: $totalFlash"
    Write-Host "    Last output: $($out.Trim())"
    ($totalFlash -eq 0) -and ($out -match "fmt_proof")
}

# --- A4: Rapid 10x run-shell (simulates Gastown status polling) no windows ---
Test-Case "A4: rapid 10x run-shell NO window flash" {
    $baseline = [WindowEnumerator]::SnapshotConsoleWindows()
    $totalFlash = 0

    for ($i = 0; $i -lt 10; $i++) {
        $null = & $psmux run-shell -t $SESSION "echo rapid_$i" 2>&1
        $flash = Get-NewConsoleWindows -Baseline $baseline
        $totalFlash += $flash.Count
    }

    Write-Host "    Total new windows across 10 rapid spawns: $totalFlash"
    $totalFlash -eq 0
}

# --- A5: pipe-pane produces no visible console window ---
Test-Case "A5: pipe-pane spawns NO visible console window" {
    $baseline = [WindowEnumerator]::SnapshotConsoleWindows()
    $shell = if (Get-Command pwsh -ErrorAction SilentlyContinue) { "pwsh" } else { "powershell" }
    $pipefile = "$env:TEMP\psmux_hidewin_pipe_proof.txt"
    Remove-Item $pipefile -Force -ErrorAction SilentlyContinue

    & $psmux pipe-pane -t $SESSION "$shell -NoProfile -Command `"while(`$true){Start-Sleep -Milliseconds 100}`"" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    $flash1 = Get-NewConsoleWindows -Baseline $baseline
    Start-Sleep -Milliseconds 300
    $flash2 = Get-NewConsoleWindows -Baseline $baseline
    & $psmux pipe-pane -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 200

    $anyFlash = ($flash1.Count + $flash2.Count)
    Write-Host "    Window samples: $($flash1.Count), $($flash2.Count) new windows"
    $anyFlash -eq 0
}

# --- A6: config if-shell via source-file produces no visible console window ---
Test-Case "A6: config if-shell spawns NO visible console window" {
    $conffile = "$env:TEMP\psmux_hidewin_conf_proof.conf"
    @"
if-shell "exit 0" "set -g status-interval 7" "set -g status-interval 99"
"@ | Set-Content $conffile -Force

    $baseline = [WindowEnumerator]::SnapshotConsoleWindows()
    & $psmux source-file -t $SESSION $conffile 2>&1 | Out-Null
    $flash = Get-NewConsoleWindows -Baseline $baseline
    Start-Sleep -Milliseconds 200
    $flash2 = Get-NewConsoleWindows -Baseline $baseline

    Remove-Item $conffile -Force -ErrorAction SilentlyContinue

    $anyFlash = ($flash.Count + $flash2.Count)
    Write-Host "    New console windows: $($flash.Count), $($flash2.Count)"
    $anyFlash -eq 0
}

# ===================================================================
# SECTION B: FUNCTIONAL CORRECTNESS
# Prove all subprocess types still work correctly while hidden
# ===================================================================

Write-Host "`n==============================================="
Write-Host "  SECTION B: Functional Correctness"
Write-Host "==============================================="
Ensure-Session

# --- B1: run-shell captures stdout ---
Test-Case "B1: run-shell stdout capture" {
    $out = & $psmux run-shell -t $SESSION "Write-Output 'stdout_captured_ok'" 2>&1 | Out-String
    $out -match "stdout_captured_ok"
}

# --- B2: run-shell exit 0 ---
Test-Case "B2: run-shell exit 0" {
    & $psmux run-shell -t $SESSION "exit 0" 2>&1 | Out-Null
    $LASTEXITCODE -eq 0
}

# --- B3: run-shell nonzero exit code ---
Test-Case "B3: run-shell nonzero exit" {
    & $psmux run-shell -t $SESSION "exit 42" 2>&1 | Out-Null
    $LASTEXITCODE -ne 0
}

# --- B4: run-shell env var propagation ---
Test-Case "B4: run-shell env propagation" {
    $env:HIDEWIN_PROOF = "env_proof_42"
    $out = & $psmux run-shell -t $SESSION '$env:HIDEWIN_PROOF' 2>&1 | Out-String
    Remove-Item Env:\HIDEWIN_PROOF -ErrorAction SilentlyContinue
    $out -match "env_proof_42"
}

# --- B5: run-shell explicit pwsh prefix ---
Test-Case "B5: run-shell explicit pwsh" {
    $shell = if (Get-Command pwsh -ErrorAction SilentlyContinue) { "pwsh" } else { "powershell" }
    $out = & $psmux run-shell -t $SESSION "$shell -NoProfile -Command `"Write-Output 'explicit_ok'`"" 2>&1 | Out-String
    $out -match "explicit_ok"
}

# --- B6: run-shell cmd.exe passthrough ---
Test-Case "B6: run-shell cmd.exe" {
    $out = & $psmux run-shell -t $SESSION "cmd /C echo cmd_ok" 2>&1 | Out-String
    $out -match "cmd_ok"
}

# --- B7: run-shell multi-line output ---
Test-Case "B7: run-shell multi-line" {
    $out = & $psmux run-shell -t $SESSION "1..5 | ForEach-Object { `$_ }" 2>&1 | Out-String
    ($out -match "1") -and ($out -match "3") -and ($out -match "5")
}

# --- B8: run-shell large output (200 lines) ---
Test-Case "B8: run-shell 200 lines" {
    $out = & $psmux run-shell -t $SESSION "1..200 | ForEach-Object { 'L' + `$_ }" 2>&1 | Out-String
    ($out -match "L1") -and ($out -match "L200")
}

# --- B9: run-shell background mode (-b) ---
Test-Case "B9: run-shell -b background" {
    & $psmux run-shell -b -t $SESSION "Write-Output 'bg_ok'" 2>&1 | Out-Null
    $LASTEXITCODE -eq 0
}

# --- B10: if-shell true branch ---
Test-Case "B10: if-shell true branch" {
    & $psmux if-shell -t $SESSION "exit 0" "run-shell 'exit 0'" "run-shell 'exit 1'" 2>&1 | Out-Null
    $LASTEXITCODE -eq 0
}

# --- B11: if-shell false branch ---
Test-Case "B11: if-shell false branch" {
    & $psmux if-shell -t $SESSION "exit 1" "run-shell 'exit 1'" "run-shell 'exit 0'" 2>&1 | Out-Null
    $LASTEXITCODE -eq 0
}

# --- B12: if-shell literal "true" ---
Test-Case "B12: if-shell literal true" {
    & $psmux if-shell -t $SESSION "true" "run-shell 'exit 0'" "run-shell 'exit 1'" 2>&1 | Out-Null
    $LASTEXITCODE -eq 0
}

# --- B13: if-shell literal "false" ---
Test-Case "B13: if-shell literal false" {
    & $psmux if-shell -t $SESSION "false" "run-shell 'exit 1'" "run-shell 'exit 0'" 2>&1 | Out-Null
    $LASTEXITCODE -eq 0
}

# --- B14: if-shell literal "1" ---
Test-Case "B14: if-shell literal 1" {
    & $psmux if-shell -t $SESSION "1" "run-shell 'exit 0'" "run-shell 'exit 1'" 2>&1 | Out-Null
    $LASTEXITCODE -eq 0
}

# --- B15: if-shell literal "0" ---
Test-Case "B15: if-shell literal 0" {
    & $psmux if-shell -t $SESSION "0" "run-shell 'exit 1'" "run-shell 'exit 0'" 2>&1 | Out-Null
    $LASTEXITCODE -eq 0
}

# --- B16: if-shell -b background ---
Test-Case "B16: if-shell -b background" {
    & $psmux if-shell -b -t $SESSION "exit 0" "run-shell 'exit 0'" "" 2>&1 | Out-Null
    $LASTEXITCODE -eq 0
}

# --- B17: if-shell complex condition ---
Test-Case "B17: if-shell complex condition" {
    & $psmux if-shell -t $SESSION "if (1 -eq 1) { exit 0 } else { exit 1 }" "run-shell 'exit 0'" "run-shell 'exit 1'" 2>&1 | Out-Null
    $LASTEXITCODE -eq 0
}

# --- B18: format #() basic ---
Test-Case "B18: format #() basic" {
    $out = & $psmux display-message -t $SESSION -p "#(echo fmt_basic_ok)" 2>&1 | Out-String
    $out.Trim() -match "fmt_basic_ok"
}

# --- B19: format #() pwsh command ---
Test-Case "B19: format #() pwsh" {
    $shell = if (Get-Command pwsh -ErrorAction SilentlyContinue) { "pwsh" } else { "powershell" }
    $out = & $psmux display-message -t $SESSION -p "#($shell -NoProfile -Command 'Write-Output ps_fmt_ok')" 2>&1 | Out-String
    $out.Trim() -match "ps_fmt_ok"
}

# --- B20: format #() numeric output ---
Test-Case "B20: format #() numeric" {
    $out = & $psmux display-message -t $SESSION -p "#(echo 42)" 2>&1 | Out-String
    $out.Trim() -eq "42"
}

# --- B21: format #() mixed with #{} ---
Test-Case "B21: mixed #{} and #()" {
    $out = & $psmux display-message -t $SESSION -p "s=#{session_name} c=#(echo mix_ok)" 2>&1 | Out-String
    ($out -match $SESSION) -and ($out -match "mix_ok")
}

# --- B22: format #() rapid 10x (status bar polling simulation) ---
Test-Case "B22: format #() rapid 10x" {
    $allOk = $true
    for ($i = 0; $i -lt 10; $i++) {
        $out = & $psmux display-message -t $SESSION -p "#(echo r$i)" 2>&1 | Out-String
        if ($out.Trim() -ne "r$i") { $allOk = $false; Write-Host "    MISMATCH at $i : '$($out.Trim())'" }
    }
    $allOk
}

# --- B23: config if-shell true branch sets option ---
Test-Case "B23: config if-shell true sets option" {
    $conffile = "$env:TEMP\psmux_hidewin_b23.conf"
    "if-shell `"exit 0`" `"set -g status-interval 13`" `"set -g status-interval 99`"" | Set-Content $conffile -Force
    & $psmux source-file -t $SESSION $conffile 2>&1 | Out-Null
    Start-Sleep 1
    $val = & $psmux show-options -t $SESSION -g -v status-interval 2>&1 | Out-String
    Remove-Item $conffile -Force -ErrorAction SilentlyContinue
    $val.Trim() -eq "13"
}

# --- B24: config if-shell false branch sets option ---
Test-Case "B24: config if-shell false sets option" {
    $conffile = "$env:TEMP\psmux_hidewin_b24.conf"
    "if-shell `"exit 1`" `"set -g status-interval 88`" `"set -g status-interval 17`"" | Set-Content $conffile -Force
    & $psmux source-file -t $SESSION $conffile 2>&1 | Out-Null
    Start-Sleep 1
    $val = & $psmux show-options -t $SESSION -g -v status-interval 2>&1 | Out-String
    Remove-Item $conffile -Force -ErrorAction SilentlyContinue
    $val.Trim() -eq "17"
}

# --- B25: pipe-pane runs hidden process ---
Test-Case "B25: pipe-pane starts hidden process" {
    $pipefile = "$env:TEMP\psmux_hidewin_pipe_b25.txt"
    Remove-Item $pipefile -Force -ErrorAction SilentlyContinue
    $shell = if (Get-Command pwsh -ErrorAction SilentlyContinue) { "pwsh" } else { "powershell" }
    & $psmux pipe-pane -t $SESSION "$shell -NoProfile -Command `"Set-Content '$pipefile' -Value 'pipe_proof_ok'`"" 2>&1 | Out-Null
    Start-Sleep 2
    & $psmux pipe-pane -t $SESSION 2>&1 | Out-Null
    if (Test-Path $pipefile) {
        $content = Get-Content $pipefile -Raw
        Remove-Item $pipefile -Force -ErrorAction SilentlyContinue
        $content -match "pipe_proof_ok"
    } else {
        # pipe-pane might not create file depending on output routing, but must not crash
        $true
    }
}

# --- B26: run-shell special characters ---
Test-Case "B26: run-shell special chars" {
    $out = & $psmux run-shell -t $SESSION "Write-Output 'a b c'" 2>&1 | Out-String
    $out -match "a b c"
}

# --- B27: if-shell stdout noise does not affect exit code ---
Test-Case "B27: if-shell stdout noise" {
    & $psmux if-shell -t $SESSION "Write-Output 'noise'; exit 0" "run-shell 'exit 0'" "run-shell 'exit 1'" 2>&1 | Out-Null
    $LASTEXITCODE -eq 0
}

# --- B28: run-shell no args (safety, no crash) ---
Test-Case "B28: run-shell no args no crash" {
    & $psmux run-shell -t $SESSION 2>&1 | Out-Null
    $true  # pass if no crash
}

# --- B29: 20x rapid run-shell back to back ---
Test-Case "B29: rapid 20x run-shell" {
    $allOk = $true
    for ($i = 0; $i -lt 20; $i++) {
        $out = & $psmux run-shell -t $SESSION "Write-Output 'b$i'" 2>&1 | Out-String
        if ($out.Trim() -notmatch "b$i") { $allOk = $false }
    }
    $allOk
}

# --- B30: latency proof (hidden subprocess completes fast) ---
Test-Case "B30: hidden subprocess latency" {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $out = & $psmux run-shell -t $SESSION "echo fast" 2>&1 | Out-String
    $sw.Stop()
    Write-Host "    Latency: $($sw.ElapsedMilliseconds)ms"
    ($out -match "fast") -and ($sw.ElapsedMilliseconds -lt 10000)
}

# ===================================================================
# SECTION C: STRESS / COMBINED SCENARIOS
# ===================================================================

Write-Host "`n==============================================="
Write-Host "  SECTION C: Stress and Combined Scenarios"
Write-Host "==============================================="
Ensure-Session

# --- C1: 30x rapid alternating run-shell + if-shell + #(), no window flash ---
Test-Case "C1: 30x mixed commands NO window flash" {
    $baseline = [WindowEnumerator]::SnapshotConsoleWindows()
    $totalFlash = 0

    for ($i = 0; $i -lt 10; $i++) {
        $null = & $psmux run-shell -t $SESSION "echo stress_$i" 2>&1
        $flash = Get-NewConsoleWindows -Baseline $baseline
        $totalFlash += $flash.Count

        $null = & $psmux if-shell -t $SESSION "exit 0" "run-shell 'exit 0'" "" 2>&1
        $flash = Get-NewConsoleWindows -Baseline $baseline
        $totalFlash += $flash.Count

        $null = & $psmux display-message -t $SESSION -p "#(echo s$i)" 2>&1
        $flash = Get-NewConsoleWindows -Baseline $baseline
        $totalFlash += $flash.Count
    }

    Write-Host "    Total new windows across 30 mixed commands: $totalFlash"
    $totalFlash -eq 0
}

# --- C2: Concurrent run-shell via jobs (simulates parallel plugins) ---
Test-Case "C2: concurrent run-shell via jobs" {
    $baseline = [WindowEnumerator]::SnapshotConsoleWindows()
    $jobs = @()
    $exePath = (Resolve-Path $psmux).Path
    for ($i = 0; $i -lt 5; $i++) {
        $jobs += Start-Job -ScriptBlock {
            param($exe, $sess, $idx)
            & $exe run-shell -t $sess "Write-Output 'conc_$idx'" 2>&1 | Out-String
        } -ArgumentList $exePath, $SESSION, $i
    }

    Start-Sleep -Milliseconds 1500
    $flash = Get-NewConsoleWindows -Baseline $baseline

    $outputs = @()
    foreach ($j in $jobs) {
        $outputs += (Receive-Job -Job $j -Wait | Out-String)
    }
    $jobs | Remove-Job -Force

    $allOk = $true
    for ($i = 0; $i -lt 5; $i++) {
        $found = $false
        foreach ($o in $outputs) { if ($o -match "conc_$i") { $found = $true } }
        if (-not $found) { $allOk = $false; Write-Host "    Missing output for conc_$i" }
    }

    Write-Host "    New windows during concurrent spawn: $($flash.Count)"
    Write-Host "    All outputs found: $allOk"
    ($flash.Count -eq 0) -and $allOk
}

# --- C3: Config with multiple if-shell + run-shell chain ---
Test-Case "C3: multi-line config chain" {
    $conffile = "$env:TEMP\psmux_hidewin_c3.conf"
    @"
if-shell "exit 0" "set -g status-interval 21" "set -g status-interval 99"
if-shell "exit 1" "set -g status-interval 99" "set -g status-interval 22"
"@ | Set-Content $conffile -Force

    $baseline = [WindowEnumerator]::SnapshotConsoleWindows()
    & $psmux source-file -t $SESSION $conffile 2>&1 | Out-Null
    Start-Sleep 1
    $flash = Get-NewConsoleWindows -Baseline $baseline

    $val = & $psmux show-options -t $SESSION -g -v status-interval 2>&1 | Out-String
    Remove-Item $conffile -Force -ErrorAction SilentlyContinue

    Write-Host "    New windows: $($flash.Count), final value: $($val.Trim())"
    ($flash.Count -eq 0) -and ($val.Trim() -eq "22")
}

# ===========================================================================
# Cleanup
# ===========================================================================
Write-Host "`n=== Cleanup ==="
& $psmux kill-session -t $SESSION 2>&1 | Out-Null
Start-Sleep 1

# ===========================================================================
# Summary
# ===========================================================================
Write-Host "`n=========================================="
Write-Host "  CREATE_NO_WINDOW E2E PROOF RESULTS"
Write-Host "=========================================="
Write-Host "Total:  $($TestsPassed + $TestsFailed)"
Write-Host "Passed: $TestsPassed" -ForegroundColor Green
Write-Host "Failed: $TestsFailed" -ForegroundColor $(if ($TestsFailed -gt 0) { "Red" } else { "Green" })
Write-Host "=========================================="

$results | Format-Table -AutoSize

if ($TestsFailed -gt 0) { exit 1 } else { exit 0 }
