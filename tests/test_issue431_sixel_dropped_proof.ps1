# Issue #431 - REPRODUCTION PROOF: sixel graphics dropped by psmux
#
# Reporter: `chafa -f sixel image.png` renders correctly in Windows Terminal
# directly, but shows NOTHING when run inside psmux (even with
# `set -g allow-passthrough all`).
#
# chafa -f sixel emits a raw sixel DCS sequence: ESC P <params> q <data> ESC \
# psmux parses the child PTY through its VT emulator and re-renders a character
# grid to the outer terminal. If the VT layer drops the sixel DCS, the image
# never reaches Windows Terminal -> blank, exactly as reported.
#
# GROUND TRUTH: run psmux under a REAL pseudoconsole (the same way Windows
# Terminal hosts it) and capture EVERY byte psmux writes to the outer terminal
# into conpty_out.bin. Emit a sixel with a DISTINCTIVE color marker "13;57;91"
# from inside a pane, then check whether those bytes survive to the outer PTY.
#
# A/B design mirrors the reporter's two screenshots:
#   A) BASELINE (no psmux): same sixel under the ConPTY host running pwsh
#      directly -> marker MUST appear (proves emitter + capture both work).
#   B) WITH psmux: same sixel emitted inside a psmux pane -> marker presence
#      decides the verdict.

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$SESSION = "issue431_sixel"
$script:Pass = 0
$script:Fail = 0

function Pass($m) { Write-Host "  [PASS] $m" -ForegroundColor Green; $script:Pass++ }
function FailX($m) { Write-Host "  [FAIL] $m" -ForegroundColor Red; $script:Fail++ }
function Info($m)  { Write-Host "  [INFO] $m" -ForegroundColor DarkCyan }

$MARKER = "13;57;91"   # distinctive sixel color; will not appear in normal psmux output

function Cleanup {
    & $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Milliseconds 400
    Remove-Item "$psmuxDir\$SESSION.*" -Force -EA SilentlyContinue
    Get-Process conpty_host -EA SilentlyContinue | ForEach-Object { try { Stop-Process -Id $_.Id -Force -EA SilentlyContinue } catch {} }
    Remove-Item "$env:TEMP\conpty_ctrl.txt","$env:TEMP\conpty_out.bin" -Force -EA SilentlyContinue
}

# --- Compile the ConPTY host (captures raw outer-terminal bytes) ---
$hostExe = "$env:TEMP\conpty_host.exe"
$csc = "C:\Windows\Microsoft.NET\Framework64\v4.0.30319\csc.exe"
& $csc /nologo /optimize /out:$hostExe "$PSScriptRoot\conpty_ctrlc_host.cs" 2>&1 | Out-Null
if (-not (Test-Path $hostExe)) { Write-Host "Cannot compile conpty host" -ForegroundColor Red; exit 2 }

# --- Build the sixel emitter script (writes raw sixel bytes to stdout) ---
# ESC P q "1;1;24;12  #7;2;13;57;91  #7 !24~ $ - !24~ $ - ESC \
$mid = [System.Text.Encoding]::ASCII.GetBytes('q"1;1;24;12#7;2;13;57;91#7!24~$-!24~$-')
$sixel = [byte[]]@(0x1B,0x50) + $mid + [byte[]]@(0x1B,0x5C)
$hex = ($sixel | ForEach-Object { '0x{0:X2}' -f $_ }) -join ','
$emitter = "$env:TEMP\sixel_emit.ps1"
@"
`$b = [byte[]]@($hex)
`$o = [Console]::OpenStandardOutput()
`$o.Write(`$b, 0, `$b.Length)
`$o.Flush()
Start-Sleep -Milliseconds 200
Write-Host "SIXEL_EMITTED_DONE"
"@ | Set-Content -Path $emitter -Encoding ASCII

function Read-OutBin {
    $bin = "$env:TEMP\conpty_out.bin"
    if (-not (Test-Path $bin)) { return $null }
    for ($i = 0; $i -lt 5; $i++) {
        try { return [System.IO.File]::ReadAllBytes($bin) } catch { Start-Sleep -Milliseconds 200 }
    }
    return $null
}
function Contains-Marker($bytes, $needle) {
    if ($null -eq $bytes) { return $false }
    $n = [System.Text.Encoding]::ASCII.GetBytes($needle)
    for ($i = 0; $i -le $bytes.Length - $n.Length; $i++) {
        $ok = $true
        for ($j = 0; $j -lt $n.Length; $j++) { if ($bytes[$i+$j] -ne $n[$j]) { $ok = $false; break } }
        if ($ok) { return $true }
    }
    return $false
}
function Contains-DcsSixelIntro($bytes) {
    # ESC P ... q  (0x1B 0x50 then a 'q' before the next ST)
    if ($null -eq $bytes) { return $false }
    for ($i = 0; $i -lt $bytes.Length - 1; $i++) {
        if ($bytes[$i] -eq 0x1B -and $bytes[$i+1] -eq 0x50) {
            for ($k = $i+2; $k -lt [Math]::Min($bytes.Length, $i+40); $k++) {
                if ($bytes[$k] -eq 0x71) { return $true }       # 'q' sixel introducer
                if ($bytes[$k] -eq 0x1B) { break }              # hit ST first
            }
        }
    }
    return $false
}

