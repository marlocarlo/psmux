# =============================================================================
# FORMAT ENGINE COMPREHENSIVE TEST
# Tests all tmux-compatible format features in psmux
# =============================================================================
$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) {
    Write-Host "[FATAL] Binary not found" -ForegroundColor Red; exit 1
}

$SESSION = "fmt_test_$(Get-Random)"

Write-Info "Binary: $PSMUX"
Write-Info "Session: $SESSION"

# Start session
Start-Process -FilePath $PSMUX -ArgumentList "new-session", "-d", "-s", $SESSION -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 3
$ls = (& $PSMUX ls 2>&1) -join "`n"
if ($ls -notmatch [regex]::Escape($SESSION)) {
    Write-Host "[FATAL] Could not start session" -ForegroundColor Red; exit 1
}

function Fmt { param($f) (& $PSMUX display-message -t $SESSION -p "$f" 2>&1 | Out-String).Trim() }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "1. SIMPLE VARIABLE EXPANSION"
Write-Host ("=" * 70)

Write-Test "#{session_name}"
$v = Fmt '#{session_name}'
if ($v -eq $SESSION) { Write-Pass "session_name = $v" } else { Write-Fail "Expected '$SESSION', got '$v'" }

Write-Test "#{window_index}"
$v = Fmt '#{window_index}'
if ($v -eq "0") { Write-Pass "window_index = $v" } else { Write-Fail "Expected '0', got '$v'" }

Write-Test "#{window_name}"
$v = Fmt '#{window_name}'
if ($v -eq "pwsh") { Write-Pass "window_name = $v" } else { Write-Fail "Expected 'pwsh', got '$v'" }

Write-Test "#{pane_index}"
$v = Fmt '#{pane_index}'
if ($v -eq "0") { Write-Pass "pane_index = $v" } else { Write-Fail "Expected '0', got '$v'" }

Write-Test "#{pane_id}"
$v = Fmt '#{pane_id}'
if ($v -match '^%\d+$') { Write-Pass "pane_id = $v" } else { Write-Fail "Expected %%N, got '$v'" }

Write-Test "#{session_id}"
$v = Fmt '#{session_id}'
if ($v -match '^\$\d+$') { Write-Pass "session_id = $v" } else { Write-Fail "Expected \$N, got '$v'" }

Write-Test "#{window_id}"
$v = Fmt '#{window_id}'
if ($v -match '^@\d+$') { Write-Pass "window_id = $v" } else { Write-Fail "Expected @N, got '$v'" }

Write-Test "#{window_active}"
$v = Fmt '#{window_active}'
if ($v -eq "1") { Write-Pass "window_active = $v" } else { Write-Fail "Expected '1', got '$v'" }

Write-Test "#{pane_active}"
$v = Fmt '#{pane_active}'
if ($v -eq "1") { Write-Pass "pane_active = $v" } else { Write-Fail "Expected '1', got '$v'" }

Write-Test "#{pane_width} is numeric"
$v = Fmt '#{pane_width}'
if ($v -match '^\d+$' -and [int]$v -gt 0) { Write-Pass "pane_width = $v" } else { Write-Fail "Got '$v'" }

Write-Test "#{pane_height} is numeric"
$v = Fmt '#{pane_height}'
if ($v -match '^\d+$' -and [int]$v -gt 0) { Write-Pass "pane_height = $v" } else { Write-Fail "Got '$v'" }

Write-Test "#{version}"
$v = Fmt '#{version}'
if ($v -match '^\d+\.\d+\.\d+$') { Write-Pass "version = $v" } else { Write-Fail "Got '$v'" }

Write-Test "#{host}"
$v = Fmt '#{host}'
if ($v.Length -gt 0) { Write-Pass "host = $v" } else { Write-Fail "Got empty" }

Write-Test "#{cursor_x} is numeric"
$v = Fmt '#{cursor_x}'
if ($v -match '^\d+$') { Write-Pass "cursor_x = $v" } else { Write-Fail "Got '$v'" }

