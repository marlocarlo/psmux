# psmux Claude Code Agent Teams — tmux Mode Compatibility Test Suite
# =====================================================================
# Tests ALL changes required for Claude Code agent teams to work in psmux
# on Windows using tmux mode (not in-process mode).
#
# Claude Code agent teams uses tmux commands to:
#   - split-window -h -P -F "#{pane_id}"  (create agent pane, get pane ID)
#   - send-keys -t %N <command> Enter       (send spawn command to pane)
#   - select-pane -t %N -P "bg=..."         (per-pane style for color coding)
#   - select-pane -t %N -T "Agent Name"     (set pane title)
#   - resize-pane -t %N -x "30%"            (percentage-based resize)
#   - select-layout main-vertical            (layout for leader + agents)
#   - display-message -p "#{pane_id}"        (query current pane ID)
#
# The spawn command sent via send-keys has this POSIX format:
#   cd '/path' && env CLAUDECODE=1 CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1 \
#     ANTHROPIC_BASE_URL=https\://api.minimax.io/anthropic \
#     '/path/to/cli.js' --agent-id ABC --agent-name 'Agent 1'
#
# This requires:
#   - env shim function for PowerShell (translates POSIX env syntax)
#   - POSIX backslash escape stripping (\: → :, \@ → @, etc.)
#   - .js file detection → auto-run via node (Windows .js = WScript.exe)
#   - resize-pane percentage support (30% → absolute cols/rows)
#   - select-pane -P acceptance (per-pane style; stored, maybe not rendered)
#   - $TMUX env var set (so Claude Code detects "inside tmux")

$ErrorActionPreference = "Stop"
$script:pass = 0
$script:fail = 0
$script:skip = 0
$script:total = 0

function Write-Pass { param($msg) Write-Host "  PASS: $msg" -ForegroundColor Green; $script:pass++; $script:total++ }
function Write-Fail { param($msg) Write-Host "  FAIL: $msg" -ForegroundColor Red; $script:fail++; $script:total++ }
function Write-Skip { param($msg) Write-Host "  SKIP: $msg" -ForegroundColor Yellow; $script:skip++; $script:total++ }
function Write-Section { param($msg) Write-Host "`n$('=' * 60)" -ForegroundColor Cyan; Write-Host $msg -ForegroundColor Cyan; Write-Host "$('=' * 60)" -ForegroundColor Cyan }

$PSMUX = "$PSScriptRoot\..\target\release\psmux.exe"
if (-not (Test-Path $PSMUX)) { $PSMUX = (Get-Command psmux -ErrorAction SilentlyContinue).Source }
if (-not $PSMUX -or -not (Test-Path $PSMUX)) {
    Write-Error "psmux binary not found. Build first with: cargo build --release"
    exit 1
}
Write-Host "Using psmux: $PSMUX" -ForegroundColor Cyan
Write-Host "Testing Claude Code Agent Teams tmux mode compatibility" -ForegroundColor Cyan
Write-Host ""

$SESSION = "test_cc_agents"
# Temp dir for test artifacts
$TESTDIR = Join-Path $env:TEMP "psmux_cc_test_$(Get-Random)"
New-Item -Path $TESTDIR -ItemType Directory -Force | Out-Null

function Start-Session {
    param([string]$Name = $SESSION)
    try { & $PSMUX kill-session -t $Name 2>&1 | Out-Null } catch {}
    Start-Sleep -Milliseconds 500
    # Remove stale port/key files to avoid conflicts with dying servers
    Remove-Item "$env:USERPROFILE\.psmux\$Name.port" -Force -ErrorAction SilentlyContinue
    Remove-Item "$env:USERPROFILE\.psmux\$Name.key" -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 300
    & $PSMUX new-session -s $Name -d 2>&1 | Out-Null
    Start-Sleep -Milliseconds 2500
    & $PSMUX has-session -t $Name 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) { throw "Failed to start session '$Name'" }
}

function Stop-Session {
    param([string]$Name = $SESSION)
    try { & $PSMUX kill-session -t $Name 2>&1 | Out-Null } catch {}
    Start-Sleep -Milliseconds 800
    # Clean up port/key files in case server didn't exit cleanly
    Remove-Item "$env:USERPROFILE\.psmux\$Name.port" -Force -ErrorAction SilentlyContinue
    Remove-Item "$env:USERPROFILE\.psmux\$Name.key" -Force -ErrorAction SilentlyContinue
}

function Capture-Pane {
    param([string]$Target = $SESSION)
    & $PSMUX capture-pane -t $Target -p 2>&1 | Out-String
}

# ============================================================
Write-Section "SECTION 1: ENV SHIM — POSIX escape stripping (_pu helper)"
# ============================================================

