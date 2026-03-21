#!/usr/bin/env pwsh
# Batch 3: ALL previously skipped tests (perf, stress, interactive, mouse, paste, plugins, theme, warm, etc.)
# Plus re-run of Git Bash dependent tests now that Git for Windows is installed
param([string]$OutFile = "$PSScriptRoot\..\target\test_results3.txt")

$testList = @(
    # --- Re-run Git Bash dependent tests ---
    "test_cross_shell_backslash",
    "test_issue99_default_shell_bash",

    # --- Multi-shell default-shell tests ---
    "test_default_shell_cmd",
    "test_default_shell_wsl",

    # --- WSL latency tests ---
    "test_wsl_latency",
    "test_wsl_in_pwsh_latency",
    "test_wsl_in_pwsh_latency2",
    "test_wsl_pwsh_latency3",
    "test_wsl_pwsh_latency4",
    "test_wsl_pwsh_latency5",

    # --- Perf/stress tests ---
    "test_perf",
    "test_perf_vs_wt",
    "test_startup_perf",
    "test_startup_exit_bench",
    "test_pane_startup_perf",
    "test_e2e_latency",
    "test_extreme_perf",
    "test_install_speed",
    "test_stress",
    "test_stress_50",
    "test_stress_aggressive",

    # --- Interactive/mouse tests ---
    "test_mouse_handling",
    "test_mouse_hover",
    "test_conpty_mouse",
    "test_claude_mouse",
    "test_claude_cursor_diag",
    "test_cursor_style",
    "test_cursor_fallback",
    "test_issue15_altgr",
    "test_issue52_cursor",
    "test_issue60_native_tui_mouse",
    "test_stress_attached",
    "test_tui_exit_cleanup",

    # --- Paste tests ---
    "test_cjk_paste_split",
    "test_issue74_paste",
    "test_issue91_ime_paste",
    "test_issue98_bracketed_paste",

    # --- Plugin/theme tests ---
    "test_plugins_themes",
    "test_real_plugins",
    "test_theme_rendering",

    # --- Other ---
    "test_warm_pane",
    "test_pty_stability",
    "test_issue50_chinese_chars"
)

$sb = [System.Text.StringBuilder]::new()
$totalP = 0; $totalF = 0; $totalS = 0; $totalTO = 0

foreach ($t in $testList) {
    taskkill /f /im psmux.exe 2>$null | Out-Null
    Start-Sleep 2
    Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
    Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue
    Remove-Item "$env:USERPROFILE\.psmux.conf" -Force -ErrorAction SilentlyContinue

    $testPath = "$PSScriptRoot\$t.ps1"
    if (-not (Test-Path $testPath)) {
        [void]$sb.AppendLine("SKIP  $t  (file not found)")
        $totalS++
        continue
    }

    Write-Host ">>> Starting $t ..." -ForegroundColor Cyan

    # Run with 120-second timeout per test to prevent hangs
    $job = Start-Job -ScriptBlock {
        param($path)
        & pwsh -NoProfile -ExecutionPolicy Bypass -File $path 2>&1 | Out-String
    } -ArgumentList $testPath

    $completed = $job | Wait-Job -Timeout 120
    if ($null -eq $completed) {
        # Timed out
        $job | Stop-Job
        $job | Remove-Job -Force
        $line = "TIMEOUT  $t  (exceeded 120s)"
        [void]$sb.AppendLine($line)
        Write-Host $line -ForegroundColor Yellow
        $totalTO++
        taskkill /f /im psmux.exe 2>$null | Out-Null
        continue
    }

    $out = $job | Receive-Job | Out-String
    $job | Remove-Job -Force

    $p = ([regex]::Matches($out, '(?i)\[PASS\]')).Count
    $f = ([regex]::Matches($out, '(?i)\[FAIL\]')).Count
    $totalP += $p; $totalF += $f
    $status = if ($f -eq 0) { "OK" } else { "FAIL" }
    $line = "$status  $t  (${p}P/${f}F)"
    [void]$sb.AppendLine($line)
    Write-Host $line -ForegroundColor $(if ($f -eq 0) {"Green"} else {"Red"})

    if ($f -gt 0) {
        $failLines = $out -split "`n" | Where-Object { $_ -match '(?i)\[FAIL\]' } | Select-Object -First 5
        foreach ($fl in $failLines) {
            $trimmed = "  >> $($fl.Trim())"
            [void]$sb.AppendLine($trimmed)
            Write-Host $trimmed -ForegroundColor Red
        }
    }
}

taskkill /f /im psmux.exe 2>$null | Out-Null

[void]$sb.AppendLine("")
[void]$sb.AppendLine("BATCH3 TOTAL: ${totalP}P / ${totalF}F / ${totalS}S / ${totalTO}TO")
Write-Host "`nBATCH3 TOTAL: ${totalP}P / ${totalF}F / ${totalS}S / ${totalTO} TIMEOUTS" -ForegroundColor $(if ($totalF -eq 0 -and $totalTO -eq 0) {"Green"} else {"Yellow"})

Set-Content -Path $OutFile -Value $sb.ToString() -Encoding UTF8
Write-Host "Results written to: $OutFile"
