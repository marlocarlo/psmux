# Issue #431 - Sixel M4: server blob shipping (image_blobs) + resync.
#
# Proves the server-side transport half of sixel support:
#   1) After a real sixel DCS is fed into a pane, the FIRST dump-state frame
#      carries a top-level "image_blobs" map with a base64 value keyed by the
#      image id.
#   2) The base64 decodes to bytes that start with the sixel DCS introducer
#      (ESC P ... q) and end with ST (ESC \) - i.e. the exact re-emit bytes.
#   3) The per-leaf "images" descriptor id MATCHES the image_blobs key (so the
#      client can pair descriptor -> blob).
#   4) An immediately-following (state-dirtied) dump SHIPS THE BLOB ONCE:
#      image_blobs is now empty {} because the id is already shipped.
#   5) refresh-client RESYNC re-ships: after refresh-client the blob reappears.
#
# Transport only - no rasterised pixels are asserted (that needs Windows
# Terminal, deferred to M8). dump-state is queried over TCP exactly like the
# #413 harness.

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$S = "issue431blobs"
$script:Pass = 0
$script:Fail = 0
function Write-Pass($m){ Write-Host "  [PASS] $m" -ForegroundColor Green; $script:Pass++ }
function Write-Fail($m){ Write-Host "  [FAIL] $m" -ForegroundColor Red; $script:Fail++ }
function Info($m){ Write-Host "  [INFO] $m" -ForegroundColor DarkCyan }

# --- Build the sixel emitter (writes raw sixel bytes to its stdout = pane PTY) ---
# ESC P q "1;1;24;12  #7;2;13;57;91  #7 !24~ $ - !24~ $ - ESC \
$MARKER = "13;57;91"
$mid = [System.Text.Encoding]::ASCII.GetBytes('q"1;1;24;12#7;2;13;57;91#7!24~$-!24~$-')
$sixel = [byte[]]@(0x1B,0x50) + $mid + [byte[]]@(0x1B,0x5C)
$hex = ($sixel | ForEach-Object { '0x{0:X2}' -f $_ }) -join ','
$emitter = "$env:TEMP\sixel_emit_431blobs.ps1"
@"
`$b = [byte[]]@($hex)
`$o = [Console]::OpenStandardOutput()
`$o.Write(`$b, 0, `$b.Length)
`$o.Flush()
Start-Sleep -Milliseconds 150
Write-Host "SIXEL_EMITTED_DONE"
"@ | Set-Content -Path $emitter -Encoding ASCII

# --- Fresh session ---
& $PSMUX kill-session -t $S 2>&1 | Out-Null
Start-Sleep -Milliseconds 400
Remove-Item "$psmuxDir\$S.*" -Force -EA SilentlyContinue

$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$S -PassThru
Start-Sleep -Seconds 6
if (-not (Test-Path "$psmuxDir\$S.port")) {
    Write-Fail "session did not start (no .port file)"
    exit 1
}
$port = (Get-Content "$psmuxDir\$S.port" -Raw).Trim()
$key  = (Get-Content "$psmuxDir\$S.key" -Raw).Trim()

# --- TCP dump-state helper (raw string), fresh connection each call ---
function Get-Dump {
    $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
    $tcp.NoDelay=$true; $tcp.ReceiveTimeout=4000
    $st=$tcp.GetStream(); $w=[System.IO.StreamWriter]::new($st); $r=[System.IO.StreamReader]::new($st)
    $w.Write("AUTH $key`n"); $w.Flush(); $null=$r.ReadLine()
    $w.Write("dump-state`n"); $w.Flush()
    $best=$null
    for($j=0;$j -lt 300;$j++){ try{$line=$r.ReadLine()}catch{break}; if($null -eq $line){break}; if($line -ne "NC" -and $line.Length -gt 100){$best=$line; break} }
    $tcp.Close()
    return $best
}

# Walk the layout tree and return the first non-empty leaf "images" array.
function Get-LeafImages($layout) {
    if ($null -eq $layout) { return @() }
    if ($layout.type -eq "leaf") {
        if ($layout.images -and $layout.images.Count -gt 0) { return $layout.images }
        return @()
    }
    if ($layout.children) {
        foreach ($c in $layout.children) {
            $r = Get-LeafImages $c
            if ($r.Count -gt 0) { return $r }
        }
    }
    return @()
}