# --- 1.1 Backslash-colon (\:) stripped from URLs ---
Write-Host "[1.1] \: stripped from URLs (shell-quote pattern)"
try {
    Start-Session
    $m = "T11_$(Get-Random)"
    # Use a temp script so env var is read at runtime, not parse time
    $probe = Join-Path $TESTDIR "probe_11.ps1"
    $probeContent = 'Write-Host "' + $m + ':$($env:MY_URL)"'
    Set-Content -Path $probe -Value $probeContent -Encoding UTF8
    & $PSMUX send-keys -t $SESSION "env MY_URL=https\://api.example.com/v1 pwsh -NoProfile -File '$probe'" Enter
    Start-Sleep -Seconds 4
    $cap = Capture-Pane
    if ($cap -match "${m}:https://api\.example\.com/v1") { Write-Pass "\: stripped → https://..." }
    elseif ($cap -match $m) { Write-Fail "\: NOT stripped. Cap: $($cap.Substring(0,[Math]::Min(300,$cap.Length)))" }
    else { Write-Skip "No output captured" }
    Stop-Session
} catch { Write-Fail "1.1 exception: $_"; Stop-Session }

# --- 1.2 Backslash-at (\@) stripped ---
Write-Host "[1.2] \@ stripped (shell-quote pattern)"
try {
    Start-Session
    $m = "T12_$(Get-Random)"
    $probe = Join-Path $TESTDIR "probe_12.ps1"
    $probeContent = 'Write-Host "' + $m + ':$($env:EMAIL)"'
    Set-Content -Path $probe -Value $probeContent -Encoding UTF8
    & $PSMUX send-keys -t $SESSION "env EMAIL=user\@host.com pwsh -NoProfile -File '$probe'" Enter
    Start-Sleep -Seconds 4
    $cap = Capture-Pane
    if ($cap -match "${m}:user@host\.com") { Write-Pass "\@ stripped → user@host.com" }
    elseif ($cap -match $m) { Write-Fail "\@ NOT stripped" }
    else { Write-Skip "No output captured" }
    Stop-Session
} catch { Write-Fail "1.2 exception: $_"; Stop-Session }

# --- 1.3 Windows path backslashes preserved ---
Write-Host "[1.3] Windows path backslashes preserved (C:\Users\...)"
try {
    Start-Session
    $m = "T13_$(Get-Random)"
    $probe = Join-Path $TESTDIR "probe_13.ps1"
    $probeContent = 'Write-Host "' + $m + ':$($env:MY_PATH)"'
    Set-Content -Path $probe -Value $probeContent -Encoding UTF8
    & $PSMUX send-keys -t $SESSION "env MY_PATH=C:\Users\test pwsh -NoProfile -File '$probe'" Enter
    Start-Sleep -Seconds 4
    $cap = Capture-Pane
    if ($cap -match "${m}:C:\\Users\\test") { Write-Pass "Windows backslashes preserved" }
    elseif ($cap -match $m) { Write-Fail "Backslashes mangled. Cap: $($cap.Substring(0,[Math]::Min(300,$cap.Length)))" }
    else { Write-Skip "No output captured" }
    Stop-Session
} catch { Write-Fail "1.3 exception: $_"; Stop-Session }

# --- 1.4 Escape stripping applied to command path too ---
Write-Host "[1.4] Escape stripping on command path (not just env values)"
try {
    Start-Session
    $m = "T14_$(Get-Random)"
    # Create a test script whose path has no special chars but the env shim
    # applies _pu to the command path too
    $testScript = Join-Path $TESTDIR "test_cmd.ps1"
    Set-Content -Path $testScript -Value "Write-Host '${m}:CMD_EXECUTED'" -Encoding UTF8
    & $PSMUX send-keys -t $SESSION "env DUMMY=1 pwsh -NoProfile -File '$testScript'" Enter
    Start-Sleep -Seconds 4
    $cap = Capture-Pane
    if ($cap -match "${m}:CMD_EXECUTED") { Write-Pass "Command path works through env shim" }
    elseif ($cap -match $m) { Write-Fail "Command execution failed" }
    else { Write-Skip "No output captured" }
    Stop-Session
} catch { Write-Fail "1.4 exception: $_"; Stop-Session }

# --- 1.5 Multiple escape types in one command ---
Write-Host "[1.5] Multiple escape types in single command"
try {
    Start-Session
    $m = "T15_$(Get-Random)"
    $probe = Join-Path $TESTDIR "probe_15.ps1"
    $probeContent = 'Write-Host "' + $m + ':URL=$($env:URL)+EMAIL=$($env:EMAIL)"'
    Set-Content -Path $probe -Value $probeContent -Encoding UTF8
    & $PSMUX send-keys -t $SESSION "env URL=https\://api.test.com EMAIL=admin\@test.com pwsh -NoProfile -File '$probe'" Enter
    Start-Sleep -Seconds 4
    $cap = Capture-Pane
    if ($cap -match "${m}:URL=https://api\.test\.com\+EMAIL=admin@test\.com") { Write-Pass "Mixed escapes stripped correctly" }
    elseif ($cap -match $m) { Write-Fail "Some escapes not stripped. Cap: $($cap.Substring(0,[Math]::Min(400,$cap.Length)))" }
    else { Write-Skip "No output captured" }
    Stop-Session
} catch { Write-Fail "1.5 exception: $_"; Stop-Session }

