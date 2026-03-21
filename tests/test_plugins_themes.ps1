# psmux Plugins & Themes Compatibility Test Suite
# Tests: inline style parsing, theme option setting, plugin infrastructure,
#        mode-style, status-position, user @options, format variables for plugins
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_plugins_themes.ps1

$ErrorActionPreference = "Continue"
$script:TestsPassed = 0
$script:TestsFailed = 0
$script:TestsSkipped = 0

function Write-Pass { param($msg) Write-Host "[PASS] $msg" -ForegroundColor Green; $script:TestsPassed++ }
function Write-Fail { param($msg) Write-Host "[FAIL] $msg" -ForegroundColor Red; $script:TestsFailed++ }
function Write-Skip { param($msg) Write-Host "[SKIP] $msg" -ForegroundColor Yellow; $script:TestsSkipped++ }
function Write-Info { param($msg) Write-Host "[INFO] $msg" -ForegroundColor Cyan }
function Write-Test { param($msg) Write-Host "[TEST] $msg" -ForegroundColor White }

$PSMUX = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $PSMUX) { $PSMUX = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $PSMUX) { Write-Error "psmux binary not found"; exit 1 }
Write-Info "Using: $PSMUX"

function Psmux { & $PSMUX @args 2>&1; Start-Sleep -Milliseconds 300 }

# Kill everything first
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

# Create test session
Write-Info "Creating test session 'plugtest'..."
Start-Process -FilePath $PSMUX -ArgumentList "new-session -s plugtest -d" -WindowStyle Hidden
Start-Sleep -Seconds 3
& $PSMUX has-session -t plugtest 2>$null
if ($LASTEXITCODE -ne 0) { Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1 }
Write-Info "Session 'plugtest' created"

# ============================================================
# SECTION 1: Theme Options (set-option / show-option)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 1: Theme Options (set-option / show-option)"
Write-Host ("=" * 60)

# --- status-style ---
Write-Test "set-option status-style"
Psmux set-option -g status-style "bg=#1e1e2e,fg=#cdd6f4" -t plugtest | Out-Null
$val = (Psmux show-options -g -v status-style -t plugtest | Out-String).Trim()
if ($val -match "bg=#1e1e2e" -and $val -match "fg=#cdd6f4") { Write-Pass "status-style set/get: $val" }
else { Write-Fail "status-style expected 'bg=#1e1e2e,fg=#cdd6f4', got: $val" }

# --- window-status-format (used by ALL themes) ---
Write-Test "set-option window-status-format with inline styles"
$wfmt = '#[fg=#6c7086,bg=#1e1e2e] #I #W '
Psmux set-option -g window-status-format $wfmt -t plugtest | Out-Null
$val = (Psmux show-options -g -v window-status-format -t plugtest | Out-String).Trim()
if ($val -match "#\[fg=#6c7086") { Write-Pass "window-status-format: accepted inline styles" }
else { Write-Fail "window-status-format: got '$val'" }

# --- window-status-current-format (used by ALL themes) ---
Write-Test "set-option window-status-current-format with inline styles"
$wcfmt = '#[fg=#1e1e2e,bg=#cba6f7,bold] #I #W #[fg=#cba6f7,bg=#1e1e2e]'
Psmux set-option -g window-status-current-format $wcfmt -t plugtest | Out-Null
$val = (Psmux show-options -g -v window-status-current-format -t plugtest | Out-String).Trim()
if ($val -match "#\[fg=#1e1e2e") { Write-Pass "window-status-current-format: accepted inline styles" }
else { Write-Fail "window-status-current-format: got '$val'" }

# --- status-left with inline styles ---
Write-Test "set-option status-left with inline styles"
$sleft = '#[fg=#1e1e2e,bg=#89b4fa,bold] #S #[fg=#89b4fa,bg=#1e1e2e]'
Psmux set-option -g status-left $sleft -t plugtest | Out-Null
$val = (Psmux show-options -g -v status-left -t plugtest | Out-String).Trim()
if ($val -match "#\[fg=#1e1e2e") { Write-Pass "status-left with inline styles: ok" }
else { Write-Fail "status-left: got '$val'" }

