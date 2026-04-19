#!/usr/bin/env pwsh
# Test for issue #146 (real fix): List commands from the command prompt
# MUST show a popup overlay via the server's PopupMode mechanism.
#
# The original "fix" only added handlers in execute_command_prompt(), which
# runs on the SERVER side for embedded/non-server mode. But in the normal
# server+client architecture, the CLIENT sends raw commands via TCP and the
# server wrote text output back to the TCP stream. The client's reader
# thread only understands JSON (dump-state) frames, so the text output was
# silently discarded as a JSON parse error.
#
# The real fix adds CtrlReq::ShowTextPopup so that in persistent mode
# (attached client), list commands route through PopupMode on the server,
# which the client picks up via the dump-state JSON (popup_active, etc.).
#
# This test verifies the fix by sending commands through the persistent TCP
# path (exactly what the real client does) and checking dump-state for
# popup_active=true.

$ErrorActionPreference = "Continue"
$results = @()

function Add-Result($name, $pass, $detail="") {
    $script:results += [PSCustomObject]@{
        Test=$name
        Result=if($pass){"PASS"}else{"FAIL"}
        Detail=$detail
    }
}

$SESSION = "test146popup_$$"
$h = $env:USERPROFILE

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
    Start-Sleep -Milliseconds 1000
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

    # Create a detached session
    psmux new-session -d -s $SESSION -x 120 -y 30
    Start-Sleep -Seconds 3

    # ---- Test each list command via persistent TCP (simulates command prompt) ----
    foreach ($cmd in @("list-windows", "list-panes", "list-clients", "list-commands", "show-hooks")) {
        Send-PersistentCmd $cmd
        $json = Get-DumpState

        $popupActive = $json -match '"popup_active"\s*:\s*true'
        $popupCmd = if ($json -match '"popup_command"\s*:\s*"([^"]*)"') { $Matches[1] } else { "(none)" }

        Add-Result "$cmd (popup shown)" $popupActive "popup_command=$popupCmd"

        # Dismiss and verify
        Dismiss-Popup
        $json2 = Get-DumpState
        $dismissed = -not ($json2 -match '"popup_active"\s*:\s*true')
        Add-Result "$cmd (popup dismissed)" $dismissed ""
    }

    # ---- External CLI should still work (not broken by fix) ----
    $lsw = psmux list-windows -t $SESSION 2>&1 | Out-String
    $pass = $lsw -match "\d+:" -or $lsw.Trim().Length -gt 5
    Add-Result "list-windows (external CLI, still works)" $pass "Output: $($lsw.Trim().Substring(0, [Math]::Min(80, $lsw.Trim().Length)))"

    $lsp = psmux list-panes -t $SESSION 2>&1 | Out-String
    $pass = $lsp -match "\d+:" -or $lsp.Trim().Length -gt 5
    Add-Result "list-panes (external CLI, still works)" $pass "Output: $($lsp.Trim().Substring(0, [Math]::Min(80, $lsp.Trim().Length)))"

    # ---- dump-state should NOT have popup after external CLI call ----
    $dump = Get-DumpState
    $noPopup = -not ($dump -match '"popup_active"\s*:\s*true')
    Add-Result "No popup after external CLI list" $noPopup "External CLI should not trigger popup"

} finally {
    # Cleanup
    psmux kill-session -t $SESSION 2>$null
    Start-Sleep -Milliseconds 500
}

# Summary
Write-Host ""
Write-Host "=== Issue #146: Popup via Command Prompt Test Results ==="
$results | Format-Table -AutoSize
$fail = ($results | Where-Object { $_.Result -eq "FAIL" }).Count
$total = $results.Count
Write-Host "Result: $($total - $fail)/$total passed"
if ($fail -gt 0) { exit 1 }