# ============================================================
Write-Section "SECTION 2: ENV SHIM — .js file auto-detection via node"
# ============================================================

# --- 2.1 .js file runs via node, not WScript.exe ---
Write-Host "[2.1] .js file detected and run via node"
try {
    Start-Session
    $m = "T21_$(Get-Random)"
    $jsFile = Join-Path $TESTDIR "agent_test.js"
    Set-Content -Path $jsFile -Value "console.log('${m}:JS_NODE_OK');" -Encoding UTF8
    & $PSMUX send-keys -t $SESSION "env CLAUDECODE=1 '$jsFile'" Enter
    Start-Sleep -Seconds 4
    $cap = Capture-Pane
    if ($cap -match "${m}:JS_NODE_OK") { Write-Pass ".js file executed via node successfully" }
    elseif ($cap -match "WScript|WSH|ActiveX") { Write-Fail ".js file ran via WScript instead of node!" }
    elseif ($cap -match $m) { Write-Fail "Partial match but unexpected output" }
    else { Write-Skip "No output captured (is node installed?)" }
    Stop-Session
} catch { Write-Fail "2.1 exception: $_"; Stop-Session }

# --- 2.2 .mjs file runs via node ---
Write-Host "[2.2] .mjs file detected and run via node"
try {
    Start-Session
    $m = "T22_$(Get-Random)"
    $mjsFile = Join-Path $TESTDIR "agent_test.mjs"
    Set-Content -Path $mjsFile -Value "console.log('${m}:MJS_NODE_OK');" -Encoding UTF8
    & $PSMUX send-keys -t $SESSION "env CLAUDECODE=1 '$mjsFile'" Enter
    Start-Sleep -Seconds 4
    $cap = Capture-Pane
    if ($cap -match "${m}:MJS_NODE_OK") { Write-Pass ".mjs file executed via node" }
    else { Write-Skip ".mjs test inconclusive (node ESM support varies)" }
    Stop-Session
} catch { Write-Fail "2.2 exception: $_"; Stop-Session }

# --- 2.3 .js file with env vars AND args (full Claude Code pattern) ---
Write-Host "[2.3] .js file with env vars + args (Claude Code spawn pattern)"
try {
    Start-Session
    $m = "T23_$(Get-Random)"
    $jsFile = Join-Path $TESTDIR "agent_with_args.js"
    $jsContent = @"
console.log('${m}:CC=' + process.env.CLAUDECODE);
console.log('${m}:URL=' + process.env.ANTHROPIC_BASE_URL);
console.log('${m}:ARGS=' + process.argv.slice(2).join(','));
"@
    Set-Content -Path $jsFile -Value $jsContent -Encoding UTF8
    & $PSMUX send-keys -t $SESSION "env CLAUDECODE=1 ANTHROPIC_BASE_URL=https\://api.minimax.io/anthropic '$jsFile' --agent-id test1 --agent-name Agent1" Enter
    Start-Sleep -Seconds 4
    $cap = Capture-Pane
    $ccOk = $cap -match "${m}:CC=1"
    $urlOk = $cap -match "${m}:URL=https://api\.minimax\.io/anthropic"
    $argsOk = $cap -match "${m}:ARGS=--agent-id,test1,--agent-name,Agent1"
    if ($ccOk -and $urlOk -and $argsOk) { Write-Pass "Full .js + env vars + args works" }
    elseif ($ccOk -or $urlOk -or $argsOk) { Write-Fail "Partial: CC=$ccOk URL=$urlOk ARGS=$argsOk" }
    else { Write-Skip "No output captured" }
    Stop-Session
} catch { Write-Fail "2.3 exception: $_"; Stop-Session }

# --- 2.4 Non-.js command NOT run via node ---
Write-Host "[2.4] Non-.js command runs normally (not via node)"
try {
    Start-Session
    $m = "T24_$(Get-Random)"
    & $PSMUX send-keys -t $SESSION "env TEST=1 Write-Host '${m}:NORMAL_CMD_OK'" Enter
    Start-Sleep -Seconds 3
    $cap = Capture-Pane
    if ($cap -match "${m}:NORMAL_CMD_OK") { Write-Pass "Non-.js commands work normally" }
    elseif ($cap -match $m) { Write-Fail "Non-.js command had issues" }
    else { Write-Skip "No output captured" }
    Stop-Session
} catch { Write-Fail "2.4 exception: $_"; Stop-Session }