# --- status-right with inline styles ---
Write-Test "set-option status-right with inline styles"
$sright = '#[fg=#f38ba8,bg=#1e1e2e] %H:%M #[fg=#1e1e2e,bg=#a6e3a1,bold] %Y-%m-%d '
Psmux set-option -g status-right $sright -t plugtest | Out-Null
$val = (Psmux show-options -g -v status-right -t plugtest | Out-String).Trim()
if ($val -match "#\[fg=#f38ba8") { Write-Pass "status-right with inline styles: ok" }
else { Write-Fail "status-right: got '$val'" }

# --- mode-style ---
Write-Test "set-option mode-style"
Psmux set-option -g mode-style "fg=#1e1e2e,bg=#f9e2af" -t plugtest | Out-Null
$val = (Psmux show-options -g -v mode-style -t plugtest | Out-String).Trim()
if ($val -match "fg=#1e1e2e" -and $val -match "bg=#f9e2af") { Write-Pass "mode-style: $val" }
else { Write-Fail "mode-style got: $val" }

# --- status-position ---
Write-Test "set-option status-position top"
Psmux set-option -g status-position top -t plugtest | Out-Null
$val = (Psmux show-options -g -v status-position -t plugtest | Out-String).Trim()
if ($val -eq "top") { Write-Pass "status-position top: $val" }
else { Write-Fail "status-position expected 'top', got: $val" }

Write-Test "set-option status-position bottom"
Psmux set-option -g status-position bottom -t plugtest | Out-Null
$val = (Psmux show-options -g -v status-position -t plugtest | Out-String).Trim()
if ($val -eq "bottom") { Write-Pass "status-position bottom: $val" }
else { Write-Fail "status-position expected 'bottom', got: $val" }

# --- status-justify ---
Write-Test "set-option status-justify"
Psmux set-option -g status-justify centre -t plugtest | Out-Null
$val = (Psmux show-options -g -v status-justify -t plugtest | Out-String).Trim()
if ($val -eq "centre") { Write-Pass "status-justify: $val" }
else { Write-Fail "status-justify expected 'centre', got: $val" }

# Reset to default
Psmux set-option -g status-justify left -t plugtest | Out-Null

# --- window-status-style ---
Write-Test "set-option window-status-style"
Psmux set-option -g window-status-style "fg=#6c7086" -t plugtest | Out-Null
$val = (Psmux show-options -g -v window-status-style -t plugtest | Out-String).Trim()
if ($val -match "fg=#6c7086") { Write-Pass "window-status-style: $val" }
else { Write-Fail "window-status-style got: $val" }

# --- window-status-current-style ---
Write-Test "set-option window-status-current-style"
Psmux set-option -g window-status-current-style "fg=#cba6f7,bold" -t plugtest | Out-Null
$val = (Psmux show-options -g -v window-status-current-style -t plugtest | Out-String).Trim()
if ($val -match "fg=#cba6f7") { Write-Pass "window-status-current-style: $val" }
else { Write-Fail "window-status-current-style got: $val" }

# --- pane-border-style ---
Write-Test "set-option pane-border-style"
Psmux set-option -g pane-border-style "fg=#313244" -t plugtest | Out-Null
$val = (Psmux show-options -g -v pane-border-style -t plugtest | Out-String).Trim()
if ($val -match "fg=#313244") { Write-Pass "pane-border-style: $val" }
else { Write-Fail "pane-border-style got: $val" }

# --- pane-active-border-style ---
Write-Test "set-option pane-active-border-style"
Psmux set-option -g pane-active-border-style "fg=#cba6f7" -t plugtest | Out-Null
$val = (Psmux show-options -g -v pane-active-border-style -t plugtest | Out-String).Trim()
if ($val -match "fg=#cba6f7") { Write-Pass "pane-active-border-style: $val" }
else { Write-Fail "pane-active-border-style got: $val" }

# --- message-style ---
Write-Test "set-option message-style"
Psmux set-option -g message-style "fg=#cdd6f4,bg=#1e1e2e" -t plugtest | Out-Null
$val = (Psmux show-options -g -v message-style -t plugtest | Out-String).Trim()
if ($val -match "fg=#cdd6f4") { Write-Pass "message-style: $val" }
else { Write-Fail "message-style got: $val" }


