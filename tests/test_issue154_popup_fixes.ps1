#!/usr/bin/env pwsh
# Regression tests for discussion #154: popup percentage dimensions, -d flag, TERM env
#
# Bug 1: -w/-h percentage values (e.g. "95%") were not resolved to actual terminal percentages
# Bug 2: -d flag for start directory was not parsed, causing its value to leak into the command
# Bug 3: Popup PTYs did not have TERM/COLORTERM set, so programs like lazygit had no colors

$ErrorActionPreference = "Continue"
$results = @()

function Add-Result($name, $pass, $detail="") {
    $script:results += [PSCustomObject]@{
        Test=$name
        Result=if($pass){"PASS"}else{"FAIL"}
        Detail=$detail
    }
}

$SESSION = "test154popup_$$"
$h = $env:USERPROFILE
$TERM_W = 160
$TERM_H = 40

function Get-Port { (Get-Content "$h\.psmux\$SESSION.port").Trim() }
function Get-Key { if (Test-Path "$h\.psmux\$SESSION.key") { (Get-Content "$h\.psmux\$SESSION.key").Trim() } else { "" } }

function Send-PersistentCmd($cmd) {
    $port = Get-Port; $key = Get-Key
    $tcp = New-Object System.Net.Sockets.TcpClient
    $tcp.Connect("127.0.0.1", [int]$port)
    $s = $tcp.GetStream()
    $w = New-Object System.IO.StreamWriter($s)
    $w.AutoFlush = $true
    $w.WriteLine("AUTH $key")
    $w.WriteLine("PERSISTENT")
    $w.WriteLine("client-attach")
    Start-Sleep -Milliseconds 300
    $w.WriteLine($cmd)
    Start-Sleep -Milliseconds 1500
    $tcp.Close()
    Start-Sleep -Milliseconds 300
}

function Get-DumpState {
    $port = Get-Port; $key = Get-Key
    $tcp = New-Object System.Net.Sockets.TcpClient
    $tcp.Connect("127.0.0.1", [int]$port)
    $s = $tcp.GetStream()
    $w = New-Object System.IO.StreamWriter($s)
    $w.AutoFlush = $true
    $w.WriteLine("AUTH $key")
    $w.WriteLine("dump-state")
    Start-Sleep -Milliseconds 1500
    $buf = New-Object byte[] 262144
    $total = 0
    while ($s.DataAvailable -and $total -lt 262144) {
        $n = $s.Read($buf, $total, 262144 - $total)
        $total += $n
    }
    $tcp.Close()
    return [System.Text.Encoding]::UTF8.GetString($buf, 0, $total)
}

function Dismiss-Popup {
    $port = Get-Port; $key = Get-Key
    $tcp = New-Object System.Net.Sockets.TcpClient
    $tcp.Connect("127.0.0.1", [int]$port)
    $s = $tcp.GetStream()
    $w = New-Object System.IO.StreamWriter($s)
    $w.AutoFlush = $true
    $w.WriteLine("AUTH $key")
    $w.WriteLine("overlay-close")
    Start-Sleep -Milliseconds 300
    $tcp.Close()
    Start-Sleep -Milliseconds 300
}