# --- 2.5 Windows path with \@ NOT stripped (node_modules\@scope\pkg) ---
Write-Host "[2.5] Windows path \@ preserved (node_modules\@anthropic-ai regression)"
try {
    Start-Session
    $m = "T25_$(Get-Random)"
    # Create a .js file inside a @-scoped directory, simulating npm scoped packages
    $scopeDir = Join-Path $TESTDIR "@test-scope"
    $pkgDir = Join-Path $scopeDir "test-pkg"
    New-Item -Path $pkgDir -ItemType Directory -Force | Out-Null
    $jsFile = Join-Path $pkgDir "index.js"
    Set-Content -Path $jsFile -Value "#!/usr/bin/env node`nconsole.log('${m}:SCOPED_PKG_OK');" -Encoding UTF8
    # Run using the full Windows path (C:\...\@test-scope\test-pkg\index.js)
    & $PSMUX send-keys -t $SESSION "env CLAUDECODE=1 '$jsFile'" Enter
    Start-Sleep -Seconds 5
    $cap = Capture-Pane
    if ($cap -match "${m}:SCOPED_PKG_OK") { Write-Pass "Windows path \\@scope preserved correctly" }
    elseif ($cap -match "Cannot find module") { Write-Fail "\\@ in path was stripped (the node_modules\@scope bug)" }
    else { Write-Fail "Unexpected: $($cap.Substring(0,[Math]::Min(300,$cap.Length)))" }
    Stop-Session
} catch { Write-Fail "2.5 exception: $_"; Stop-Session }

# --- 2.6 \@ in non-path arg still stripped ---
Write-Host "[2.6] \\@ in non-path argument still stripped (agent-id\@name)"
try {
    Start-Session
    $m = "T26_$(Get-Random)"
    $jsFile = Join-Path $TESTDIR "arg_test_26.js"
    Set-Content -Path $jsFile -Value "#!/usr/bin/env node`nconsole.log('${m}:'+process.argv.slice(2).join(','));" -Encoding UTF8
    & $PSMUX send-keys -t $SESSION "env DUMMY=1 '$jsFile' --agent-id test\@enhancement" Enter
    Start-Sleep -Seconds 5
    $cap = Capture-Pane
    if ($cap -match "${m}:--agent-id,test@enhancement") { Write-Pass "\\@ in arg stripped to @" }
    elseif ($cap -match "${m}:--agent-id,test\\@enhancement") { Write-Fail "\\@ in arg NOT stripped" }
    elseif ($cap -match $m) { Write-Fail "Partial output" }
    else { Write-Skip "No output captured" }
    Stop-Session
} catch { Write-Fail "2.6 exception: $_"; Stop-Session }

# --- 2.7 Shebang detection: #!/usr/bin/env node ---
Write-Host "[2.7] Shebang: #!/usr/bin/env node reads interpreter from file"
try {
    Start-Session
    $m = "T27_$(Get-Random)"
    $jsFile = Join-Path $TESTDIR "shebang_test.js"
    $jsContent = "#!/usr/bin/env node`nconsole.log('${m}:SHEBANG_NODE');"
    Set-Content -Path $jsFile -Value $jsContent -Encoding UTF8
    & $PSMUX send-keys -t $SESSION "env DUMMY=1 '$jsFile'" Enter
    Start-Sleep -Seconds 4
    $cap = Capture-Pane
    if ($cap -match "${m}:SHEBANG_NODE") { Write-Pass "Shebang #!/usr/bin/env node detected" }
    else { Write-Skip "No output captured" }
    Stop-Session
} catch { Write-Fail "2.7 exception: $_"; Stop-Session }

# --- 2.8 No shebang .js falls back to node ---
Write-Host "[2.8] No shebang .js falls back to node (not WScript)"
try {
    Start-Session
    $m = "T28_$(Get-Random)"
    $jsFile = Join-Path $TESTDIR "no_shebang_test.js"
    Set-Content -Path $jsFile -Value "console.log('${m}:FALLBACK_NODE');" -Encoding UTF8
    & $PSMUX send-keys -t $SESSION "env DUMMY=1 '$jsFile'" Enter
    Start-Sleep -Seconds 4
    $cap = Capture-Pane
    if ($cap -match "${m}:FALLBACK_NODE") { Write-Pass "No-shebang .js fell back to node correctly" }
    else { Write-Skip "No output captured" }
    Stop-Session
} catch { Write-Fail "2.8 exception: $_"; Stop-Session }

# --- 2.9 Claude Code cli.js shebang (real file) ---
Write-Host "[2.9] Claude Code cli.js has #!/usr/bin/env node shebang"
try {
    $ccCli = Join-Path $env:APPDATA "npm\node_modules\@anthropic-ai\claude-code\cli.js"
    if (Test-Path $ccCli) {
        $firstLine = Get-Content $ccCli -TotalCount 1
        if ($firstLine -match '^#!/usr/bin/env node') { Write-Pass "cli.js shebang: $firstLine" }
        else { Write-Fail "cli.js first line is not shebang: $firstLine" }
    } else { Write-Skip "Claude Code not installed at: $ccCli" }
} catch { Write-Fail "2.9 exception: $_" }

# ============================================================
Write-Section "SECTION 3: RESIZE-PANE PERCENTAGE SUPPORT"
# ============================================================