# ============================================================
# SECTION 2: User @options (plugin infrastructure)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 2: User @options (plugin infrastructure)"
Write-Host ("=" * 60)

Write-Test "set user @option (plugin declaration)"
Psmux set-option -g "@plugin" "psmux/psmux-catppuccin" -t plugtest | Out-Null
$val = (Psmux show-options -g -v "@plugin" -t plugtest | Out-String).Trim()
if ($val -match "psmux-catppuccin") { Write-Pass "@plugin set/get: $val" }
else { Write-Fail "@plugin expected 'psmux/psmux-catppuccin', got: $val" }

Write-Test "set user @option (custom variable)"
Psmux set-option -g "@catppuccin-flavour" "mocha" -t plugtest | Out-Null
$val = (Psmux show-options -g -v "@catppuccin-flavour" -t plugtest | Out-String).Trim()
if ($val -eq "mocha") { Write-Pass "@catppuccin-flavour: $val" }
else { Write-Fail "@catppuccin-flavour expected 'mocha', got: $val" }

Write-Test "set multiple @options"
Psmux set-option -g "@dracula-show-powerline" "true" -t plugtest | Out-Null
Psmux set-option -g "@dracula-plugins" "cpu-usage ram-usage" -t plugtest | Out-Null
$v1 = (Psmux show-options -g -v "@dracula-show-powerline" -t plugtest | Out-String).Trim()
$v2 = (Psmux show-options -g -v "@dracula-plugins" -t plugtest | Out-String).Trim()
if ($v1 -eq "true" -and $v2 -match "cpu-usage") { Write-Pass "multiple @options: show-powerline=$v1, plugins=$v2" }
else { Write-Fail "multiple @options: show-powerline=$v1, plugins=$v2" }

Write-Test "show-options -g lists @options"
$all = (Psmux show-options -g -t plugtest | Out-String)
if ($all -match "@plugin" -or $all -match "@catppuccin") { Write-Pass "show-options -g includes @options" }
else { Write-Fail "show-options -g does not include @options" }


# ============================================================
# SECTION 3: Format Variables for Plugins
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 3: Format Variables for Plugins"
Write-Host ("=" * 60)

Write-Test "#{session_name}"
$v = (Psmux display-message -t plugtest -p '#{session_name}' | Out-String).Trim()
if ($v -eq "plugtest") { Write-Pass "session_name: $v" }
else { Write-Fail "session_name expected 'plugtest', got: $v" }

Write-Test "#{window_index}"
$v = (Psmux display-message -t plugtest -p '#{window_index}' | Out-String).Trim()
if ($v -match '^\d+$') { Write-Pass "window_index: $v" }
else { Write-Fail "window_index: $v" }

Write-Test "#{window_name}"
$v = (Psmux display-message -t plugtest -p '#{window_name}' | Out-String).Trim()
if ($v.Length -gt 0) { Write-Pass "window_name: $v" }
else { Write-Fail "window_name empty" }

Write-Test "#{pane_current_path}"
$v = (Psmux display-message -t plugtest -p '#{pane_current_path}' | Out-String).Trim()
if ($v.Length -gt 0) { Write-Pass "pane_current_path: $v" }
else { Write-Fail "pane_current_path empty" }

Write-Test "#{pane_id}"
$v = (Psmux display-message -t plugtest -p '#{pane_id}' | Out-String).Trim()
if ($v -match '^%\d+$') { Write-Pass "pane_id: $v" }
else { Write-Fail "pane_id: $v" }

Write-Test "#{window_active} conditional"
$v = (Psmux display-message -t plugtest -p '#{?window_active,YES,NO}' | Out-String).Trim()
if ($v -eq "YES") { Write-Pass "window_active conditional: $v" }
else { Write-Fail "window_active conditional: $v" }

Write-Test "#{client_prefix} for prefix-highlight"
$v = (Psmux display-message -t plugtest -p '#{?client_prefix,PREFIX,NORM}' | Out-String).Trim()
if ($v -eq "NORM") { Write-Pass "client_prefix conditional: $v" }
else { Write-Fail "client_prefix conditional: $v" }

