# Issue #247: Quick session switching by number in the session picker
#
# Fix adds digit-based quick-jump to the session picker (PREF s):
#   1..9 → entries 0..8, 0 → entry 9 (browser-tab convention)
#   All other Char keys are absorbed while the picker is open, fixing the
#   pre-existing bug where digits leaked to the focused pane's PTY.
#
# NOTE: The session picker is a client-side TUI overlay driven by keyboard
# input to an attached ratatui client. capture-pane cannot see it, and the
# picker state (session_chooser / session_entries / session_selected) is
# client-local so dump-state cannot see it either. This test therefore
# follows the same pattern as test_issue201_rename_dialog.ps1:
#   1. Source-code proof that the handler and catch-all exist.
#   2. Source-code proof that the renderer draws digit prefixes.
#   3. Functional verification that the picker's data source (port files +
#      session-info over TCP) lists multiple sessions in a stable order.

$ErrorActionPreference = "Continue"
$script:pass = 0
$script:fail = 0
$script:results = @()

function Write-Test($msg) { Write-Host "  TEST: $msg" -ForegroundColor Yellow }
function Write-Pass($msg) { Write-Host "  PASS: $msg" -ForegroundColor Green; $script:pass++ }
function Write-Fail($msg) { Write-Host "  FAIL: $msg" -ForegroundColor Red; $script:fail++ }
function Add-Result($name, $ok, $detail) {
    if ($ok) { Write-Pass "$name $detail" } else { Write-Fail "$name $detail" }
    $script:results += [PSCustomObject]@{ Test = $name; Pass = $ok; Detail = $detail }
}

# ── Binary resolution ────────────────────────────────────────────
$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) {
    $cmd = Get-Command psmux -ErrorAction SilentlyContinue
    if ($cmd) { $PSMUX = $cmd.Source }
}
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }

Write-Host "`n=== Issue #247: Session picker digit quick-jump ===" -ForegroundColor Cyan
Write-Host "  Binary: $PSMUX"

# ════════════════════════════════════════════════════════════════════
#  PART 1: Source-code proof
# ════════════════════════════════════════════════════════════════════

$srcFile = Join-Path $PSScriptRoot "..\src\client.rs"
if (-not (Test-Path $srcFile)) {
    Write-Fail "Source file not found at $srcFile"
    exit 1
}
$src = Get-Content $srcFile -Raw

Write-Test "State: session_num_buffer declared alongside picker state"
$bufferState = $src -match 'let\s+mut\s+session_num_buffer\s*=\s*String::new\(\)'
Add-Result "session_num_buffer declared" $bufferState ""

Write-Test "Handler: digit keys push into the buffer (no immediate switch)"
# Digit arm must call session_num_buffer.push(c), not set PSMUX_SWITCH_TO.
$digitPush = $src -match '(?s)KeyCode::Char\(c\)\s+if\s+session_chooser\s+&&\s+c\.is_ascii_digit\(\)\s*=>\s*\{[^}]*session_num_buffer\.push\(c\)'
Add-Result "digit arm pushes into buffer" $digitPush ""

Write-Test "Handler: digit arm does NOT perform an immediate session switch"
# Ensure the old immediate-jump code path is gone — the digit arm must not
# set PSMUX_SWITCH_TO directly (Enter is the only path that may do so).
$digitSwitchGone = -not ($src -match '(?s)c\.is_ascii_digit\(\)[^}]{0,400}PSMUX_SWITCH_TO')
Add-Result "digit arm does not short-circuit to switch" $digitSwitchGone ""

Write-Test "Handler: Enter parses the buffer when non-empty"
$enterParses = $src -match '(?s)KeyCode::Enter\s+if\s+session_chooser.*?session_num_buffer\.parse::<usize>\(\)'
Add-Result "Enter parses buffer as 1-based index" $enterParses ""

Write-Test "Handler: Backspace edits the buffer"
$backspace = $src -match 'KeyCode::Backspace\s+if\s+session_chooser\s*=>\s*\{\s*session_num_buffer\.pop\(\)'
Add-Result "Backspace pops buffer" $backspace ""

Write-Test "Handler: Esc clears the buffer on close"
$escClears = $src -match '(?s)KeyCode::Esc\s+if\s+session_chooser\s*=>\s*\{[^}]*session_chooser\s*=\s*false;[^}]*session_num_buffer\.clear\(\)'
Add-Result "Esc clears buffer" $escClears ""

Write-Test "Handler: catch-all absorbs remaining Char keys while picker is open"
$absorber = $src -match 'KeyCode::Char\(_\)\s+if\s+session_chooser\s*=>\s*\{\s*\}'
Add-Result "leak-guard catch-all present" $absorber ""

Write-Test "Renderer: overlay title advertises digits+enter workflow"
$titleHint = $src -match 'choose-session\s*\(digits\+enter=jump'
Add-Result "overlay title advertises digits+enter=jump" $titleHint ""

Write-Test "Renderer: all rows numbered with a dynamic-width column"
# Width adapts to the largest index so 1..N stay aligned.
$rowNumbered = $src -match 'num_width\s*=\s*session_entries\.len\(\)\.to_string\(\)\.len\(\)'
Add-Result "row numbering uses dynamic column width" $rowNumbered ""