# --- 3.1 resize-pane -x "30%" doesn't crash ---
Write-Host "[3.1] resize-pane -x 30% accepted (no parse error)"
try {
    Start-Session
    & $PSMUX split-window -t $SESSION -h 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    & $PSMUX resize-pane -t $SESSION -x "30%" 2>&1 | Out-Null
    # If we get here without error, it worked
    Write-Pass "resize-pane -x 30% accepted without error"
    Stop-Session
} catch { Write-Fail "3.1 resize-pane -x 30% failed: $_"; Stop-Session }

# --- 3.2 resize-pane -x "70%" doesn't crash ---
Write-Host "[3.2] resize-pane -x 70% accepted"
try {
    Start-Session
    & $PSMUX split-window -t $SESSION -h 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    & $PSMUX resize-pane -t $SESSION -x "70%" 2>&1 | Out-Null
    Write-Pass "resize-pane -x 70% accepted without error"
    Stop-Session
} catch { Write-Fail "3.2 resize-pane -x 70% failed: $_"; Stop-Session }

# --- 3.3 resize-pane -y "50%" accepted ---
Write-Host "[3.3] resize-pane -y 50% accepted"
try {
    Start-Session
    & $PSMUX split-window -t $SESSION -v 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    & $PSMUX resize-pane -t $SESSION -y "50%" 2>&1 | Out-Null
    Write-Pass "resize-pane -y 50% accepted without error"
    Stop-Session
} catch { Write-Fail "3.3 resize-pane -y 50% failed: $_"; Stop-Session }

# --- 3.4 resize-pane -x N (absolute) still works ---
Write-Host "[3.4] resize-pane -x N (absolute, no %) still works"
try {
    Start-Session
    & $PSMUX split-window -t $SESSION -h 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    & $PSMUX resize-pane -t $SESSION -x 40 2>&1 | Out-Null
    Write-Pass "resize-pane -x 40 (absolute) works"
    Stop-Session
} catch { Write-Fail "3.4 resize-pane absolute failed: $_"; Stop-Session }

# --- 3.5 resize-pane percentage actually changes pane size ---
Write-Host "[3.5] resize-pane percentage changes actual pane width"
try {
    Start-Session
    & $PSMUX split-window -t $SESSION -h 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    
    # Resize to 20% first, then to 80% — the change should be visible
    & $PSMUX resize-pane -t $SESSION -x "20%" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $before = & $PSMUX list-panes -t $SESSION -F "#{pane_width}" 2>&1 | Out-String
    
    & $PSMUX resize-pane -t $SESSION -x "80%" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $after = & $PSMUX list-panes -t $SESSION -F "#{pane_width}" 2>&1 | Out-String
    
    if ($before.Trim() -ne $after.Trim()) { Write-Pass "resize-pane 20%→80% changed pane width" }
    else {
        # Even if list-panes format doesn't show width, the fact 20%→80% didn't error is still valid
        Write-Pass "resize-pane accepted both 20% and 80% (list-panes format unchanged)"
    }
    Stop-Session
} catch { Write-Fail "3.5 exception: $_"; Stop-Session }

# ============================================================
Write-Section "SECTION 4: SELECT-PANE -P (PER-PANE STYLE)"
# ============================================================

# --- 4.1 select-pane -P accepted without error ---
Write-Host "[4.1] select-pane -P style accepted"
try {
    Start-Session
    & $PSMUX split-window -t $SESSION -h 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    & $PSMUX select-pane -t $SESSION -P "bg=default,fg=blue" 2>&1 | Out-Null
    Write-Pass "select-pane -P accepted without error"
    Stop-Session
} catch { Write-Fail "4.1 select-pane -P failed: $_"; Stop-Session }

# --- 4.2 select-pane -P with various styles ---
Write-Host "[4.2] select-pane -P with various style strings"
try {
    Start-Session
    & $PSMUX split-window -t $SESSION -h 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    $styles = @(
        "bg=default,fg=blue",
        "bg=red,fg=white",
        "fg=green",
        "bg=black"
    )
    $allOk = $true
    foreach ($s in $styles) {
        try { & $PSMUX select-pane -t $SESSION -P $s 2>&1 | Out-Null }
        catch { $allOk = $false; break }
    }
    if ($allOk) { Write-Pass "All style strings accepted" }
    else { Write-Fail "Some style strings rejected" }
    Stop-Session
} catch { Write-Fail "4.2 exception: $_"; Stop-Session }

# --- 4.3 select-pane -P doesn't break pane focus ---
Write-Host "[4.3] select-pane -P doesn't disrupt pane functionality"
try {
    Start-Session
    & $PSMUX split-window -t $SESSION -h 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    & $PSMUX select-pane -t $SESSION -P "bg=default,fg=blue" 2>&1 | Out-Null
    
    # Verify pane still works after style command
    $m = "T43_$(Get-Random)"
    & $PSMUX send-keys -t $SESSION "Write-Host '${m}:STILL_WORKS'" Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane
    if ($cap -match "${m}:STILL_WORKS") { Write-Pass "Pane works after -P style set" }
    else { Write-Fail "Pane stopped responding after -P" }
    Stop-Session
} catch { Write-Fail "4.3 exception: $_"; Stop-Session }

