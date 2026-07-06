# Issue #431 - FIX PROOF (server-layer, machine-verifiable)
#
# BEFORE the fix: a sixel DCS emitted into a psmux pane was silently swallowed
# by the vt100 layer (proven earlier: BEFORE_MARK/AFTER_MARK survived, the sixel
# vanished with no trace in capture-pane OR any server state).
#
# AFTER the fix: the server parses the sixel into a SixelImage, stores it anchored
# in the pane, and transports it to the client via dump-state:
#   - a per-leaf "images":[{id,row,col,pw,ph,cw,ch}] descriptor, AND
#   - a one-shot "image_blobs":{"<id>":"<base64 of the raw ESC P..ST bytes>"}.
# This script PROVES that end-to-end at the server layer over raw TCP, and proves
# the pane stays functional. (Final rasterized pixels are an interactive WT step;
# see the checklist printed at the end.)

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$S = "issue431_fix"
$MARKER = "13;57;91"   # distinctive sixel color embedded in the raw bytes
$script:Pass = 0; $script:Fail = 0
function Pass($m){Write-Host "  [PASS] $m" -ForegroundColor Green; $script:Pass++}
function FailX($m){Write-Host "  [FAIL] $m" -ForegroundColor Red; $script:Fail++}
function Info($m){Write-Host "  [INFO] $m" -ForegroundColor DarkCyan}

function Cleanup {
    & $PSMUX kill-session -t $S 2>&1 | Out-Null
    Start-Sleep -Milliseconds 400
    Remove-Item "$psmuxDir\$S.*" -Force -EA SilentlyContinue
}

function Get-DumpState {
    param([string]$Session)
    $port = (Get-Content "$psmuxDir\$Session.port" -Raw).Trim()
    $key  = (Get-Content "$psmuxDir\$Session.key" -Raw).Trim()
    $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
    $tcp.NoDelay = $true
    $stream = $tcp.GetStream()
    $w = [System.IO.StreamWriter]::new($stream); $r = [System.IO.StreamReader]::new($stream)
    $w.Write("AUTH $key`n"); $w.Flush(); $null = $r.ReadLine()
    $w.Write("dump-state`n"); $w.Flush()
    $stream.ReadTimeout = 5000
    $sb = [System.Text.StringBuilder]::new()
    try {
        while ($true) {
            $line = $r.ReadLine()
            if ($null -eq $line) { break }
            [void]$sb.AppendLine($line)
            if ($line.EndsWith('}') -and $sb.Length -gt 100) { break }
        }
    } catch {}
    $tcp.Close()
    return $sb.ToString().Trim()
}

Cleanup
Write-Host "`n=== Issue #431 FIX PROOF (server carries sixel) ===" -ForegroundColor Cyan

# Build the sixel emitter: BEFORE_MARK + raw sixel (color 13;57;91) + AFTER_MARK
$mid = [System.Text.Encoding]::ASCII.GetBytes('q"1;1;24;12#7;2;13;57;91#7!24~$-!24~$-')
$sixel = [byte[]]@(0x1B,0x50) + $mid + [byte[]]@(0x1B,0x5C)
$hex = ($sixel | ForEach-Object { '0x{0:X2}' -f $_ }) -join ','
$emitter = "$env:TEMP\issue431_fix_emit.ps1"
@"
Write-Host "BEFORE_MARK"
`$b = [byte[]]@($hex)
`$o = [Console]::OpenStandardOutput()
`$o.Write(`$b, 0, `$b.Length); `$o.Flush()
Start-Sleep -Milliseconds 150
Write-Host "AFTER_MARK"
"@ | Set-Content -Path $emitter -Encoding ASCII

& $PSMUX new-session -d -s $S 2>&1 | Out-Null
Start-Sleep -Seconds 3
& $PSMUX has-session -t $S 2>$null
if ($LASTEXITCODE -ne 0) { FailX "session did not start"; Cleanup; exit 1 }
Pass "session started"

& $PSMUX send-keys -t $S 'cls' Enter 2>&1 | Out-Null
Start-Sleep -Milliseconds 600
& $PSMUX send-keys -t $S (". '" + $emitter + "'") Enter 2>&1 | Out-Null
Start-Sleep -Seconds 2

# 1) Pane stays functional: BEFORE/AFTER markers survive
$cap = & $PSMUX capture-pane -t $S -p 2>&1 | Out-String
if ($cap -match "BEFORE_MARK" -and $cap -match "AFTER_MARK") { Pass "pane functional: BEFORE_MARK and AFTER_MARK both survive the sixel" }
else { FailX "pane text markers missing (BEFORE=$($cap -match 'BEFORE_MARK') AFTER=$($cap -match 'AFTER_MARK'))" }

# The sixel raw bytes must NOT leak as text into the grid
if ($cap -notmatch [regex]::Escape($MARKER) -and $cap -notmatch '!24~') { Pass "sixel bytes did NOT leak into the text grid (clean DCS consumption)" }
else { FailX "sixel bytes leaked into text grid" }