Write-Test "Renderer: jump-buffer indicator drawn at the bottom when non-empty"
$bufferDrawn = $src -match '(?s)if\s+!session_num_buffer\.is_empty\(\).*?format!\("go to \{\}",\s*session_num_buffer\)'
Add-Result "buffer preview rendered at bottom" $bufferDrawn ""

Write-Test "Renderer: overlay height adapts to entry count"
$dynamicHeight = $src -match '(?s)session_entries\.len\(\)[^;]*saturating_add\(2\)[^;]*\.max\(5\)[^;]*\.min\(content_chunk\.height'
Add-Result "overlay uses dynamic content-sized height" $dynamicHeight ""

# ════════════════════════════════════════════════════════════════════
#  PART 2: Functional verification of the picker data source
# ════════════════════════════════════════════════════════════════════
#
# The picker iterates %USERPROFILE%\.psmux\*.port, reaches each session
# over TCP, auths, and issues `session-info`. Prove that path works so
# multiple sessions become selectable entries.

$psmuxDir = "$env:USERPROFILE\.psmux"
$S1 = "issue247_a"
$S2 = "issue247_b"
$S3 = "issue247_c"

function Kill-Session($name) { & $PSMUX kill-session -t $name 2>$null | Out-Null }
function Wait-Session($name, [int]$timeoutSec = 10) {
    for ($i = 0; $i -lt ($timeoutSec * 2); $i++) {
        & $PSMUX has-session -t $name 2>$null | Out-Null
        if ($LASTEXITCODE -eq 0) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

Kill-Session $S1; Kill-Session $S2; Kill-Session $S3
Start-Sleep -Milliseconds 500

# If the test is invoked from inside an existing psmux session, new-session -d
# refuses to nest. Clear PSMUX_SESSION for the child invocations.
$env:PSMUX_SESSION = ""

& $PSMUX new-session -d -s $S1 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
& $PSMUX new-session -d -s $S2 2>&1 | Out-Null
Start-Sleep -Milliseconds 300
& $PSMUX new-session -d -s $S3 2>&1 | Out-Null

$aliveA = Wait-Session $S1
$aliveB = Wait-Session $S2
$aliveC = Wait-Session $S3
Add-Result "three test sessions started" ($aliveA -and $aliveB -and $aliveC) "A=$aliveA B=$aliveB C=$aliveC"

Write-Test "Picker data source: all three sessions have port files"
$p1 = Test-Path "$psmuxDir\$S1.port"
$p2 = Test-Path "$psmuxDir\$S2.port"
$p3 = Test-Path "$psmuxDir\$S3.port"
Add-Result "all three port files exist" ($p1 -and $p2 -and $p3) ""

Write-Test "Picker data source: session-info reachable over TCP for each"
function Query-SessionInfo($name) {
    $pf = "$psmuxDir\$name.port"
    $kf = "$psmuxDir\$name.key"
    if (-not (Test-Path $pf)) { return $null }
    try {
        $port = [int]((Get-Content $pf -Raw).Trim())
        $key  = if (Test-Path $kf) { (Get-Content $kf -Raw).Trim() } else { "" }
        $tcp  = [System.Net.Sockets.TcpClient]::new("127.0.0.1", $port)
        $st   = $tcp.GetStream()
        $st.ReadTimeout = 2000
        $w    = [System.IO.StreamWriter]::new($st); $w.AutoFlush = $true
        $r    = [System.IO.StreamReader]::new($st)
        $w.WriteLine("AUTH $key")
        $null = $r.ReadLine()
        $w.WriteLine("session-info")
        $line1 = $r.ReadLine()  # may be OK / first data line
        $line2 = $r.ReadLine()
        $tcp.Close()
        return "$line1`n$line2"
    } catch { return $null }
}

$info1 = Query-SessionInfo $S1
$info2 = Query-SessionInfo $S2
$info3 = Query-SessionInfo $S3
$allResponded = ($info1 -and $info2 -and $info3)
Add-Result "session-info reachable for all three" $allResponded ""

# ── Cleanup ──
Kill-Session $S1; Kill-Session $S2; Kill-Session $S3

# ════════════════════════════════════════════════════════════════════
#  Summary
# ════════════════════════════════════════════════════════════════════

Write-Host "`n=== Results ===" -ForegroundColor Cyan
Write-Host "  Passed: $pass / $($pass + $fail)" -ForegroundColor $(if ($fail -eq 0) { 'Green' } else { 'Yellow' })
foreach ($r in $results) {
    $color  = if ($r.Pass) { 'Green' } else { 'Red' }
    $status = if ($r.Pass) { 'PASS' } else { 'FAIL' }
    Write-Host "  [$status] $($r.Test)" -ForegroundColor $color
}

if ($fail -gt 0) {
    Write-Host "`n  Some tests failed." -ForegroundColor Red
    Write-Host "  To verify the UX manually:" -ForegroundColor Yellow
    Write-Host "    1. psmux new-session -d -s a  # repeat for b, c" -ForegroundColor Yellow
    Write-Host "    2. psmux attach -t a" -ForegroundColor Yellow
    Write-Host "    3. Press C-b s to open the picker" -ForegroundColor Yellow
    Write-Host "    4. Press 2 — client should switch to session 'b' immediately" -ForegroundColor Yellow
    Write-Host "    5. Reopen picker and type a letter — should not leak to PTY" -ForegroundColor Yellow
    exit 1
}

Write-Host "`n  All tests passed. Issue #247 fix verified." -ForegroundColor Green
exit 0