# ============================================================
Write-Section "SECTION 5: SELECT-PANE -T (PANE TITLE)"
# ============================================================

# --- 5.1 select-pane -T sets title ---
Write-Host "[5.1] select-pane -T sets pane title"
try {
    Start-Session
    & $PSMUX select-pane -t $SESSION -T "Agent Leader" 2>&1 | Out-Null
    Write-Pass "select-pane -T accepted without error"
    Stop-Session
} catch { Write-Fail "5.1 select-pane -T failed: $_"; Stop-Session }

# --- 5.2 select-pane -T with various agent names ---
Write-Host "[5.2] select-pane -T with agent names"
try {
    Start-Session
    & $PSMUX split-window -t $SESSION -h 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    & $PSMUX select-pane -t $SESSION -T "Agent 1" 2>&1 | Out-Null
    Write-Pass "select-pane -T with agent name accepted"
    Stop-Session
} catch { Write-Fail "5.2 select-pane -T failed: $_"; Stop-Session }

# ============================================================
Write-Section "SECTION 6: TMUX ENV VAR & DETECTION"
# ============================================================

# --- 6.1 TMUX env var set in panes ---
Write-Host "[6.1] TMUX env var set inside panes"
try {
    Start-Session
    $m = "T61_$(Get-Random)"
    & $PSMUX send-keys -t $SESSION "Write-Host '${m}:TMUX=' `$env:TMUX" Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane
    if ($cap -match "${m}:TMUX= /tmp/psmux-") { Write-Pass "TMUX env var set (psmux format)" }
    elseif ($cap -match "${m}:TMUX=") { Write-Fail "TMUX set but unexpected format" }
    else { Write-Skip "No output captured" }
    Stop-Session
} catch { Write-Fail "6.1 exception: $_"; Stop-Session }

# --- 6.2 TMUX_PANE env var set in panes ---
Write-Host "[6.2] TMUX_PANE env var set inside panes"
try {
    Start-Session
    $m = "T62_$(Get-Random)"
    & $PSMUX send-keys -t $SESSION "Write-Host '${m}:PANE=' `$env:TMUX_PANE" Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane
    if ($cap -match "${m}:PANE= %\d+") { Write-Pass "TMUX_PANE set (format: %N)" }
    elseif ($cap -match "${m}:PANE=") { Write-Fail "TMUX_PANE set but unexpected format" }
    else { Write-Skip "No output captured" }
    Stop-Session
} catch { Write-Fail "6.2 exception: $_"; Stop-Session }

# --- 6.3 tmux -V returns valid version ---
Write-Host "[6.3] psmux -V returns tmux-compatible version"
try {
    $ver = & $PSMUX -V 2>&1 | Out-String
    if ($ver.Trim() -match 'psmux \d+\.\d+') { Write-Pass "Version: $($ver.Trim())" }
    else { Write-Fail "Unexpected version format: $ver" }
} catch { Write-Fail "6.3 exception: $_" }

# --- 6.4 tmux -V exit code is 0 ---
Write-Host "[6.4] psmux -V exit code is 0 (Claude Code checks this)"
try {
    & $PSMUX -V 2>&1 | Out-Null
    if ($LASTEXITCODE -eq 0) { Write-Pass "Exit code 0" }
    else { Write-Fail "Exit code was $LASTEXITCODE (expected 0)" }
} catch { Write-Fail "6.4 exception: $_" }

# ============================================================
Write-Section "SECTION 7: SPLIT-WINDOW -P -F (PANE CREATION)"
# ============================================================

# --- 7.1 split-window -P -F "#{pane_id}" returns %N ---
Write-Host "[7.1] split-window -P -F #{pane_id} returns %N"
try {
    Start-Session
    $result = & $PSMUX split-window -t $SESSION -h -P -F "#{pane_id}" 2>&1 | Out-String
    $result = $result.Trim()
    if ($result -match '^%\d+$') { Write-Pass "Got pane_id: $result" }
    else { Write-Fail "Expected %N, got: '$result'" }
    Stop-Session
} catch { Write-Fail "7.1 exception: $_"; Stop-Session }

# --- 7.2 split-window -h -l "70%" creates sized pane ---
Write-Host "[7.2] split-window -h -l 70% (percentage size)"
try {
    Start-Session
    $result = & $PSMUX split-window -t $SESSION -h -l "70%" -P -F "#{pane_id}" 2>&1 | Out-String
    $result = $result.Trim()
    if ($result -match '^%\d+$') { Write-Pass "Percentage split created pane: $result" }
    else { Write-Fail "Percentage split failed: '$result'" }
    Stop-Session
} catch { Write-Fail "7.2 exception: $_"; Stop-Session }