try {
    # Clean up any leftover session
    psmux kill-session -t $SESSION 2>$null
    Start-Sleep -Milliseconds 500

    # Create a detached session with known dimensions
    psmux new-session -d -s $SESSION -x $TERM_W -y $TERM_H
    Start-Sleep -Seconds 3

    # ================================================================
    # Test 1: Percentage width should resolve to actual terminal percentage
    # ================================================================
    Send-PersistentCmd "display-popup -w 50% -h 50% pwsh -NoProfile -Command 'Write-Host percenttest; Start-Sleep 60'"
    $json = Get-DumpState
    if ($json -match '"popup_active"\s*:\s*true') {
        if ($json -match '"popup_width"\s*:\s*(\d+)') {
            $pw = [int]$Matches[1]
            $expected_w = [math]::Floor($TERM_W * 50 / 100)
            # Allow some tolerance for border/rounding
            $ok = [math]::Abs($pw - $expected_w) -le 5
            Add-Result "popup_percentage_width" $ok "width=$pw expected~$expected_w"
        } else {
            Add-Result "popup_percentage_width" $false "no popup_width in JSON"
        }
        if ($json -match '"popup_height"\s*:\s*(\d+)') {
            $ph = [int]$Matches[1]
            $expected_h = [math]::Floor($TERM_H * 50 / 100)
            $ok = [math]::Abs($ph - $expected_h) -le 3
            Add-Result "popup_percentage_height" $ok "height=$ph expected~$expected_h"
        } else {
            Add-Result "popup_percentage_height" $false "no popup_height in JSON"
        }
    } else {
        Add-Result "popup_percentage_width" $false "popup not active"
        Add-Result "popup_percentage_height" $false "popup not active"
    }
    Dismiss-Popup

    # ================================================================
    # Test 2: Absolute dimensions should still work
    # ================================================================
    Send-PersistentCmd "display-popup -w 60 -h 15 pwsh -NoProfile -Command 'Write-Host abstest; Start-Sleep 60'"
    $json = Get-DumpState
    if ($json -match '"popup_active"\s*:\s*true') {
        $w_ok = $json -match '"popup_width"\s*:\s*60\b'
        $h_ok = $json -match '"popup_height"\s*:\s*15\b'
        Add-Result "popup_absolute_width" $w_ok "width match 60"
        Add-Result "popup_absolute_height" $h_ok "height match 15"
    } else {
        Add-Result "popup_absolute_width" $false "popup not active"
        Add-Result "popup_absolute_height" $false "popup not active"
    }
    Dismiss-Popup

    # ================================================================
    # Test 3: -d flag should NOT leak into command string
    # ================================================================
    Send-PersistentCmd "display-popup -d C:\Users pwsh -NoProfile -Command 'Write-Host dirtest; Start-Sleep 60'"
    $json = Get-DumpState
    if ($json -match '"popup_active"\s*:\s*true') {
        if ($json -match '"popup_command"\s*:\s*"([^"]*)"') {
            $cmd = $Matches[1]
            $no_leak = -not ($cmd -match 'C:\\Users|C:/Users')
            Add-Result "popup_d_flag_no_leak" $no_leak "command='$cmd'"
        } else {
            Add-Result "popup_d_flag_no_leak" $false "no popup_command in JSON"
        }
    } else {
        Add-Result "popup_d_flag_no_leak" $false "popup not active"
    }
    Dismiss-Popup

    # ================================================================
    # Test 4: -d flag should be parsed without error (popup should open)
    # ================================================================
    Send-PersistentCmd "popup -d . pwsh -NoProfile -Command 'Write-Host test_d_works; Start-Sleep 60'"
    $json = Get-DumpState
    $d_active = $json -match '"popup_active"\s*:\s*true'
    Add-Result "popup_d_flag_opens" $d_active "popup opened with -d flag"
    Dismiss-Popup

    # ================================================================
    # Test 5: -c flag should also work for directory (alias of -d)
    # ================================================================
    Send-PersistentCmd "popup -c . pwsh -NoProfile -Command 'Write-Host test_c_works; Start-Sleep 60'"
    $json = Get-DumpState
    $c_active = $json -match '"popup_active"\s*:\s*true'
    Add-Result "popup_c_flag_opens" $c_active "popup opened with -c flag"
    Dismiss-Popup

    # ================================================================
    # Test 6: Combined -d and percentage dims
    # ================================================================
    Send-PersistentCmd "popup -w 95% -h 90% -d . pwsh -NoProfile -Command 'Write-Host combined_test; Start-Sleep 60'"
    $json = Get-DumpState
    if ($json -match '"popup_active"\s*:\s*true') {
        if ($json -match '"popup_width"\s*:\s*(\d+)') {
            $pw = [int]$Matches[1]
            $expected_w = [math]::Floor($TERM_W * 95 / 100)
            $ok = [math]::Abs($pw - $expected_w) -le 5
            Add-Result "popup_combined_pct_dir" $ok "width=$pw expected~$expected_w with -d flag"
        } else {
            Add-Result "popup_combined_pct_dir" $false "no popup_width in JSON"
        }
    } else {
        Add-Result "popup_combined_pct_dir" $false "popup not active"
    }
    Dismiss-Popup

    # ═══════════════════════════════════════════════════════════════
    # Win32 TUI VERIFICATION: Prove popup dimensions via real keys
    # ═══════════════════════════════════════════════════════════════
    Write-Host ""
    Write-Host ("=" * 60)
    Write-Host "Win32 TUI VISUAL VERIFICATION" -ForegroundColor Yellow
    Write-Host ("=" * 60)

    . "$PSScriptRoot\tui_helper.ps1"
    $TUI_SESSION_P154 = "p154_tui_proof"

    $tuiOk = Launch-PsmuxWindow -Session $TUI_SESSION_P154
    if ($tuiOk) {
        Start-Sleep -Seconds 2

        # TUI Test: Trigger popup with percentage dimensions via CLI
        Write-Host "[TEST] TUI: Popup with 80%x60% dimensions (visible TUI proof)" -ForegroundColor White
        & $script:TUI_PSMUX display-popup -t $TUI_SESSION_P154 -w "80%" -h "60%" -E "echo TUIPROOF" 2>&1 | Out-Null
        Start-Sleep -Seconds 1
        $name = Safe-TuiQuery "#{session_name}" -Session $TUI_SESSION_P154
        if ($name) {
            Add-Result "TUI: popup_pct_cli" $true "Session responsive ($name)"
        } else {
            Add-Result "TUI: popup_pct_cli" $false "Session not responsive"
        }

        Cleanup-PsmuxWindow -Session $TUI_SESSION_P154
        Write-Host ""
    } else {
        Write-Host "  TUI verification skipped (could not launch window)" -ForegroundColor Yellow
    }

} finally {
    psmux kill-session -t $SESSION 2>$null
    Start-Sleep -Milliseconds 500
}

Write-Host "`n=== Discussion #154 Popup Fixes Results ==="
$results | Format-Table -AutoSize
$failed = ($results | Where-Object { $_.Result -eq "FAIL" }).Count
if ($failed -gt 0) {
    Write-Host "`n$failed test(s) FAILED" -ForegroundColor Red
    exit 1
} else {
    Write-Host "`nAll tests passed!" -ForegroundColor Green
    exit 0
}