Write-Test "#{cursor_y} is numeric"
$v = Fmt '#{cursor_y}'
if ($v -match '^\d+$') { Write-Pass "cursor_y = $v" } else { Write-Fail "Got '$v'" }

Write-Test "#{session_windows}"
$v = Fmt '#{session_windows}'
if ($v -eq "1") { Write-Pass "session_windows = $v" } else { Write-Fail "Expected '1', got '$v'" }

Write-Test "#{window_panes}"
$v = Fmt '#{window_panes}'
if ($v -eq "1") { Write-Pass "window_panes = $v" } else { Write-Fail "Expected '1', got '$v'" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "2. SHORTHAND VARIABLES"
Write-Host ("=" * 70)

Write-Test "#S (session name)"
$v = Fmt '#S'
if ($v -eq $SESSION) { Write-Pass "#S = $v" } else { Write-Fail "Expected '$SESSION', got '$v'" }

Write-Test "#I (window index)"
$v = Fmt '#I'
if ($v -eq "0") { Write-Pass "#I = $v" } else { Write-Fail "Expected '0', got '$v'" }

Write-Test "#W (window name)"
$v = Fmt '#W'
if ($v -eq "pwsh") { Write-Pass "#W = $v" } else { Write-Fail "Expected 'pwsh', got '$v'" }

Write-Test "#P (pane index)"
$v = Fmt '#P'
if ($v -eq "0") { Write-Pass "#P = $v" } else { Write-Fail "Expected '0', got '$v'" }

Write-Test "#H (hostname)"
$v = Fmt '#H'
if ($v.Length -gt 0) { Write-Pass "#H = $v" } else { Write-Fail "Got empty" }

Write-Test "#D (pane id - tmux unique pane identifier)"
$v = Fmt '#D'
if ($v -match '^%\d+$') { Write-Pass "#D = $v" } else { Write-Fail "Expected %%N, got '$v'" }

Write-Test "## (literal #)"
$v = Fmt '##'
if ($v -eq "#") { Write-Pass "## = #" } else { Write-Fail "Expected '#', got '$v'" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "3. COMPOUND FORMAT STRINGS"
Write-Host ("=" * 70)

Write-Test "Compound: session:window"
$v = Fmt '#{session_name}:#{window_index}'
if ($v -eq "${SESSION}:0") { Write-Pass "compound = $v" } else { Write-Fail "Expected '${SESSION}:0', got '$v'" }

Write-Test "Compound: [session] index:name"
$v = Fmt '[#S] #I:#W'
if ($v -eq "[$SESSION] 0:pwsh") { Write-Pass "compound = $v" } else { Write-Fail "Expected '[$SESSION] 0:pwsh', got '$v'" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "4. CONDITIONALS"
Write-Host ("=" * 70)

Write-Test "#{?window_active,ACTIVE,inactive}"
$v = Fmt '#{?window_active,ACTIVE,inactive}'
if ($v -eq "ACTIVE") { Write-Pass "conditional true = $v" } else { Write-Fail "Expected 'ACTIVE', got '$v'" }

Write-Test "#{?window_zoomed_flag,ZOOMED,normal}"
$v = Fmt '#{?window_zoomed_flag,ZOOMED,normal}'
if ($v -eq "normal") { Write-Pass "conditional false = $v" } else { Write-Fail "Expected 'normal', got '$v'" }

Write-Test "Nested conditional variable"
$v = Fmt '#{?#{window_active},YES,NO}'
if ($v -eq "YES") { Write-Pass "nested conditional = $v" } else { Write-Fail "Expected 'YES', got '$v'" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "5. COMPARISON OPERATORS"
Write-Host ("=" * 70)

Write-Test "#{?a==a,EQUAL,DIFF}"
$v = Fmt '#{?a==a,EQUAL,DIFF}'
if ($v -eq "EQUAL") { Write-Pass "== true = $v" } else { Write-Fail "Expected 'EQUAL', got '$v'" }

Write-Test "#{?a!=b,DIFF,SAME}"
$v = Fmt '#{?a!=b,DIFF,SAME}'
if ($v -eq "DIFF") { Write-Pass "!= true = $v" } else { Write-Fail "Expected 'DIFF', got '$v'" }

Write-Test "#{?a==b,YES,NO}"
$v = Fmt '#{?a==b,YES,NO}'
if ($v -eq "NO") { Write-Pass "== false = $v" } else { Write-Fail "Expected 'NO', got '$v'" }

Write-Test "#{==:hello,hello} returns 1"
$v = Fmt '#{==:hello,hello}'
if ($v -eq "1") { Write-Pass "top-level == = $v" } else { Write-Fail "Expected '1', got '$v'" }

Write-Test "#{!=:hello,world} returns 1"
$v = Fmt '#{!=:hello,world}'
if ($v -eq "1") { Write-Pass "top-level != = $v" } else { Write-Fail "Expected '1', got '$v'" }

Write-Test "Comparison with nested #{} in condition"
$v = Fmt '#{?#{session_name}==#{session_name},SAME,DIFF}'
if ($v -eq "SAME") { Write-Pass "nested comparison = $v" } else { Write-Fail "Expected 'SAME', got '$v'" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "6. TRUNCATION"
Write-Host ("=" * 70)

Write-Test "#{=5:session_name} truncates to 5 chars"
$v = Fmt '#{=5:session_name}'
if ($v.Length -le 5) { Write-Pass "truncation = '$v' (len=$($v.Length))" } else { Write-Fail "Expected <=5 chars, got '$v' (len=$($v.Length))" }

Write-Test "#{=-3:session_name} takes last 3 chars"
$v = Fmt '#{=-3:session_name}'
$expected = $SESSION.Substring($SESSION.Length - 3)
if ($v -eq $expected) { Write-Pass "neg truncation = '$v'" } else { Write-Fail "Expected '$expected', got '$v'" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "7. STRING SUBSTITUTION"
Write-Host ("=" * 70)

Write-Test "#{s/pwsh/SHELL/:window_name}"
$v = Fmt '#{s/pwsh/SHELL/:window_name}'
if ($v -eq "SHELL") { Write-Pass "substitution = $v" } else { Write-Fail "Expected 'SHELL', got '$v'" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "8. PADDING"
Write-Host ("=" * 70)

Write-Test "#{p10:window_name} right-pads to 10 chars"
# Use left-pad (negative) to avoid PowerShell trimming trailing spaces
$v = (& $PSMUX display-message -t $SESSION -p '#{p-10:window_name}' 2>&1 | Out-String).TrimEnd("`n").TrimEnd("`r")
if ($v.Length -eq 10 -and $v.EndsWith("pwsh")) { Write-Pass "padding = '$v' (len=$($v.Length))" } else { Write-Fail "Expected 10 chars ending with 'pwsh', got '$v' (len=$($v.Length))" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "9. ARITHMETIC"
Write-Host ("=" * 70)

Write-Test "#{e|+|:3,4} = 7"
$v = Fmt '#{e|+|:3,4}'
if ($v -eq "7") { Write-Pass "addition = $v" } else { Write-Fail "Expected '7', got '$v'" }

Write-Test "#{e|-|:10,3} = 7"
$v = Fmt '#{e|-|:10,3}'
if ($v -eq "7") { Write-Pass "subtraction = $v" } else { Write-Fail "Expected '7', got '$v'" }

Write-Test "#{e|*|:3,4} = 12"
$v = Fmt '#{e|*|:3,4}'
if ($v -eq "12") { Write-Pass "multiplication = $v" } else { Write-Fail "Expected '12', got '$v'" }

Write-Test "#{e|/|:12,3} = 4"
$v = Fmt '#{e|/|:12,3}'
if ($v -eq "4") { Write-Pass "division = $v" } else { Write-Fail "Expected '4', got '$v'" }

Write-Test "#{e|m|:10,3} = 1"
$v = Fmt '#{e|m|:10,3}'
if ($v -eq "1") { Write-Pass "modulo = $v" } else { Write-Fail "Expected '1', got '$v'" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "10. BOOLEAN OPERATORS"
Write-Host ("=" * 70)

Write-Test "#{||:1,0} = 1"
$v = Fmt '#{||:1,0}'
if ($v -eq "1") { Write-Pass "OR true = $v" } else { Write-Fail "Expected '1', got '$v'" }

Write-Test "#{||:0,0} = 0"
$v = Fmt '#{||:0,0}'
if ($v -eq "0") { Write-Pass "OR false = $v" } else { Write-Fail "Expected '0', got '$v'" }

Write-Test "#{&&:1,1} = 1"
$v = Fmt '#{&&:1,1}'
if ($v -eq "1") { Write-Pass "AND true = $v" } else { Write-Fail "Expected '1', got '$v'" }

Write-Test "#{&&:1,0} = 0"
$v = Fmt '#{&&:1,0}'
if ($v -eq "0") { Write-Pass "AND false = $v" } else { Write-Fail "Expected '0', got '$v'" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "11. MODIFIERS: basename, dirname, quote, width, literal"
Write-Host ("=" * 70)

Write-Test "#{b:C:/Users/test/file.txt} = file.txt"
$v = Fmt '#{b:C:/Users/test/file.txt}'
if ($v -eq "file.txt") { Write-Pass "basename = $v" } else { Write-Fail "Expected 'file.txt', got '$v'" }

Write-Test "#{d:C:/Users/test/file.txt} = C:/Users/test"
$v = Fmt '#{d:C:/Users/test/file.txt}'
if ($v -eq "C:/Users/test") { Write-Pass "dirname = $v" } else { Write-Fail "Expected 'C:/Users/test', got '$v'" }

Write-Test "#{w:hello} = 5"
$v = Fmt '#{w:hello}'
if ($v -eq "5") { Write-Pass "width = $v" } else { Write-Fail "Expected '5', got '$v'" }

Write-Test "#{l:literal text} = literal text"
$v = Fmt '#{l:literal text}'
if ($v -eq "literal text") { Write-Pass "literal = $v" } else { Write-Fail "Expected 'literal text', got '$v'" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "12. GLOB MATCH"
Write-Host ("=" * 70)

Write-Test "#{m:pw*,pwsh} = 1"
$v = Fmt '#{m:pw*,pwsh}'
if ($v -eq "1") { Write-Pass "glob match = $v" } else { Write-Fail "Expected '1', got '$v'" }

Write-Test "#{m:xyz*,pwsh} = 0"
$v = Fmt '#{m:xyz*,pwsh}'
if ($v -eq "0") { Write-Pass "glob no-match = $v" } else { Write-Fail "Expected '0', got '$v'" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "13. WINDOW LOOP #{W:fmt}"
Write-Host ("=" * 70)

# Create a second window for loop tests
& $PSMUX new-window -t $SESSION 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

Write-Test "#{W:#{window_index}} lists all window indices"
$v = Fmt '#{W:#{window_index}}'
if ($v -match "0" -and $v -match "1") { Write-Pass "W loop = '$v'" } else { Write-Fail "Expected '0' and '1', got '$v'" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "14. PANE LOOP #{P:fmt} — BUG FIX VERIFICATION"
Write-Host ("=" * 70)

# Split the current window to have 2 panes
& $PSMUX split-window -t $SESSION 2>&1 | Out-Null
Start-Sleep -Milliseconds 500

Write-Test "#{P:#{pane_index}} lists distinct pane indices"
$v = Fmt '#{P:#{pane_index}}'
Write-Info "P loop result: '$v'"
# Should have two distinct pane indices (e.g., "0 1")
$parts = $v -split '\s+'
if ($parts.Count -eq 2 -and $parts[0] -ne $parts[1]) {
    Write-Pass "P loop returns distinct pane indices: $v"
} else {
    Write-Fail "P loop should return 2 distinct pane indices, got '$v' (PANE_POS_OVERRIDE bug if identical)"
}

Write-Test "#{P:#{pane_id}} lists distinct pane IDs"
$v = Fmt '#{P:#{pane_id}}'
Write-Info "P loop pane_ids: '$v'"
$parts = $v -split '\s+'
if ($parts.Count -eq 2 -and $parts[0] -ne $parts[1]) {
    Write-Pass "P loop returns distinct pane IDs: $v"
} else {
    Write-Fail "P loop should return 2 distinct pane IDs, got '$v'"
}

Write-Test "#{P:#{pane_width}} lists pane widths for each pane"
$v = Fmt '#{P:#{pane_width}}'
Write-Info "P loop pane_widths: '$v'"
$parts = $v -split '\s+'
if ($parts.Count -eq 2) {
    Write-Pass "P loop returns 2 pane widths: $v"
} else {
    Write-Fail "Expected 2 values, got '$v'"
}

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "15. BUFFER FORMAT — BUG FIX VERIFICATION"
Write-Host ("=" * 70)

# Set up multiple buffers
& $PSMUX set-buffer -t $SESSION "first-buffer" 2>&1 | Out-Null
Start-Sleep -Milliseconds 200
& $PSMUX set-buffer -t $SESSION "second-buffer-longer" 2>&1 | Out-Null
Start-Sleep -Milliseconds 200

Write-Test "list-buffers -F shows distinct buffer sizes"
$v = & $PSMUX list-buffers -t $SESSION -F '#{buffer_name}:#{buffer_size}' 2>&1
$vStr = ($v | Out-String).Trim()
Write-Info "list-buffers -F: '$vStr'"
$lines = $vStr -split "`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ -ne "" }
if ($lines.Count -ge 2) {
    $allDifferent = $true
    for ($i = 0; $i -lt $lines.Count - 1; $i++) {
        if ($lines[$i] -eq $lines[$i + 1]) { $allDifferent = $false }
    }
    if ($allDifferent) {
        Write-Pass "list-buffers -F returns distinct entries for each buffer"
    } else {
        Write-Fail "list-buffers -F entries are identical (BUFFER_IDX_OVERRIDE bug): $vStr"
    }
} else {
    Write-Fail "Expected 2+ buffer entries, got $($lines.Count): '$vStr'"
}

Write-Test "list-buffers -F #{buffer_name} shows buffer0000, buffer0001..."
$v3 = & $PSMUX list-buffers -t $SESSION -F '#{buffer_name}' 2>&1
$v3Str = ($v3 | Out-String).Trim()
Write-Info "buffer_name raw: '$v3Str'"
if ($v3Str -match "buffer0000" -and $v3Str -match "buffer0001") {
    Write-Pass "Buffer names are indexed correctly"
} else {
    Write-Fail "Expected buffer0000 and buffer0001, got '$v3Str'"
}

Write-Test "list-buffers -F #{buffer_sample} shows buffer content"
$v2 = & $PSMUX list-buffers -t $SESSION -F '#{buffer_sample}' 2>&1
$v2Str = ($v2 | Out-String).Trim()
Write-Info "buffer_sample raw: '$v2Str'"
if ($v2Str -match "second-buffer-longer" -and $v2Str -match "first-buffer") {
    Write-Pass "buffer_sample shows actual content for each buffer"
} else {
    Write-Fail "Expected both buffer contents, got '$v2Str'"
}

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "16. new-session -P -F FORMAT"
Write-Host ("=" * 70)

Write-Test "new-session -P (default format)"
$nsDefault = & $PSMUX new-session -d -s test_ns_fmt -P 2>&1
$nsStr = ($nsDefault | Out-String).Trim()
Write-Info "new-session -P default: '$nsStr'"
if ($nsStr -eq "test_ns_fmt:") { Write-Pass "Default format = 'session:'" } else { Write-Fail "Expected 'test_ns_fmt:', got '$nsStr'" }

Write-Test "new-session -P -F '#{session_name}:#{window_index}'"
$nsFull = & $PSMUX new-session -d -s test_ns_fmt2 -P -F '#{session_name}:#{window_index}' 2>&1
$nsFullStr = ($nsFull | Out-String).Trim()
Write-Info "new-session -P -F complex: '$nsFullStr'"
if ($nsFullStr -eq "test_ns_fmt2:0") { Write-Pass "Complex format = '$nsFullStr'" } else { Write-Fail "Expected 'test_ns_fmt2:0', got '$nsFullStr'" }

Write-Test "new-session -P -F '#{pane_id}'"
$nsPid = & $PSMUX new-session -d -s test_ns_fmt3 -P -F '#{pane_id}' 2>&1
$nsPidStr = ($nsPid | Out-String).Trim()
Write-Info "new-session -P -F pane_id: '$nsPidStr'"
if ($nsPidStr -match '^%\d+$') { Write-Pass "pane_id format = '$nsPidStr'" } else { Write-Fail "Expected %%N, got '$nsPidStr'" }

# Cleanup new-session test sessions
& $PSMUX kill-session -t test_ns_fmt 2>$null
& $PSMUX kill-session -t test_ns_fmt2 2>$null
& $PSMUX kill-session -t test_ns_fmt3 2>$null
Start-Sleep -Milliseconds 500

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "17. new-window -P -F FORMAT"
Write-Host ("=" * 70)

Write-Test "new-window -P (default format)"
$nwDefault = & $PSMUX new-window -t $SESSION -P 2>&1
$nwStr = ($nwDefault | Out-String).Trim()
Write-Info "new-window -P default: '$nwStr'"
if ($nwStr -match "^${SESSION}:\d+$") { Write-Pass "Default new-window format = '$nwStr'" } else { Write-Fail "Expected 'session:N', got '$nwStr'" }

Write-Test "new-window -P -F '#{session_name}:#{window_index}:#{pane_id}'"
$nwFull = & $PSMUX new-window -t $SESSION -P -F '#{session_name}:#{window_index}:#{pane_id}' 2>&1
$nwFullStr = ($nwFull | Out-String).Trim()
Write-Info "new-window -P -F complex: '$nwFullStr'"
if ($nwFullStr -match "^${SESSION}:\d+:%\d+$") { Write-Pass "Complex new-window format = '$nwFullStr'" } else { Write-Fail "Expected 'session:N:%%N', got '$nwFullStr'" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "18. split-window -P -F FORMAT"
Write-Host ("=" * 70)

Write-Test "split-window -P (default format)"
$swDefault = & $PSMUX split-window -t $SESSION -P 2>&1
$swStr = ($swDefault | Out-String).Trim()
Write-Info "split-window -P default: '$swStr'"
if ($swStr -match "^${SESSION}:\d+\.\d+$") { Write-Pass "Default split-window format = '$swStr'" } else { Write-Fail "Expected 'session:N.N', got '$swStr'" }

Write-Test "split-window -P -F '#{pane_id}'"
$swPid = & $PSMUX split-window -t $SESSION -P -F '#{pane_id}' 2>&1
$swPidStr = ($swPid | Out-String).Trim()
Write-Info "split-window -P -F pane_id: '$swPidStr'"
if ($swPidStr -match '^%\d+$') { Write-Pass "split-window pane_id format = '$swPidStr'" } else { Write-Fail "Expected %%N, got '$swPidStr'" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "19. list-windows -F FORMAT"
Write-Host ("=" * 70)

Write-Test "list-windows -F '#{window_index}:#{window_name}'"
$lwFmt = & $PSMUX list-windows -t $SESSION -F '#{window_index}:#{window_name}' 2>&1
$lwStr = ($lwFmt | Out-String).Trim()
Write-Info "list-windows -F: '$lwStr'"
$lwLines = $lwStr -split "`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ -ne "" }
if ($lwLines.Count -gt 1 -and ($lwStr -match '\d+:pwsh')) {
    Write-Pass "list-windows -F shows formatted entries ($($lwLines.Count) windows)"
} else {
    Write-Fail "Expected multiple formatted entries, got '$lwStr'"
}

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "20. list-panes -F FORMAT"
Write-Host ("=" * 70)

Write-Test "list-panes -F '#{pane_index}:#{pane_id}:#{pane_width}x#{pane_height}'"
$lpFmt = & $PSMUX list-panes -t $SESSION -F '#{pane_index}:#{pane_id}:#{pane_width}x#{pane_height}' 2>&1
$lpStr = ($lpFmt | Out-String).Trim()
Write-Info "list-panes -F: '$lpStr'"
$lpLines = $lpStr -split "`n" | ForEach-Object { $_.Trim() } | Where-Object { $_ -ne "" }
if ($lpLines.Count -ge 2) {
    # Check pane indices are distinct
    $indices = $lpLines | ForEach-Object { ($_ -split ':')[0] }
    $uniqueIndices = $indices | Sort-Object -Unique
    if ($uniqueIndices.Count -eq $lpLines.Count) {
        Write-Pass "list-panes -F shows distinct pane entries ($($lpLines.Count) panes)"
    } else {
        Write-Fail "list-panes -F has duplicate pane indices: $lpStr"
    }
} else {
    Write-Fail "Expected 2+ pane entries, got '$lpStr'"
}

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "21. MODIFIER CHAINING"
Write-Host ("=" * 70)

Write-Test "#{s/pwsh/TERM/;=4:window_name} chains sub+truncate"
$v = Fmt '#{s/pwsh/TERM/;=4:window_name}'
if ($v -eq "TERM") { Write-Pass "chained modifiers = '$v'" } else { Write-Fail "Expected 'TERM', got '$v'" }

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "22. OPTION VARIABLES VIA FORMAT"
Write-Host ("=" * 70)

Write-Test "#{prefix} resolves to C-b"
$v = Fmt '#{prefix}'
if ($v -eq "C-b") { Write-Pass "prefix = $v" } else { Write-Fail "Expected 'C-b', got '$v'" }

Write-Test "#{mouse} resolves"
$v = Fmt '#{mouse}'
if ($v -eq "on" -or $v -eq "off") { Write-Pass "mouse = $v" } else { Write-Fail "Expected 'on' or 'off', got '$v'" }

Write-Test "#{history_limit} resolves"
$v = Fmt '#{history_limit}'
if ($v -match '^\d+$') { Write-Pass "history_limit = $v" } else { Write-Fail "Expected numeric, got '$v'" }

# ============================================================
# Cleanup
Write-Host ""
Write-Host ("=" * 70)
Write-Host "CLEANUP"
Write-Host ("=" * 70)

& $PSMUX kill-session -t $SESSION 2>$null
Start-Sleep -Seconds 1

# ============================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "FORMAT ENGINE TEST SUMMARY"
Write-Host ("=" * 70)
Write-Host "Passed: $script:TestsPassed" -ForegroundColor Green
Write-Host "Failed: $script:TestsFailed" -ForegroundColor Red
Write-Host "Total:  $($script:TestsPassed + $script:TestsFailed)"

if ($script:TestsFailed -gt 0) {
    Write-Host "`nSOME TESTS FAILED" -ForegroundColor Red
    exit 1
} else {
    Write-Host "`nALL TESTS PASSED!" -ForegroundColor Green
    exit 0
}