# 2) Server carries the image: dump-state has images descriptor + image_blobs
$dump = Get-DumpState -Session $S
Info "dump-state length = $($dump.Length) chars"

# images descriptor present and non-empty
$hasImagesArr = $dump -match '"images"\s*:\s*\[\s*\{'
if ($hasImagesArr) { Pass "dump-state leaf carries a NON-EMPTY images descriptor array" }
else {
    # show whether the field exists at all
    if ($dump -match '"images"\s*:\s*\[') { FailX "images array present but EMPTY - image not captured/visible" }
    else { FailX "images field absent from dump-state" }
}

# descriptor fields
if ($dump -match '"images"\s*:\s*\[\s*\{[^}]*"id"\s*:\s*(\d+)[^}]*"cw"\s*:\s*(\d+)[^}]*"ch"\s*:\s*(\d+)') {
    $imgId = $matches[1]; $cw = $matches[2]; $ch = $matches[3]
    Pass "descriptor parsed: id=$imgId cw=$cw ch=$ch (cell footprint captured)"
} else { $imgId = $null; Info "could not parse descriptor id/cw/ch" }

# image_blobs present, contains base64 for the id, and decodes to the raw sixel with our marker
if ($dump -match '"image_blobs"\s*:\s*\{\s*"(\d+)"\s*:\s*"([A-Za-z0-9+/=]+)"') {
    $blobId = $matches[1]; $b64 = $matches[2]
    Pass "image_blobs carries base64 for id=$blobId ($($b64.Length) b64 chars)"
    try {
        $raw = [System.Convert]::FromBase64String($b64)
        $rawAscii = [System.Text.Encoding]::ASCII.GetString($raw)
        $isDcs = ($raw.Length -ge 2 -and $raw[0] -eq 0x1B -and $raw[1] -eq 0x50)
        $endsSt = ($raw.Length -ge 2 -and $raw[$raw.Length-2] -eq 0x1B -and $raw[$raw.Length-1] -eq 0x5C)
        if ($isDcs -and $endsSt) { Pass "decoded blob is a well-formed DCS (starts ESC P, ends ST), $($raw.Length) bytes" }
        else { FailX "decoded blob not a well-formed DCS (startDCS=$isDcs endST=$endsSt)" }
        if ($rawAscii -match [regex]::Escape($MARKER)) { Pass "decoded blob contains the EXACT sixel color marker '$MARKER' - it is our image, faithfully preserved" }
        else { FailX "decoded blob missing marker '$MARKER'" }
        if ($blobId -eq $imgId) { Pass "blob id matches leaf descriptor id ($blobId) - transport is coherent" }
    } catch { FailX "base64 decode failed: $_" }
} else { FailX "image_blobs missing base64 payload in first dump after sixel" }

# 3) one-shot: a SECOND immediate dump should have empty image_blobs (already shipped)
$dump2 = Get-DumpState -Session $S
if ($dump2 -match '"image_blobs"\s*:\s*\{\s*\}') { Pass "second dump has EMPTY image_blobs (blob shipped once, NC-dedup preserved)" }
elseif ($dump2 -match '"image_blobs"\s*:\s*\{\s*"') { Info "second dump re-shipped blob (acceptable if a redraw/attach reset occurred)" }
else { Info "second dump image_blobs state indeterminate" }

# 4) option gating: set -g sixel off is accepted and reflected
& $PSMUX set-option -g sixel off 2>&1 | Out-Null
Start-Sleep -Milliseconds 400
$so = (& $PSMUX show-options -g 2>&1 | Out-String)
if ($so -match 'sixel\s+off') { Pass "set -g sixel off reflected in show-options" }
else { FailX "sixel option not reflected off (show-options)" }
& $PSMUX set-option -g sixel on 2>&1 | Out-Null

Cleanup
Remove-Item $emitter -Force -EA SilentlyContinue

Write-Host "`n=== Result ===" -ForegroundColor Cyan
Write-Host "  Passed: $($script:Pass)" -ForegroundColor Green
Write-Host "  Failed: $($script:Fail)" -ForegroundColor $(if ($script:Fail -gt 0){"Red"}else{"Green"})
Write-Host "`n  INTERACTIVE WT CHECKLIST (pixels - user confirms in Windows Terminal):" -ForegroundColor Yellow
Write-Host "   1. Run psmux in Windows Terminal, then: chafa -f sixel <image.png>" -ForegroundColor DarkYellow
Write-Host "   2. The image should now RENDER inside the pane (was blank before)." -ForegroundColor DarkYellow
Write-Host "   3. It should stay clipped to the pane and repaint after redraw/scroll." -ForegroundColor DarkYellow
Write-Host "   4. 'set -g sixel off' then re-run chafa => image suppressed." -ForegroundColor DarkYellow
exit $script:Fail