Cleanup
Write-Host "`n=== Issue #431 SIXEL DROP REPRODUCTION ===" -ForegroundColor Cyan
Info "Sixel bytes: $($sixel.Length) total, marker color '$MARKER'"

# =============================================================================
# TEST A (BASELINE): sixel under ConPTY host running pwsh directly (NO psmux)
# =============================================================================
Write-Host "`n[A] BASELINE: sixel emitted with NO psmux (proves emitter+capture)" -ForegroundColor Yellow
Remove-Item "$env:TEMP\conpty_ctrl.txt","$env:TEMP\conpty_out.bin" -Force -EA SilentlyContinue
$procA = Start-Process -FilePath $hostExe -ArgumentList "pwsh","-NoLogo","-NoProfile" -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 3
Add-Content "$env:TEMP\conpty_ctrl.txt" "TEXT . '$emitter'`n"
Start-Sleep -Seconds 3
Add-Content "$env:TEMP\conpty_ctrl.txt" "QUIT`n"
Start-Sleep -Seconds 1
try { Stop-Process -Id $procA.Id -Force -EA SilentlyContinue } catch {}

$binA = Read-OutBin
$aMarker = Contains-Marker $binA $MARKER
$aDcs = Contains-DcsSixelIntro $binA
Info "baseline output = $($binA.Length) bytes; marker=$aMarker dcs_q=$aDcs"
if ($aMarker -and $aDcs) {
    Pass "BASELINE: sixel marker '$MARKER' AND DCS 'q' present in raw output (emitter + capture verified)"
} else {
    FailX "BASELINE: sixel not captured (marker=$aMarker dcs=$aDcs) - harness broken, cannot trust result"
}

# =============================================================================
# TEST B (WITH PSMUX): same sixel emitted inside a psmux pane
# =============================================================================
Write-Host "`n[B] WITH PSMUX: same sixel emitted inside a psmux pane" -ForegroundColor Yellow
Remove-Item "$env:TEMP\conpty_ctrl.txt","$env:TEMP\conpty_out.bin" -Force -EA SilentlyContinue
$procB = Start-Process -FilePath $hostExe -ArgumentList $PSMUX,"new-session","-s",$SESSION -PassThru -WindowStyle Hidden
Start-Sleep -Seconds 6   # let server + pane shell come up

& $PSMUX has-session -t $SESSION 2>$null
if ($LASTEXITCODE -ne 0) {
    FailX "psmux session did not start under ConPTY host"
} else {
    Pass "psmux session started under ConPTY host"
    # Also set allow-passthrough all, exactly as the reporter did
    & $PSMUX set-option -g allow-passthrough all 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    Add-Content "$env:TEMP\conpty_ctrl.txt" "TEXT . '$emitter'`n"
    Start-Sleep -Seconds 4
    # confirm the emitter actually ran in the pane
    $cap = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
    if ($cap -match "SIXEL_EMITTED_DONE") { Info "emitter ran inside pane (marker line seen in capture-pane)" }
    else { Info "emitter completion marker not seen in capture-pane (pane may have cleared)" }
    Add-Content "$env:TEMP\conpty_ctrl.txt" "QUIT`n"
    Start-Sleep -Seconds 1
}
& $PSMUX kill-session -t $SESSION 2>&1 | Out-Null
try { Stop-Process -Id $procB.Id -Force -EA SilentlyContinue } catch {}

$binB = Read-OutBin
$bMarker = Contains-Marker $binB $MARKER
$bDcs = Contains-DcsSixelIntro $binB
Info "psmux output = $($binB.Length) bytes; marker=$bMarker dcs_q=$bDcs"

if (-not $bMarker -and -not $bDcs) {
    Pass "BUG REPRODUCED: sixel marker '$MARKER' ABSENT from psmux outer output - image dropped (matches report)"
} elseif ($bMarker -and $bDcs) {
    FailX "sixel PASSED THROUGH psmux (marker+DCS present) - cannot reproduce the drop"
} else {
    Info "PARTIAL: marker=$bMarker dcs=$bDcs - inspect conpty_out.bin"
    FailX "Ambiguous sixel handling"
}

Cleanup
Remove-Item $emitter -Force -EA SilentlyContinue

Write-Host "`n=== Result ===" -ForegroundColor Cyan
Write-Host "  Passed: $($script:Pass)" -ForegroundColor Green
Write-Host "  Failed: $($script:Fail)" -ForegroundColor $(if ($script:Fail -gt 0) { "Red" } else { "Green" })
exit $script:Fail