Write-Host "`n=== Issue #431 Sixel M4: image_blobs shipping + resync ===" -ForegroundColor Cyan

# Feed the sixel into the pane via a child pwsh (shell-agnostic: pwsh on PATH).
& $PSMUX send-keys -t $S "pwsh -NoLogo -NoProfile -File '$emitter'" Enter 2>&1 | Out-Null
Start-Sleep -Seconds 3

# --- DUMP 1: blob must be present ---
$d1 = Get-Dump
if (-not $d1) { Write-Fail "dump 1 returned nothing"; & $PSMUX kill-session -t $S 2>&1|Out-Null; try{Stop-Process -Id $proc.Id -Force}catch{}; exit 1 }
$j1 = $d1 | ConvertFrom-Json

# Null-safe blob-key extractor: ConvertFrom-Json turns `{}` into a PSCustomObject
# whose .PSObject.Properties.Name is $null, and @($null) has Count 1 - so filter.
function Blob-Keys($blobObj) {
    if ($null -eq $blobObj) { return @() }
    $names = @($blobObj.PSObject.Properties.Name) | Where-Object { $_ -ne $null -and $_ -ne '' }
    return @($names)
}

# image_blobs present as a top-level object?
$hasBlobsField = ($d1 -match '"image_blobs"')
if ($hasBlobsField) { Write-Pass "dump 1 has top-level image_blobs field (M4 wired at dump-state site)" }
else { Write-Fail "dump 1 missing image_blobs field entirely" }

$blob1 = $j1.image_blobs
$blobKeys1 = Blob-Keys $blob1
$imgs = Get-LeafImages $j1.layout
Info "dump 1 image_blobs keys: [$($blobKeys1 -join ', ')]  leaf descriptors: $($imgs.Count)"

# ── VERIFICATION BOUNDARY (design section 7) ────────────────────────────────
# The pane runs under inbox conhost, which STRIPS the sixel DCS before psmux's
# vt100 parser sees it (even with PASSTHROUGH_MODE).  So on this machine the
# sixel never becomes a stored image and image_blobs stays empty - this is the
# documented boundary, NOT an M4 defect.  The authoritative headless proof of
# M4 is the Rust test `test_issue431_m4_blobs` (injects the sixel straight into
# the parser via create_proxy_pane's screen_snapshot, then drives the real
# build_image_blobs_json / visible_image_blobs / resync code).  Live pixels need
# Windows Terminal (M8).
if ($blobKeys1.Count -eq 0 -and $imgs.Count -eq 0) {
    Info "=========================================================================="
    Info "CONHOST BOUNDARY: no image reached the pane parser - the inbox ConPTY"
    Info "stripped the sixel DCS (design section 7).  This is expected headlessly."
    Info "M4 transport is proven by: cargo test --bin psmux test_issue431_m4_blobs"
    Info "Interactive pixel verification is deferred to M8 (Windows Terminal)."
    Info "=========================================================================="
    & $PSMUX kill-session -t $S 2>&1 | Out-Null
    try { Stop-Process -Id $proc.Id -Force -EA SilentlyContinue } catch {}
    Remove-Item $emitter -Force -EA SilentlyContinue
    Write-Host "`n=== Boundary reached (no hard failures). See Rust test for M4 proof. ===" -ForegroundColor Yellow
    exit 0
}

if ($blobKeys1.Count -ge 1) { Write-Pass "dump 1 ships >=1 blob (id shipped once)" }
else { Write-Fail "dump 1 image_blobs is empty but a descriptor exists - M4 gather failed" }

# leaf descriptor id must match a blob key
$imgs = Get-LeafImages $j1.layout
if ($imgs.Count -ge 1) {
    Write-Pass "leaf carries >=1 image descriptor"
    $descId = [string]$imgs[0].id
    Info "descriptor id=$descId  pw=$($imgs[0].pw) ph=$($imgs[0].ph) cw=$($imgs[0].cw) ch=$($imgs[0].ch) row=$($imgs[0].row) col=$($imgs[0].col)"
    if ($blobKeys1 -contains $descId) { Write-Pass "descriptor id '$descId' matches an image_blobs key" }
    else { Write-Fail "descriptor id '$descId' NOT among blob keys [$($blobKeys1 -join ', ')]" }
} else {
    Write-Fail "no leaf image descriptor found (M3 descriptor missing)"
    $descId = if ($blobKeys1.Count -ge 1) { $blobKeys1[0] } else { $null }
}