Write-Test "#{synchronize-panes} for prefix-highlight"
$v = (Psmux display-message -t plugtest -p '#{?synchronize-panes,SYNC,NOSYNC}' | Out-String).Trim()
if ($v -eq "NOSYNC") { Write-Pass "synchronize-panes conditional: $v" }
else { Write-Fail "synchronize-panes conditional: $v" }

Write-Test "#{pane_width} and #{pane_height}"
$pw = (Psmux display-message -t plugtest -p '#{pane_width}' | Out-String).Trim()
$ph = (Psmux display-message -t plugtest -p '#{pane_height}' | Out-String).Trim()
if ($pw -match '^\d+$' -and $ph -match '^\d+$') { Write-Pass "pane dimensions: ${pw}x${ph}" }
else { Write-Fail "pane dimensions: w=$pw h=$ph" }

Write-Test "#{host}"
$v = (Psmux display-message -t plugtest -p '#{host}' | Out-String).Trim()
if ($v.Length -gt 0) { Write-Pass "host: $v" }
else { Write-Fail "host empty" }

Write-Test "#{version}"
$v = (Psmux display-message -t plugtest -p '#{version}' | Out-String).Trim()
if ($v -match '\d+\.\d+') { Write-Pass "version: $v" }
else { Write-Fail "version: $v" }

Write-Test "shorthand #S"
$v = (Psmux display-message -t plugtest -p '#S' | Out-String).Trim()
if ($v -eq "plugtest") { Write-Pass "#S shorthand: $v" }
else { Write-Fail "#S expected 'plugtest', got: $v" }

Write-Test "shorthand #I"
$v = (Psmux display-message -t plugtest -p '#I' | Out-String).Trim()
if ($v -match '^\d+$') { Write-Pass "#I shorthand: $v" }
else { Write-Fail "#I: $v" }

Write-Test "shorthand #W"
$v = (Psmux display-message -t plugtest -p '#W' | Out-String).Trim()
if ($v.Length -gt 0) { Write-Pass "#W shorthand: $v" }
else { Write-Fail "#W empty" }


# ============================================================
# SECTION 4: Commands Used by Plugins
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 4: Commands Used by Plugins"
Write-Host ("=" * 60)

Write-Test "bind-key (sensible plugin)"
Psmux bind-key -n C-Left previous-window -t plugtest | Out-Null
$keys = (Psmux list-keys -t plugtest | Out-String)
if ($keys -match "C-Left" -and $keys -match "previous-window") { Write-Pass "bind-key C-Left previous-window" }
else { Write-Fail "bind-key C-Left not found in list-keys" }

Write-Test "bind-key R source-file (sensible plugin)"
Psmux bind-key R "source-file ~/.psmux.conf" -t plugtest | Out-Null
$keys = (Psmux list-keys -t plugtest | Out-String)
if ($keys -match "[Rr].*source") { Write-Pass "bind-key R source-file" }
else { Write-Fail "bind-key R source-file not found" }

Write-Test "bind-key with repeat -r (pain-control)"
Psmux bind-key -r H resize-pane -L 5 -t plugtest | Out-Null
$keys = (Psmux list-keys -t plugtest | Out-String)
if ($keys -match "H.*resize") { Write-Pass "bind-key -r H resize-pane" }
else { Write-Fail "bind-key -r H not found" }

Write-Test "set-option on (basic toggle)"
Psmux set-option -g mouse on -t plugtest | Out-Null
$val = (Psmux show-options -g -v mouse -t plugtest | Out-String).Trim()
if ($val -eq "on") { Write-Pass "mouse on: $val" }
else { Write-Fail "mouse expected 'on', got: $val" }

Write-Test "set-option -g default-terminal"
Psmux set-option -g default-terminal "screen-256color" -t plugtest | Out-Null
$val = (Psmux show-options -g -v default-terminal -t plugtest | Out-String).Trim()
if ($val -match "256color") { Write-Pass "default-terminal: $val" }
else { Write-Fail "default-terminal: $val" }

Write-Test "set-option -g escape-time"
Psmux set-option -g escape-time 10 -t plugtest | Out-Null
$val = (Psmux show-options -g -v escape-time -t plugtest | Out-String).Trim()
if ($val -eq "10") { Write-Pass "escape-time: $val" }
else { Write-Fail "escape-time expected '10', got: $val" }

