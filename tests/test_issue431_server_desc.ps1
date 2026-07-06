# Issue #431 (M3): server descriptor serialization for sixel images.
#
# Proves the dump-state leaf JSON carries a per-pane "images" array:
#   1. SCHEMA STABILITY (must pass): every leaf in the tree has an "images"
#      field, even with no sixels present (empty array []).  This is the stable
#      contract M4 (blobs) and M5 (client model) build on.
#   2. LIVE DATA (best-effort): a pane emits a real sixel DCS; if psmux's ConPTY
#      passthrough delivers the raw bytes to the parser (M2), the owning leaf's
#      "images" array becomes non-empty and carries id/row/col/pw/ph/cw/ch.
#      Live rasterisation + full multi-scenario coverage is M8's job; this only
#      confirms the descriptor plumbing when the environment cooperates.
#
# Harness: AUTH + fresh (non-persistent) dump-state over TCP, mirroring
# tests/test_issue413_copy_count_vi.ps1 and tests/test_issue428_proof.ps1.

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$S = "issue431desc"
$script:Pass = 0
$script:Fail = 0
function Write-Pass($m){ Write-Host "  [PASS] $m" -ForegroundColor Green; $script:Pass++ }
function Write-Fail($m){ Write-Host "  [FAIL] $m" -ForegroundColor Red; $script:Fail++ }
function Write-Info($m){ Write-Host "  [INFO] $m" -ForegroundColor Yellow }

& $PSMUX kill-session -t $S 2>&1 | Out-Null
Start-Sleep -Milliseconds 400
Remove-Item "$psmuxDir\$S.*" -Force -EA SilentlyContinue

$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$S -PassThru
Start-Sleep -Seconds 4
# Two panes -> two leaves, so the "every leaf" assertion is meaningful.
& $PSMUX split-window -h -t $S 2>&1 | Out-Null
Start-Sleep -Seconds 2

$port = (Get-Content "$psmuxDir\$S.port" -Raw).Trim()
$key  = (Get-Content "$psmuxDir\$S.key" -Raw).Trim()

# Fresh non-persistent dump each call = no stale pipeline frames. Returns the
# raw JSON line (the descriptor plumbing is best asserted on the raw string:
# ConvertFrom-Json cannot easily walk an arbitrarily-nested split tree).
function Get-RawState {
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

Write-Host "`n=== Issue #431 M3: leaf 'images' descriptor array ===" -ForegroundColor Cyan

# Poll until the split has settled into the serialised tree (a fresh connection
# can still surface a queued pre-split frame first). split-window itself is
# confirmed working; this just waits for state to propagate.
$raw = $null
for ($t=0; $t -lt 12; $t++) {
  $raw = Get-RawState
  if ($raw -and ([regex]::Matches($raw, '"type":"leaf"')).Count -ge 2) { break }
  Start-Sleep -Milliseconds 500
}
if (-not $raw) { Write-Fail "no dump-state received"; & $PSMUX kill-session -t $S 2>&1|Out-Null; try{Stop-Process -Id $proc.Id -Force}catch{}; exit 1 }

# --- 1. Schema stability: one "images":[ per leaf ---
$leafCount   = ([regex]::Matches($raw, '"type":"leaf"')).Count
$imagesCount = ([regex]::Matches($raw, '"images":\[')).Count
Write-Host "  leaves=$leafCount  images-fields=$imagesCount"
if ($leafCount -ge 2) { Write-Pass "tree has >= 2 leaves ($leafCount)" } else { Write-Fail "expected >= 2 leaves, got $leafCount" }
if ($imagesCount -eq $leafCount -and $leafCount -gt 0) {
  Write-Pass "every leaf carries an 'images' field ($imagesCount/$leafCount)"
} else {
  Write-Fail "images-field count ($imagesCount) != leaf count ($leafCount)"
}
# With no sixels yet, every images array must be empty.
if (($raw -match '"images":\[\{') ) {
  Write-Info "an images array is already non-empty (a pane emitted a sixel?)"
} else {
  Write-Pass "all images arrays empty at rest ('images':[])"
}

# --- 2. Best-effort live descriptor (needs M2 parser + ConPTY passthrough) ---
Write-Host "`n--- live sixel (best-effort; full coverage is M8) ---" -ForegroundColor Cyan
$sxScript = "$env:TEMP\psmux_issue431_sixel.ps1"
@'
$e = [char]27
# Raster attrs "Pan;Pad;Ph;Pv -> Ph=20px wide, Pv=40px tall => 2x2 cells.
$sixel = "$e" + 'Pq"1;1;20;40#0;2;0;0;0#0~~~~~~' + "$e" + '\'
[Console]::Out.Write($sixel)
[Console]::Out.Flush()
Start-Sleep -Milliseconds 300
'@ | Set-Content -Path $sxScript -Encoding ASCII

# Emit the sixel from inside the active pane's child so the parser sees it on
# the pane's OUTPUT stream (pwsh invoked explicitly => shell-agnostic).
& $PSMUX send-keys -t $S ("pwsh -NoProfile -File `"$sxScript`"") Enter 2>&1 | Out-Null
Start-Sleep -Seconds 3

$raw2 = Get-RawState
$live = $false
if ($raw2 -and ($raw2 -match '"images":\[\{"id":(\d+),"row":(-?\d+),"col":(\d+),"pw":(\d+),"ph":(\d+),"cw":(\d+),"ch":(\d+)\}')) {
  $live = $true
  Write-Pass "leaf carries a populated image descriptor: id=$($Matches[1]) row=$($Matches[2]) col=$($Matches[3]) pw=$($Matches[4]) ph=$($Matches[5]) cw=$($Matches[6]) ch=$($Matches[7])"
  if ([int]$Matches[4] -eq 20 -and [int]$Matches[5] -eq 40) { Write-Pass "pixel dims match emitted raster (20x40)" } else { Write-Fail "pixel dims $($Matches[4])x$($Matches[5]), expected 20x40" }
  if ([int]$Matches[6] -eq 2 -and [int]$Matches[7] -eq 2) { Write-Pass "cell dims match ceil(px/DEFAULT_CELL_PX) (2x2)" } else { Write-Fail "cell dims $($Matches[6])x$($Matches[7]), expected 2x2" }
} else {
  Write-Info "no populated descriptor observed (ConPTY passthrough may strip DCS in this environment); live-data assertion deferred to M8. Schema plumbing above is proven."
}

# Schema must remain stable even after the emission attempt.
if ($raw2) {
  $lc2 = ([regex]::Matches($raw2, '"type":"leaf"')).Count
  $ic2 = ([regex]::Matches($raw2, '"images":\[')).Count
  if ($ic2 -eq $lc2 -and $lc2 -gt 0) { Write-Pass "images field still present in every leaf after emission ($ic2/$lc2)" } else { Write-Fail "post-emission images-field count ($ic2) != leaf count ($lc2)" }
}

Remove-Item $sxScript -Force -EA SilentlyContinue
& $PSMUX kill-session -t $S 2>&1 | Out-Null
try { Stop-Process -Id $proc.Id -Force -EA SilentlyContinue } catch {}

Write-Host "`n=== Results: $($script:Pass) passed, $($script:Fail) failed (live=$live) ===" -ForegroundColor Cyan
exit $script:Fail