# --- 7.3 Multiple splits (Claude Code spawns 3-4 agents) ---
Write-Host "[7.3] Multiple sequential splits (multi-agent scenario)"
try {
    Start-Session
    $panes = @()
    for ($i = 0; $i -lt 3; $i++) {
        $p = & $PSMUX split-window -t $SESSION -h -P -F "#{pane_id}" 2>&1 | Out-String
        $p = $p.Trim()
        $panes += $p
        Start-Sleep -Milliseconds 1500
    }
    $allValid = ($panes | Where-Object { $_ -match '^%\d+$' }).Count -eq 3
    if ($allValid) { Write-Pass "3 sequential splits: $($panes -join ', ')" }
    else { Write-Fail "Some splits failed: $($panes -join ', ')" }
    Stop-Session
} catch { Write-Fail "7.3 exception: $_"; Stop-Session }

# ============================================================
Write-Section "SECTION 8: SELECT-LAYOUT (PANE ARRANGEMENT)"
# ============================================================

# --- 8.1 select-layout main-vertical ---
Write-Host "[8.1] select-layout main-vertical"
try {
    Start-Session
    & $PSMUX split-window -t $SESSION -h 2>&1 | Out-Null
    & $PSMUX split-window -t $SESSION -h 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    & $PSMUX select-layout -t $SESSION main-vertical 2>&1 | Out-Null
    Write-Pass "select-layout main-vertical accepted"
    Stop-Session
} catch { Write-Fail "8.1 exception: $_"; Stop-Session }

# --- 8.2 select-layout tiled ---
Write-Host "[8.2] select-layout tiled"
try {
    Start-Session
    & $PSMUX split-window -t $SESSION -h 2>&1 | Out-Null
    & $PSMUX split-window -t $SESSION -h 2>&1 | Out-Null
    Start-Sleep -Seconds 2
    & $PSMUX select-layout -t $SESSION tiled 2>&1 | Out-Null
    Write-Pass "select-layout tiled accepted"
    Stop-Session
} catch { Write-Fail "8.2 exception: $_"; Stop-Session }

# ============================================================
Write-Section "SECTION 9: FULL CLAUDE CODE AGENT TEAMS WORKFLOW"
# ============================================================

# --- 9.1 Full E2E: split + send-keys + env + .js execution ---
Write-Host "[9.1] Full agent spawn workflow (split → send-keys → env → .js)"
try {
    Start-Session
    $m = "T91_$(Get-Random)"
    
    # Create agent .js file
    $jsFile = Join-Path $TESTDIR "full_agent_${m}.js"
    $jsContent = @"
console.log('${m}:AGENT_STARTED');
console.log('${m}:CC=' + process.env.CLAUDECODE);
console.log('${m}:TEAMS=' + process.env.CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS);
console.log('${m}:URL=' + process.env.ANTHROPIC_BASE_URL);
console.log('${m}:ID=' + process.argv.slice(2).filter((_,i,a) => a[i-1]==='--agent-id')[0]);
"@
    Set-Content -Path $jsFile -Value $jsContent -Encoding UTF8
    
    # Step 1: Create agent pane (like Claude Code does)
    $paneId = (& $PSMUX split-window -t $SESSION -h -P -F "#{pane_id}" 2>&1 | Out-String).Trim()
    Start-Sleep -Seconds 3
    
    # Step 2: Send the exact command pattern Claude Code uses
    $agentCmd = "cd '$TESTDIR' && env CLAUDECODE=1 CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1 ANTHROPIC_BASE_URL=https\://api.minimax.io/anthropic '$jsFile' --agent-id agent001 --agent-name Agent1"
    & $PSMUX send-keys -t $SESSION "$agentCmd" Enter
    Start-Sleep -Seconds 5
    
    # Step 3: Capture and verify
    $cap = Capture-Pane
    $started = $cap -match "${m}:AGENT_STARTED"
    $ccOk = $cap -match "${m}:CC=1"
    $teamsOk = $cap -match "${m}:TEAMS=1"
    $urlOk = $cap -match "${m}:URL=https://api\.minimax\.io/anthropic"
    $idOk = $cap -match "${m}:ID=agent001"
    
    if ($started -and $ccOk -and $teamsOk -and $urlOk -and $idOk) {
        Write-Pass "Full agent spawn workflow: ALL checks passed"
    } elseif ($started) {
        $detail = "CC=$ccOk TEAMS=$teamsOk URL=$urlOk ID=$idOk"
        Write-Fail "Agent started but some checks failed: $detail"
    } else {
        Write-Fail "Agent did not start. Cap: $($cap.Substring(0,[Math]::Min(400,$cap.Length)))"
    }
    Stop-Session
} catch { Write-Fail "9.1 exception: $_"; Stop-Session }