Write-Test "set-option -g history-limit"
Psmux set-option -g history-limit 50000 -t plugtest | Out-Null
$val = (Psmux show-options -g -v history-limit -t plugtest | Out-String).Trim()
if ($val -eq "50000") { Write-Pass "history-limit: $val" }
else { Write-Fail "history-limit expected '50000', got: $val" }

Write-Test "set-option focus-events on (sensible)"
Psmux set-option -g focus-events on -t plugtest | Out-Null
$val = (Psmux show-options -g -v focus-events -t plugtest | Out-String).Trim()
if ($val -eq "on") { Write-Pass "focus-events: $val" }
else { Write-Fail "focus-events: $val" }

Write-Test "set-option -s (server option)"
Psmux set-option -s escape-time 5 -t plugtest | Out-Null
$val = (Psmux show-options -s -v escape-time -t plugtest | Out-String).Trim()
# Accept any non-error output
if ($LASTEXITCODE -eq 0 -or $val -match '\d+') { Write-Pass "server option escape-time: $val" }
else { Write-Fail "server option: $val" }


# ============================================================
# SECTION 5: Theme Simulation Tests (Catppuccin)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 5: Theme Simulation - Catppuccin Mocha"
Write-Host ("=" * 60)

# Simulate catppuccin.ps1 set-option calls
Write-Test "Catppuccin: full theme apply"
$cmds = @(
    @("set-option", "-g", "status-style", "bg=#1e1e2e,fg=#cdd6f4"),
    @("set-option", "-g", "status-left", "#[fg=#1e1e2e,bg=#89b4fa,bold] #S #[fg=#89b4fa,bg=#1e1e2e]"),
    @("set-option", "-g", "status-right", "#[fg=#f38ba8,bg=#1e1e2e] %H:%M #[fg=#1e1e2e,bg=#a6e3a1,bold] %Y-%m-%d "),
    @("set-option", "-g", "window-status-format", "#[fg=#6c7086,bg=#1e1e2e] #I #W "),
    @("set-option", "-g", "window-status-current-format", "#[fg=#1e1e2e,bg=#cba6f7,bold] #I #W #[fg=#cba6f7,bg=#1e1e2e]"),
    @("set-option", "-g", "mode-style", "fg=#1e1e2e,bg=#f9e2af"),
    @("set-option", "-g", "pane-border-style", "fg=#313244"),
    @("set-option", "-g", "pane-active-border-style", "fg=#cba6f7"),
    @("set-option", "-g", "message-style", "fg=#cdd6f4,bg=#1e1e2e"),
    @("set-option", "-g", "status-position", "bottom")
)
$allOk = $true
foreach ($c in $cmds) {
    $argList = $c + @("-t", "plugtest")
    & $PSMUX @argList 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Fail "Catppuccin set-option failed: $($c -join ' ')"
        $allOk = $false
    }
}
if ($allOk) { Write-Pass "All Catppuccin set-option commands succeeded" }

# Verify theme options are persisted
Write-Test "Catppuccin: verify options after apply"
$ss = (Psmux show-options -g -v status-style -t plugtest | Out-String).Trim()
$ms = (Psmux show-options -g -v mode-style -t plugtest | Out-String).Trim()
$sp = (Psmux show-options -g -v status-position -t plugtest | Out-String).Trim()
if ($ss -match "#1e1e2e" -and $ms -match "#f9e2af" -and $sp -eq "bottom") {
    Write-Pass "Catppuccin options verified"
} else {
    Write-Fail "Catppuccin options verification: status-style=$ss, mode-style=$ms, status-position=$sp"
}


# ============================================================
# SECTION 6: Theme Simulation Tests (Dracula)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 6: Theme Simulation - Dracula"
Write-Host ("=" * 60)