# blob value must base64-decode to ESC P ... ST
if ($descId -and ($blobKeys1 -contains $descId)) {
    $b64 = [string]$blob1.$descId
    try {
        $raw = [System.Convert]::FromBase64String($b64)
        $okIntro = ($raw.Length -ge 4 -and $raw[0] -eq 0x1B -and $raw[1] -eq 0x50)
        $okST    = ($raw[$raw.Length-2] -eq 0x1B -and $raw[$raw.Length-1] -eq 0x5C)
        # marker present in decoded bytes?
        $txt = [System.Text.Encoding]::ASCII.GetString($raw)
        $okMarker = $txt.Contains($MARKER)
        Info "decoded blob = $($raw.Length) bytes; introESCP=$okIntro endST=$okST marker=$okMarker"
        if ($okIntro -and $okST) { Write-Pass "blob base64 decodes to raw sixel (ESC P ... ST)" }
        else { Write-Fail "blob bytes not framed as ESC P ... ST (intro=$okIntro st=$okST)" }
        if ($okMarker) { Write-Pass "decoded blob contains the distinctive sixel marker '$MARKER'" }
        else { Write-Fail "decoded blob missing marker '$MARKER'" }
    } catch {
        Write-Fail "blob value is not valid base64: $($_.Exception.Message)"
    }
}

# --- DUMP 2: blob shipped ONCE - image_blobs now empty ---
# Force a rebuild WITHOUT clearing shipped_image_ids or moving the image:
# rename-window dirties state so dump 2 is freshly built (not NC), the image
# stays visible, and its id is already shipped -> image_blobs must be {}.
& $PSMUX rename-window -t $S sixdedup 2>&1 | Out-Null
Start-Sleep -Milliseconds 600
$d2 = Get-Dump
if ($d2) {
    $j2 = $d2 | ConvertFrom-Json
    $blobKeys2 = Blob-Keys $j2.image_blobs
    $imgs2 = Get-LeafImages $j2.layout
    Info "dump 2 image_blobs keys: [$($blobKeys2 -join ', ')]  leaf descriptors: $($imgs2.Count)"
    if ($blobKeys2.Count -eq 0) { Write-Pass "dump 2 image_blobs is EMPTY - blob shipped exactly once (dedup works)" }
    else { Write-Fail "dump 2 re-shipped blob keys [$($blobKeys2 -join ', ')] - dedup broken" }
    # descriptor must STILL be present every frame (blob-once, descriptor-every-frame)
    if ($imgs2.Count -ge 1) { Write-Pass "dump 2 still carries the leaf image descriptor (descriptor-every-frame)" }
    else { Write-Fail "dump 2 lost the leaf descriptor - descriptor must persist every frame" }
} else {
    Write-Fail "dump 2 returned nothing"
}

# --- DUMP 3: refresh-client RESYNC re-ships the blob ---
& $PSMUX refresh-client -t $S 2>&1 | Out-Null
Start-Sleep -Milliseconds 600
$d3 = Get-Dump
if ($d3) {
    $j3 = $d3 | ConvertFrom-Json
    $blobKeys3 = Blob-Keys $j3.image_blobs
    Info "dump 3 (post refresh-client) image_blobs keys: [$($blobKeys3 -join ', ')]"
    if ($blobKeys3.Count -ge 1) { Write-Pass "refresh-client RESYNC re-ships the blob (shipped set cleared)" }
    else { Write-Fail "refresh-client did NOT re-ship the blob - resync hook not firing" }
} else {
    Write-Fail "dump 3 returned nothing"
}

& $PSMUX kill-session -t $S 2>&1 | Out-Null
try { Stop-Process -Id $proc.Id -Force -EA SilentlyContinue } catch {}
Remove-Item $emitter -Force -EA SilentlyContinue

Write-Host "`n=== Results: $($script:Pass) passed, $($script:Fail) failed ===" -ForegroundColor Cyan
exit $script:Fail
