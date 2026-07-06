# Issue #431 (MODULE M7): dedicated session boolean option `sixel` (default on).
#
# Proves:
#   1. `sixel` defaults to on on a fresh session (show-options -g -v sixel).
#   2. `set-option -g sixel off` is reflected by show-options.
#   3. `set-option -g sixel on` flips it back.
#   4. The dump-state JSON carries a boolean `sixel` field (downstream M5/M6
#      client emit gate reads it) and it tracks the option value.
#
# The `sixel` option only gates client-side sixel emission; the server always
# parses+stores images. This test exercises the option plumbing + dump-state
# sync, all headless over the CLI and the TCP dump-state channel.

$ErrorActionPreference = "Continue"
$PSMUX = (Get-Command psmux -EA Stop).Source
$psmuxDir = "$env:USERPROFILE\.psmux"
$S = "issue431sixel"
$script:Pass = 0
$script:Fail = 0
function Write-Pass($m){ Write-Host "  [PASS] $m" -ForegroundColor Green; $script:Pass++ }
function Write-Fail($m){ Write-Host "  [FAIL] $m" -ForegroundColor Red; $script:Fail++ }

& $PSMUX kill-session -t $S 2>&1 | Out-Null
Start-Sleep -Milliseconds 400
Remove-Item "$psmuxDir\$S.*" -Force -EA SilentlyContinue

$proc = Start-Process -FilePath $PSMUX -ArgumentList "new-session","-s",$S -PassThru
$ok=$false
for($i=0;$i -lt 30;$i++){ Start-Sleep -Milliseconds 400; & $PSMUX has-session -t $S 2>$null; if($LASTEXITCODE -eq 0){$ok=$true;break} }
if(-not $ok){ Write-Fail "session did not start"; exit 1 }
Write-Pass "session started"

$port = (Get-Content "$psmuxDir\$S.port" -Raw).Trim()
$key  = (Get-Content "$psmuxDir\$S.key" -Raw).Trim()

# Fresh non-persistent dump each call = no stale pipeline frames.
function Get-SixelField {
  $tcp = [System.Net.Sockets.TcpClient]::new("127.0.0.1", [int]$port)
  $tcp.NoDelay=$true; $tcp.ReceiveTimeout=4000
  $st=$tcp.GetStream(); $w=[System.IO.StreamWriter]::new($st); $r=[System.IO.StreamReader]::new($st)
  $w.Write("AUTH $key`n"); $w.Flush(); $null=$r.ReadLine()
  $w.Write("dump-state`n"); $w.Flush()
  $best=$null
  for($j=0;$j -lt 300;$j++){ try{$line=$r.ReadLine()}catch{break}; if($null -eq $line){break}; if($line -ne "NC" -and $line.Length -gt 100){$best=$line; break} }
  $tcp.Close()
  if($best){
    $jj=$best|ConvertFrom-Json
    # PSObject.Properties lets us detect the field is actually present.
    $present = $jj.PSObject.Properties.Name -contains "sixel"
    return @{ present=$present; value=$jj.sixel }
  }
  return $null
}

function Get-ShowOpt {
  # show-options -g -v sixel prints just the value
  (& $PSMUX show-options -g -v sixel 2>&1 | Out-String).Trim()
}

Write-Host "`n=== Issue #431 M7: sixel option plumbing + dump-state field ===" -ForegroundColor Cyan

# 1. Default on
$v = Get-ShowOpt
if ($v -match "on") { Write-Pass "default: show-options -g -v sixel = 'on' (got '$v')" }
else { Write-Fail "default: expected 'on', got '$v'" }

$d = Get-SixelField
if ($d -and $d.present) { Write-Pass "dump-state carries a 'sixel' field" }
else { Write-Fail "dump-state missing 'sixel' field" }
if ($d -and $d.value -eq $true) { Write-Pass "dump-state sixel==true by default" }
else { Write-Fail "dump-state sixel not true by default (got '$($d.value)')" }

# 2. Turn off
& $PSMUX set-option -g sixel off 2>&1 | Out-Null
Start-Sleep -Milliseconds 400
$v = Get-ShowOpt
if ($v -match "off") { Write-Pass "set -g sixel off: show-options reflects 'off' (got '$v')" }
else { Write-Fail "set off: expected 'off', got '$v'" }
$d = Get-SixelField
if ($d -and $d.value -eq $false) { Write-Pass "dump-state sixel==false after set off" }
else { Write-Fail "dump-state sixel not false after set off (got '$($d.value)')" }

# 3. Turn back on
& $PSMUX set-option -g sixel on 2>&1 | Out-Null
Start-Sleep -Milliseconds 400
$v = Get-ShowOpt
if ($v -match "on") { Write-Pass "set -g sixel on: show-options reflects 'on' (got '$v')" }
else { Write-Fail "set on: expected 'on', got '$v'" }
$d = Get-SixelField
if ($d -and $d.value -eq $true) { Write-Pass "dump-state sixel==true after set on" }
else { Write-Fail "dump-state sixel not true after set on (got '$($d.value)')" }

# 4. show-options -g (full listing) contains the sixel line
$all = (& $PSMUX show-options -g 2>&1 | Out-String)
if ($all -match "(?m)^sixel\s+on") { Write-Pass "show-options -g lists 'sixel on'" }
else { Write-Fail "show-options -g does not list 'sixel on'" }

& $PSMUX kill-session -t $S 2>&1 | Out-Null
try { Stop-Process -Id $proc.Id -Force -EA SilentlyContinue } catch {}
Remove-Item "$psmuxDir\$S.*" -Force -EA SilentlyContinue

Write-Host "`n=== Results: $($script:Pass) passed, $($script:Fail) failed ===" -ForegroundColor Cyan
exit $script:Fail