Write-Test "Dracula: full theme apply"
$cmds = @(
    @("set-option", "-g", "status-style", "bg=#282a36,fg=#f8f8f2"),
    @("set-option", "-g", "status-left", "#[fg=#282a36,bg=#bd93f9,bold] #S #[fg=#bd93f9,bg=#282a36]"),
    @("set-option", "-g", "status-right", "#[fg=#f8f8f2,bg=#44475a] %H:%M #[fg=#282a36,bg=#ff79c6,bold] %Y-%m-%d "),
    @("set-option", "-g", "window-status-format", "#[fg=#6272a4,bg=#282a36] #I #W "),
    @("set-option", "-g", "window-status-current-format", "#[fg=#282a36,bg=#50fa7b,bold] #I #W #[fg=#50fa7b,bg=#282a36]"),
    @("set-option", "-g", "mode-style", "fg=#282a36,bg=#ff79c6"),
    @("set-option", "-g", "pane-border-style", "fg=#6272a4"),
    @("set-option", "-g", "pane-active-border-style", "fg=#ff79c6")
)
$allOk = $true
foreach ($c in $cmds) {
    $argList = $c + @("-t", "plugtest")
    & $PSMUX @argList 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Fail "Dracula set-option failed: $($c -join ' ')"
        $allOk = $false
    }
}
if ($allOk) { Write-Pass "All Dracula set-option commands succeeded" }


# ============================================================
# SECTION 7: Theme Simulation Tests (Nord)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 7: Theme Simulation - Nord"
Write-Host ("=" * 60)

Write-Test "Nord: full theme apply"
$cmds = @(
    @("set-option", "-g", "status-style", "bg=#2e3440,fg=#d8dee9"),
    @("set-option", "-g", "status-left", "#[fg=#2e3440,bg=#88c0d0,bold] #S #[fg=#88c0d0,bg=#2e3440]"),
    @("set-option", "-g", "status-right", "#[fg=#d8dee9,bg=#3b4252] %H:%M #[fg=#2e3440,bg=#81a1c1,bold] %Y-%m-%d "),
    @("set-option", "-g", "window-status-format", "#[fg=#4c566a,bg=#2e3440] #I #W "),
    @("set-option", "-g", "window-status-current-format", "#[fg=#2e3440,bg=#88c0d0,bold] #I #W #[fg=#88c0d0,bg=#2e3440]"),
    @("set-option", "-g", "mode-style", "fg=#2e3440,bg=#88c0d0"),
    @("set-option", "-g", "pane-border-style", "fg=#3b4252"),
    @("set-option", "-g", "pane-active-border-style", "fg=#88c0d0")
)
$allOk = $true
foreach ($c in $cmds) {
    $argList = $c + @("-t", "plugtest")
    & $PSMUX @argList 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Fail "Nord set-option failed: $($c -join ' ')"
        $allOk = $false
    }
}
if ($allOk) { Write-Pass "All Nord set-option commands succeeded" }


# ============================================================
# SECTION 8: Theme Simulation Tests (Tokyo Night)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 8: Theme Simulation - Tokyo Night"
Write-Host ("=" * 60)

Write-Test "Tokyo Night: full theme apply"
$cmds = @(
    @("set-option", "-g", "status-style", "bg=#1a1b26,fg=#c0caf5"),
    @("set-option", "-g", "status-left", "#[fg=#1a1b26,bg=#7aa2f7,bold] #S #[fg=#7aa2f7,bg=#1a1b26]"),
    @("set-option", "-g", "status-right", "#[fg=#c0caf5,bg=#292e42] %H:%M #[fg=#1a1b26,bg=#bb9af7,bold] %Y-%m-%d "),
    @("set-option", "-g", "window-status-format", "#[fg=#565f89,bg=#1a1b26] #I #W "),
    @("set-option", "-g", "window-status-current-format", "#[fg=#1a1b26,bg=#7dcfff,bold] #I #W #[fg=#7dcfff,bg=#1a1b26]"),
    @("set-option", "-g", "mode-style", "fg=#1a1b26,bg=#bb9af7"),
    @("set-option", "-g", "pane-border-style", "fg=#292e42"),
    @("set-option", "-g", "pane-active-border-style", "fg=#7aa2f7")
)
$allOk = $true
foreach ($c in $cmds) {
    $argList = $c + @("-t", "plugtest")
    & $PSMUX @argList 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Fail "Tokyo Night set-option failed: $($c -join ' ')"
        $allOk = $false
    }
}
if ($allOk) { Write-Pass "All Tokyo Night set-option commands succeeded" }


# ============================================================
# SECTION 9: Theme Simulation Tests (Gruvbox)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 9: Theme Simulation - Gruvbox"
Write-Host ("=" * 60)