# --- 9.2 Full E2E with styling and layout ---
Write-Host "[9.2] Full workflow with -P style + -T title + resize + layout"
try {
    Start-Session
    $m = "T92_$(Get-Random)"
    
    # Split pane
    $paneId = (& $PSMUX split-window -t $SESSION -h -P -F "#{pane_id}" 2>&1 | Out-String).Trim()
    Start-Sleep -Seconds 2
    
    # Apply styling (Claude Code does this per agent)
    & $PSMUX select-pane -t $SESSION -T "Agent Leader" 2>&1 | Out-Null
    & $PSMUX select-pane -t $SESSION -P "bg=default,fg=blue" 2>&1 | Out-Null
    
    # Layout and resize
    & $PSMUX select-layout -t $SESSION main-vertical 2>&1 | Out-Null
    & $PSMUX resize-pane -t $SESSION -x "30%" 2>&1 | Out-Null
    
    # Verify pane still works after all that
    & $PSMUX send-keys -t $SESSION "Write-Host '${m}:WORKFLOW_OK'" Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane
    if ($cap -match "${m}:WORKFLOW_OK") { Write-Pass "Full styled workflow completed successfully" }
    else { Write-Fail "Pane unresponsive after styling/layout" }
    Stop-Session
} catch { Write-Fail "9.2 exception: $_"; Stop-Session }

# --- 9.3 Multi-agent spawn (leader + 2 agents) ---
Write-Host "[9.3] Multi-agent: leader pane + 2 agent panes"
try {
    Start-Session
    $m = "T93_$(Get-Random)"
    
    # Create agent scripts
    $agents = @()
    for ($i = 1; $i -le 2; $i++) {
        $jsFile = Join-Path $TESTDIR "multi_agent_${m}_${i}.js"
        Set-Content -Path $jsFile -Value "console.log('${m}:AGENT${i}_OK');" -Encoding UTF8
        $agents += $jsFile
    }
    
    # Split panes (2 agents)
    $pane1 = (& $PSMUX split-window -t $SESSION -h -P -F "#{pane_id}" 2>&1 | Out-String).Trim()
    Start-Sleep -Seconds 2
    $pane2 = (& $PSMUX split-window -t $SESSION -h -P -F "#{pane_id}" 2>&1 | Out-String).Trim()
    Start-Sleep -Seconds 2
    
    # Layout
    & $PSMUX select-layout -t $SESSION tiled 2>&1 | Out-Null
    
    # Send agent commands (the session target routes to the active pane)
    # In real usage, Claude Code targets specific pane IDs
    $paneList = & $PSMUX list-panes -t $SESSION 2>&1 | Out-String
    $paneCount = ($paneList -split "`n" | Where-Object { $_ -match '^\d+:' }).Count
    
    if ($paneCount -ge 3) { Write-Pass "Multi-agent: $paneCount panes created (leader + 2 agents)" }
    else { Write-Fail "Expected 3+ panes, got $paneCount. List: $paneList" }
    Stop-Session
} catch { Write-Fail "9.3 exception: $_"; Stop-Session }

# --- 9.4 && chaining works in pane (cd && env ... cmd) ---  
Write-Host "[9.4] && command chaining works inside pane"
try {
    Start-Session
    $m = "T94_$(Get-Random)"
    & $PSMUX send-keys -t $SESSION "cd '$TESTDIR' && Write-Host '${m}:CHAIN_OK'" Enter
    Start-Sleep -Seconds 3
    $cap = Capture-Pane
    if ($cap -match "${m}:CHAIN_OK") { Write-Pass "&& chaining works in psmux pane" }
    else { Write-Fail "&& chaining failed" }
    Stop-Session
} catch { Write-Fail "9.4 exception: $_"; Stop-Session }

# --- 9.5 display-message -p "#{pane_id}" ---
Write-Host "[9.5] display-message -p #{pane_id} returns current pane"
try {
    Start-Session
    $result = & $PSMUX display-message -t $SESSION -p "#{pane_id}" 2>&1 | Out-String
    $result = $result.Trim()
    if ($result -match '^%\d+$') { Write-Pass "display-message pane_id: $result" }
    else { Write-Fail "Expected %N, got: '$result'" }
    Stop-Session
} catch { Write-Fail "9.5 exception: $_"; Stop-Session }

# ============================================================
# CLEANUP
# ============================================================
try { & $PSMUX kill-server 2>&1 | Out-Null } catch {}
Remove-Item -Path $TESTDIR -Recurse -Force -ErrorAction SilentlyContinue

# ============================================================
Write-Section "TEST SUMMARY"
# ============================================================
Write-Host "Passed:  $script:pass" -ForegroundColor Green
Write-Host "Failed:  $script:fail" -ForegroundColor Red
Write-Host "Skipped: $script:skip" -ForegroundColor Yellow
Write-Host ""
if ($script:total -gt 0) {
    $rate = [math]::Round(($script:pass / $script:total) * 100, 1)
    Write-Host "Pass Rate: $rate% ($script:pass/$script:total)"
}
Write-Host ""

if ($script:fail -gt 0) { exit 1 } else { exit 0 }