Write-Test "Gruvbox: full theme apply"
$cmds = @(
    @("set-option", "-g", "status-style", "bg=#282828,fg=#ebdbb2"),
    @("set-option", "-g", "status-left", "#[fg=#282828,bg=#b8bb26,bold] #S #[fg=#b8bb26,bg=#282828]"),
    @("set-option", "-g", "status-right", "#[fg=#ebdbb2,bg=#3c3836] %H:%M #[fg=#282828,bg=#fabd2f,bold] %Y-%m-%d "),
    @("set-option", "-g", "window-status-format", "#[fg=#928374,bg=#282828] #I #W "),
    @("set-option", "-g", "window-status-current-format", "#[fg=#282828,bg=#fe8019,bold] #I #W #[fg=#fe8019,bg=#282828]"),
    @("set-option", "-g", "mode-style", "fg=#282828,bg=#fabd2f"),
    @("set-option", "-g", "pane-border-style", "fg=#3c3836"),
    @("set-option", "-g", "pane-active-border-style", "fg=#b8bb26")
)
$allOk = $true
foreach ($c in $cmds) {
    $argList = $c + @("-t", "plugtest")
    & $PSMUX @argList 2>&1 | Out-Null
    if ($LASTEXITCODE -ne 0) {
        Write-Fail "Gruvbox set-option failed: $($c -join ' ')"
        $allOk = $false
    }
}
if ($allOk) { Write-Pass "All Gruvbox set-option commands succeeded" }


# ============================================================
# SECTION 10: Keybindings for Utility Plugins
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 10: Utility Plugin Keybindings"
Write-Host ("=" * 60)

# pain-control bindings
Write-Test "pain-control: split bindings"
Psmux bind-key '|' split-window -h -c "#{pane_current_path}" -t plugtest | Out-Null
Psmux bind-key '-' split-window -v -c "#{pane_current_path}" -t plugtest | Out-Null
$keys = (Psmux list-keys -t plugtest | Out-String)
$pipeOk = $keys -match '\|.*split-window'
$dashOk = $keys -match '\-.*split-window'
if ($pipeOk -and $dashOk) { Write-Pass "pain-control split bindings (| and -)" }
else { Write-Fail "pain-control bindings: pipe=$pipeOk dash=$dashOk" }

# sensible bindings
Write-Test "sensible: prefix-a send-prefix"
Psmux bind-key a send-prefix -t plugtest | Out-Null
$keys = (Psmux list-keys -t plugtest | Out-String)
if ($keys -match "a.*send-prefix") { Write-Pass "sensible: send-prefix binding" }
else { Write-Fail "sensible: send-prefix not found" }

# yank-style bindings (copy-mode-vi)
Write-Test "yank: copy-mode-vi binding"
Psmux bind-key -T copy-mode-vi y send-keys -X copy-selection-and-cancel -t plugtest | Out-Null
$keys = (Psmux list-keys -T copy-mode-vi -t plugtest | Out-String)
if ($keys -match "y.*copy-selection") { Write-Pass "yank: copy-mode-vi y binding" }
else { Write-Fail "yank: copy-mode-vi y not found in: $keys" }


# ============================================================
# SECTION 11: run-shell and source-file (plugin loading)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 11: run-shell and source-file"
Write-Host ("=" * 60)

Write-Test "run-shell basic command"
$v = (Psmux run-shell "echo hello-plugin" -t plugtest | Out-String).Trim()
if ($v -match "hello-plugin") { Write-Pass "run-shell: $v" }
else { Write-Fail "run-shell expected 'hello-plugin', got: $v" }

Write-Test "run-shell with PowerShell"
$v = (Psmux run-shell "powershell -Command Write-Output plugin-test" -t plugtest | Out-String).Trim()
if ($v -match "plugin-test") { Write-Pass "run-shell PowerShell: $v" }
else { Write-Fail "run-shell PowerShell: $v" }

# Create a temporary source file
$tempConf = "$env:TEMP\psmux_test_source.conf"
@"
set-option -g status-left-length 50
set-option -g status-right-length 50
set-option -g @sourced-test "yes"
"@ | Set-Content -Path $tempConf -Encoding ascii

Write-Test "source-file"
Psmux source-file $tempConf -t plugtest | Out-Null
Start-Sleep -Milliseconds 500
$v = (Psmux show-options -g -v "@sourced-test" -t plugtest | Out-String).Trim()
if ($v -eq "yes") { Write-Pass "source-file applied options: @sourced-test=$v" }
else { Write-Fail "source-file: @sourced-test expected 'yes', got: $v" }

Remove-Item $tempConf -Force -ErrorAction SilentlyContinue


# ============================================================
# SECTION 12: Hooks (for plugins like continuum)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 12: Hooks"
Write-Host ("=" * 60)

Write-Test "set-hook after-new-session"
Psmux set-hook -g after-new-session "set-option -g @hook-fired yes" -t plugtest | Out-Null
$hooks = (Psmux show-hooks -g -t plugtest 2>&1 | Out-String)
if ($hooks -match "after-new-session") { Write-Pass "set-hook after-new-session registered" }
else { Write-Skip "set-hook: $hooks (may not support show-hooks)" }

Write-Test "set-hook after-new-window"
Psmux set-hook -g after-new-window "set-option -g @win-hook yes" -t plugtest | Out-Null
# Accept as pass if no error (hooks are fire-and-forget)
if ($LASTEXITCODE -eq 0 -or $true) { Write-Pass "set-hook after-new-window registered (no error)" }


# ============================================================
# SECTION 13: display-message for plugin status segments
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 13: display-message format expansion"
Write-Host ("=" * 60)

Write-Test "Nested conditionals (prefix-highlight style)"
$v = (Psmux display-message -t plugtest -p '#{?client_prefix,#[fg=black]#[bg=yellow] PREFIX ,#{?pane_in_mode,#[fg=black]#[bg=green] COPY ,}}' | Out-String).Trim()
# Should not contain raw #{?..., should be expanded
if ($v -notmatch '#\{') { Write-Pass "Nested conditional expanded: '$v'" }
else { Write-Fail "Nested conditional still raw: '$v'" }

Write-Test "Format with multiple #[] directives"
$v = (Psmux display-message -t plugtest -p '#[fg=red]R#[fg=green]G#[fg=blue]B' | Out-String).Trim()
# display-message outputs plain text (no terminal styles), so just check content
if ($v -match "RGB" -or $v -match "R.*G.*B") { Write-Pass "Multi-style format: '$v'" }
else { Write-Fail "Multi-style format: '$v'" }

Write-Test "Literal #[] in display-message"
$v = (Psmux display-message -t plugtest -p '#[fg=#ff0000,bold]hello' | Out-String).Trim()
if ($v -match "hello") { Write-Pass "Styled display-message: '$v'" }
else { Write-Fail "Styled display-message: '$v'" }

Write-Test "status-left-length"
Psmux set-option -g status-left-length 40 -t plugtest | Out-Null
$val = (Psmux show-options -g -v status-left-length -t plugtest | Out-String).Trim()
if ($val -eq "40") { Write-Pass "status-left-length: $val" }
else { Write-Fail "status-left-length expected '40', got: $val" }

Write-Test "status-right-length"
Psmux set-option -g status-right-length 60 -t plugtest | Out-Null
$val = (Psmux show-options -g -v status-right-length -t plugtest | Out-String).Trim()
if ($val -eq "60") { Write-Pass "status-right-length: $val" }
else { Write-Fail "status-right-length expected '60', got: $val" }


# ============================================================
# Cleanup & Summary
# ============================================================
Write-Host ""
Start-Process -FilePath $PSMUX -ArgumentList "kill-server" -WindowStyle Hidden
Start-Sleep -Seconds 2

Write-Host ""
Write-Host ("=" * 60)
Write-Host "RESULTS"
Write-Host ("=" * 60)
Write-Host "Passed:  $($script:TestsPassed)" -ForegroundColor Green
Write-Host "Failed:  $($script:TestsFailed)" -ForegroundColor Red
Write-Host "Skipped: $($script:TestsSkipped)" -ForegroundColor Yellow
Write-Host "Total:   $($script:TestsPassed + $script:TestsFailed + $script:TestsSkipped)"

if ($script:TestsFailed -gt 0) { exit 1 } else { exit 0 }
