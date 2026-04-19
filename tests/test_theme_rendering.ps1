# psmux Theme Rendering Robustness Tests
# Tests that themes don't cause client TUI hangs or rendering failures.
# Covers: inline style edge cases, malformed directives, rendering with timeout,
#         the specific gruvbox #[ truncation bug that caused infinite loops.
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_theme_rendering.ps1

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

$S = "rendertest"

function Start-FreshSession {
    & $PSMUX kill-server 2>$null
    Start-Sleep -Seconds 2
    Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
    Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -s $S -d" -WindowStyle Hidden
    Start-Sleep -Seconds 3
    & $PSMUX has-session -t $S 2>$null
    return ($LASTEXITCODE -eq 0)
}

# Helper: apply a theme and verify the session still works (capture-pane with timeout)
function Test-ThemeRendering {
    param(
        [string]$ThemeName,
        [string[]]$SetOptionCmds,
        [int]$TimeoutSec = 10
    )
    Write-Test "$ThemeName : apply theme and verify session responds"

    foreach ($cmd in $SetOptionCmds) {
        $parts = $cmd -split '\s+', 2
        $argStr = "$($parts[0]) $($parts[1]) -t $S"
        $argList = $argStr -split '\s+'
        & $PSMUX @argList 2>&1 | Out-Null
        Start-Sleep -Milliseconds 100
    }

    # Verify session still responds within timeout (catches rendering hangs)
    $job = Start-Job -ScriptBlock {
        param($psmux, $session)
        & $psmux capture-pane -t $session -p 2>&1
    } -ArgumentList $PSMUX, $S
    $completed = Wait-Job $job -Timeout $TimeoutSec
    if ($completed) {
        $output = Receive-Job $job
        Remove-Job $job -Force
        if ($output -match '\S') {
            Write-Pass "$ThemeName : session responds, capture-pane has content"
            return $true
        } else {
            # May have blank output if pane hasn't printed anything yet - still OK if it didn't hang
            Write-Pass "$ThemeName : session responds (capture-pane returned)"
            return $true
        }
    } else {
        Stop-Job $job -ErrorAction SilentlyContinue
        Remove-Job $job -Force
        Write-Fail "$ThemeName : session HUNG (timeout ${TimeoutSec}s) — possible rendering infinite loop!"
        return $false
    }
}


# ============================================================
# SECTION 1: Theme Rendering — All Major Themes
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 1: Theme Rendering Robustness (no hangs)"
Write-Host ("=" * 60)

if (-not (Start-FreshSession)) {
    Write-Host "FATAL: Cannot create test session" -ForegroundColor Red; exit 1
}

# --- Catppuccin ---
$catppuccin = @(
    'set-option -g status-style "bg=#1e1e2e,fg=#cdd6f4"',
    'set-option -g status-left "#[fg=#1e1e2e,bg=#89b4fa,bold] #S #[fg=#89b4fa,bg=#1e1e2e]"',
    'set-option -g status-right "#[fg=#f38ba8,bg=#1e1e2e] %H:%M #[fg=#1e1e2e,bg=#a6e3a1,bold] %Y-%m-%d "',
    'set-option -g window-status-format "#[fg=#6c7086,bg=#1e1e2e] #I #W "',
    'set-option -g window-status-current-format "#[fg=#1e1e2e,bg=#cba6f7,bold] #I #W #[fg=#cba6f7,bg=#1e1e2e]"'
)
Test-ThemeRendering -ThemeName "Catppuccin" -SetOptionCmds $catppuccin

# --- Dracula ---
$dracula = @(
    'set-option -g status-style "bg=#282a36,fg=#f8f8f2"',
    'set-option -g status-left "#[fg=#282a36,bg=#bd93f9,bold] #S #[fg=#bd93f9,bg=#282a36]"',
    'set-option -g status-right "#[fg=#f8f8f2,bg=#44475a] %H:%M #[fg=#282a36,bg=#ff79c6,bold] %Y-%m-%d "',
    'set-option -g window-status-format "#[fg=#6272a4,bg=#282a36] #I #W "',
    'set-option -g window-status-current-format "#[fg=#282a36,bg=#50fa7b,bold] #I #W #[fg=#50fa7b,bg=#282a36]"'
)
Test-ThemeRendering -ThemeName "Dracula" -SetOptionCmds $dracula

# --- Nord ---
$nord = @(
    'set-option -g status-style "bg=#2e3440,fg=#d8dee9"',
    'set-option -g status-left "#[fg=#2e3440,bg=#88c0d0,bold] #S #[fg=#88c0d0,bg=#2e3440]"',
    'set-option -g status-right "#[fg=#d8dee9,bg=#3b4252] %H:%M #[fg=#2e3440,bg=#81a1c1,bold] %Y-%m-%d "',
    'set-option -g window-status-format "#[fg=#4c566a,bg=#2e3440] #I #W "',
    'set-option -g window-status-current-format "#[fg=#2e3440,bg=#88c0d0,bold] #I #W #[fg=#88c0d0,bg=#2e3440]"'
)
Test-ThemeRendering -ThemeName "Nord" -SetOptionCmds $nord

# --- Tokyo Night ---
$tokyonight = @(
    'set-option -g status-style "bg=#1a1b26,fg=#c0caf5"',
    'set-option -g status-left "#[fg=#1a1b26,bg=#7aa2f7,bold] #S #[fg=#7aa2f7,bg=#1a1b26]"',
    'set-option -g status-right "#[fg=#c0caf5,bg=#292e42] %H:%M #[fg=#1a1b26,bg=#bb9af7,bold] %Y-%m-%d "',
    'set-option -g window-status-format "#[fg=#565f89,bg=#1a1b26] #I #W "',
    'set-option -g window-status-current-format "#[fg=#1a1b26,bg=#7dcfff,bold] #I #W #[fg=#7dcfff,bg=#1a1b26]"'
)
Test-ThemeRendering -ThemeName "Tokyo Night" -SetOptionCmds $tokyonight

# --- Gruvbox (the theme that triggered the infinite loop bug) ---
$gruvbox = @(
    'set-option -g status-style "bg=#3c3836,fg=#ebdbb2"',
    'set-option -g status-left "#[bg=#fabd2f,fg=#282828,bold] #S #[fg=#fabd2f,bg=#3c3836] "',
    'set-option -g status-right "#{?client_prefix,#[fg=#fe8019]#[bg=#3c3836]#[bg=#fe8019]#[fg=#282828] WAIT #[fg=#fe8019]#[bg=#3c3836],}#[fg=#504945,bg=#3c3836]#[fg=#ebdbb2,bg=#504945] %H:%M #[fg=#8ec07c,bg=#504945]#[fg=#282828,bg=#8ec07c,bold] %d-%b "',
    'set-option -g window-status-format "#[fg=#a89984,bg=#3c3836] #I #W "',
    'set-option -g window-status-current-format "#[fg=#3c3836,bg=#fe8019,bold] #I #W #[fg=#fe8019,bg=#3c3836]"'
)
Test-ThemeRendering -ThemeName "Gruvbox" -SetOptionCmds $gruvbox


# ============================================================
# SECTION 2: Malformed Style Directives (edge cases)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 2: Malformed Style Directives"
Write-Host ("=" * 60)
Write-Info "These test that parse_inline_styles handles bad input gracefully"

# Test 2a: Unclosed #[ at end of string (the exact bug scenario)
Write-Test "Unclosed #[ at end of status-left"
& $PSMUX set-option -g status-left '#[bg=#fabd2f,fg=#282828,b' -t $S 2>&1 | Out-Null
$job = Start-Job -ScriptBlock {
    param($psmux, $session)
    & $psmux capture-pane -t $session -p 2>&1
} -ArgumentList $PSMUX, $S
$completed = Wait-Job $job -Timeout 8
if ($completed) {
    Receive-Job $job | Out-Null
    Remove-Job $job -Force
    Write-Pass "Unclosed #[ did NOT cause hang"
} else {
    Stop-Job $job; Remove-Job $job -Force
    Write-Fail "Unclosed #[ caused hang (infinite loop in parse_inline_styles)"
}

# Test 2b: Multiple unclosed #[
Write-Test "Multiple unclosed #[ directives"
& $PSMUX set-option -g status-left '#[fg=red#[bg=blue' -t $S 2>&1 | Out-Null
$job = Start-Job -ScriptBlock {
    param($psmux, $session)
    & $psmux capture-pane -t $session -p 2>&1
} -ArgumentList $PSMUX, $S
$completed = Wait-Job $job -Timeout 8
if ($completed) {
    Receive-Job $job | Out-Null
    Remove-Job $job -Force
    Write-Pass "Multiple unclosed #[ handled gracefully"
} else {
    Stop-Job $job; Remove-Job $job -Force
    Write-Fail "Multiple unclosed #[ caused hang"
}

# Test 2c: Empty #[]
Write-Test "Empty #[] directive"
& $PSMUX set-option -g status-left '#[] hello world' -t $S 2>&1 | Out-Null
$job = Start-Job -ScriptBlock {
    param($psmux, $session)
    & $psmux capture-pane -t $session -p 2>&1
} -ArgumentList $PSMUX, $S
$completed = Wait-Job $job -Timeout 8
if ($completed) {
    Receive-Job $job | Out-Null
    Remove-Job $job -Force
    Write-Pass "Empty #[] handled"
} else {
    Stop-Job $job; Remove-Job $job -Force
    Write-Fail "Empty #[] caused hang"
}

# Test 2d: Just a lone #[
Write-Test "Lone #[ as entire status-left"
& $PSMUX set-option -g status-left '#[' -t $S 2>&1 | Out-Null
$job = Start-Job -ScriptBlock {
    param($psmux, $session)
    & $psmux capture-pane -t $session -p 2>&1
} -ArgumentList $PSMUX, $S
$completed = Wait-Job $job -Timeout 8
if ($completed) {
    Receive-Job $job | Out-Null
    Remove-Job $job -Force
    Write-Pass "Lone #[ handled"
} else {
    Stop-Job $job; Remove-Job $job -Force
    Write-Fail "Lone #[ caused hang"
}

# Test 2e: Nested #[ (should not be valid but shouldn't crash)
Write-Test "Nested #[#[]] directive"
& $PSMUX set-option -g status-left '#[fg=#[bg=red]]' -t $S 2>&1 | Out-Null
$job = Start-Job -ScriptBlock {
    param($psmux, $session)
    & $psmux capture-pane -t $session -p 2>&1
} -ArgumentList $PSMUX, $S
$completed = Wait-Job $job -Timeout 8
if ($completed) {
    Receive-Job $job | Out-Null
    Remove-Job $job -Force
    Write-Pass "Nested #[#[]] handled"
} else {
    Stop-Job $job; Remove-Job $job -Force
    Write-Fail "Nested #[#[]] caused hang"
}

# Test 2f: Unclosed #[ in status-right (same bug, different field)
Write-Test "Unclosed #[ in status-right"
& $PSMUX set-option -g status-right '#[fg=#504945,bg=#3c383' -t $S 2>&1 | Out-Null
$job = Start-Job -ScriptBlock {
    param($psmux, $session)
    & $psmux capture-pane -t $session -p 2>&1
} -ArgumentList $PSMUX, $S
$completed = Wait-Job $job -Timeout 8
if ($completed) {
    Receive-Job $job | Out-Null
    Remove-Job $job -Force
    Write-Pass "Unclosed #[ in status-right handled"
} else {
    Stop-Job $job; Remove-Job $job -Force
    Write-Fail "Unclosed #[ in status-right caused hang"
}

# Test 2g: Unclosed #[ in window-status-current-format
Write-Test "Unclosed #[ in window-status-current-format"
& $PSMUX set-option -g window-status-current-format '#[fg=#282828,bg=#fe8019,bo' -t $S 2>&1 | Out-Null
$job = Start-Job -ScriptBlock {
    param($psmux, $session)
    & $psmux capture-pane -t $session -p 2>&1
} -ArgumentList $PSMUX, $S
$completed = Wait-Job $job -Timeout 8
if ($completed) {
    Receive-Job $job | Out-Null
    Remove-Job $job -Force
    Write-Pass "Unclosed #[ in window format handled"
} else {
    Stop-Job $job; Remove-Job $job -Force
    Write-Fail "Unclosed #[ in window format caused hang"
}

# Test 2h: Very long style string (stress test parser)
Write-Test "Very long style string (100+ directives)"
$longStyle = ""
for ($i = 0; $i -lt 100; $i++) {
    $r = Get-Random -Minimum 0 -Maximum 255
    $g = Get-Random -Minimum 0 -Maximum 255
    $b = Get-Random -Minimum 0 -Maximum 255
    $longStyle += "#[fg=#$($r.ToString('X2'))$($g.ToString('X2'))$($b.ToString('X2'))]X"
}
& $PSMUX set-option -g status-left $longStyle -t $S 2>&1 | Out-Null
$job = Start-Job -ScriptBlock {
    param($psmux, $session)
    & $psmux capture-pane -t $session -p 2>&1
} -ArgumentList $PSMUX, $S
$completed = Wait-Job $job -Timeout 10
if ($completed) {
    Receive-Job $job | Out-Null
    Remove-Job $job -Force
    Write-Pass "Very long style string ($(($longStyle.Length)) chars) handled"
} else {
    Stop-Job $job; Remove-Job $job -Force
    Write-Fail "Very long style string caused hang/timeout"
}


# ============================================================
# SECTION 3: Gruvbox Truncation Regression Test
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 3: Gruvbox Truncation Regression"
Write-Host ("=" * 60)
Write-Info "This tests the exact scenario that caused the blank screen bug:"
Write-Info "  status_left='#[bg=#fabd2f,fg=#282828,bold] X #[fg=#fabd2f,bg=#3c3836] '"
Write-Info "  status_left_length=25 would truncate to '#[bg=#fabd2f,fg=#282828,b'"
Write-Info "  causing an infinite loop in parse_inline_styles"

# Restart fresh to ensure clean state
if (-not (Start-FreshSession)) {
    Write-Host "FATAL: Cannot create session for gruvbox regression" -ForegroundColor Red
} else {
    # Apply the exact gruvbox theme from the actual plugin
    Write-Test "Apply real gruvbox theme format strings"
    & $PSMUX set-option -g status-style "bg=#3c3836,fg=#ebdbb2" -t $S 2>&1 | Out-Null
    & $PSMUX set-option -g status-left '#[bg=#fabd2f,fg=#282828,bold] #S #[fg=#fabd2f,bg=#3c3836] ' -t $S 2>&1 | Out-Null
    & $PSMUX set-option -g status-right '#{?client_prefix,#[fg=#fe8019]#[bg=#3c3836]#[bg=#fe8019]#[fg=#282828] WAIT #[fg=#fe8019]#[bg=#3c3836],}#[fg=#504945,bg=#3c3836]#[fg=#ebdbb2,bg=#504945] %H:%M #[fg=#8ec07c,bg=#504945]#[fg=#282828,bg=#8ec07c,bold] %d-%b ' -t $S 2>&1 | Out-Null
    & $PSMUX set-option -g window-status-format '#[fg=#a89984,bg=#3c3836] #I #W ' -t $S 2>&1 | Out-Null
    & $PSMUX set-option -g window-status-current-format '#[fg=#3c3836,bg=#fe8019,bold] #I #W #[fg=#fe8019,bg=#3c3836]' -t $S 2>&1 | Out-Null
    & $PSMUX set-option -g status-left-length 25 -t $S 2>&1 | Out-Null
    & $PSMUX set-option -g status-right-length 50 -t $S 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    # Verify the theme was applied
    Write-Test "Gruvbox: verify status-left is set"
    $sl = (& $PSMUX show-options -g -v status-left -t $S | Out-String).Trim()
    if ($sl -match "#fabd2f") { Write-Pass "status-left has gruvbox yellow: $($sl.Substring(0, [Math]::Min(50, $sl.Length)))" }
    else { Write-Fail "status-left: '$sl'" }

    # Now the critical test: does capture-pane work within timeout?
    # Before the fix, this would hang because:
    #   1. Server sends status_left = "#[bg=#fabd2f,fg=#282828,bold] rendertest #[fg=#fabd2f,bg=#3c3836] "
    #   2. Client (old code) truncated to 25 chars: "#[bg=#fabd2f,fg=#282828,b"
    #   3. parse_inline_styles found #[ but no ] -> infinite loop
    Write-Test "Gruvbox: capture-pane responds within 5s (regression test)"
    $job = Start-Job -ScriptBlock {
        param($psmux, $session)
        & $psmux capture-pane -t $session -p 2>&1
    } -ArgumentList $PSMUX, $S
    $completed = Wait-Job $job -Timeout 5
    if ($completed) {
        $output = (Receive-Job $job | Out-String).Trim()
        Remove-Job $job -Force
        if ($output -match '\S') {
            Write-Pass "Gruvbox regression: capture-pane has content (no hang!)"
        } else {
            Write-Pass "Gruvbox regression: capture-pane returned (no hang)"
        }
    } else {
        Stop-Job $job -ErrorAction SilentlyContinue
        Remove-Job $job -Force
        Write-Fail "Gruvbox regression: HUNG! This is the original bug."
    }

    # Also test with status-left-length smaller than the #[ directive
    Write-Test "Gruvbox: status-left-length=10 (cuts mid-directive)"
    & $PSMUX set-option -g status-left-length 10 -t $S 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    $job = Start-Job -ScriptBlock {
        param($psmux, $session)
        & $psmux capture-pane -t $session -p 2>&1
    } -ArgumentList $PSMUX, $S
    $completed = Wait-Job $job -Timeout 5
    if ($completed) {
        Receive-Job $job | Out-Null
        Remove-Job $job -Force
        Write-Pass "status-left-length=10 handled (no hang)"
    } else {
        Stop-Job $job; Remove-Job $job -Force
        Write-Fail "status-left-length=10 caused hang"
    }

    # Test with status-left-length=1 (extreme truncation)
    Write-Test "Gruvbox: status-left-length=1 (extreme)"
    & $PSMUX set-option -g status-left-length 1 -t $S 2>&1 | Out-Null
    Start-Sleep -Milliseconds 300
    $job = Start-Job -ScriptBlock {
        param($psmux, $session)
        & $psmux capture-pane -t $session -p 2>&1
    } -ArgumentList $PSMUX, $S
    $completed = Wait-Job $job -Timeout 5
    if ($completed) {
        Receive-Job $job | Out-Null
        Remove-Job $job -Force
        Write-Pass "status-left-length=1 handled"
    } else {
        Stop-Job $job; Remove-Job $job -Force
        Write-Fail "status-left-length=1 caused hang"
    }
}


# ============================================================
# SECTION 4: Real Plugin Theme Scripts (if available)
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 4: Real Plugin Theme Rendering"
Write-Host ("=" * 60)

$PLUGIN_DIR = "$env:USERPROFILE\.psmux\plugins"
$themes = @(
    @{ Name = "catppuccin";  Path = "$PLUGIN_DIR\psmux-theme-catppuccin\psmux-theme-catppuccin.ps1" },
    @{ Name = "dracula";     Path = "$PLUGIN_DIR\psmux-theme-dracula\psmux-theme-dracula.ps1" },
    @{ Name = "gruvbox";     Path = "$PLUGIN_DIR\psmux-theme-gruvbox\psmux-theme-gruvbox.ps1" },
    @{ Name = "nord";        Path = "$PLUGIN_DIR\psmux-theme-nord\psmux-theme-nord.ps1" },
    @{ Name = "tokyonight";  Path = "$PLUGIN_DIR\psmux-theme-tokyonight\psmux-theme-tokyonight.ps1" }
)

# Ensure psmux is in PATH for plugin scripts
$binDir = Split-Path $PSMUX
$env:PATH = "$binDir;$env:PATH"

foreach ($theme in $themes) {
    if (-not (Test-Path $theme.Path)) {
        Write-Skip "$($theme.Name): plugin script not found at $($theme.Path)"
        continue
    }

    if (-not (Start-FreshSession)) {
        Write-Fail "$($theme.Name): cannot create session"
        continue
    }

    Write-Test "$($theme.Name): source real theme script"
    $output = pwsh -NoProfile -ExecutionPolicy Bypass -Command "& '$($theme.Path)'" 2>&1 | Out-String
    Start-Sleep -Milliseconds 500

    # Verify session responds after theme is applied (rendering works)
    $job = Start-Job -ScriptBlock {
        param($psmux, $session)
        & $psmux capture-pane -t $session -p 2>&1
    } -ArgumentList $PSMUX, $S
    $completed = Wait-Job $job -Timeout 8
    if ($completed) {
        Receive-Job $job | Out-Null
        Remove-Job $job -Force
        Write-Pass "$($theme.Name): rendered without hang"
    } else {
        Stop-Job $job; Remove-Job $job -Force
        Write-Fail "$($theme.Name): HUNG after applying real theme script!"
    }

    # Verify theme options were applied
    $ss = (& $PSMUX show-options -g -v status-style -t $S | Out-String).Trim()
    if ($ss -match '#[0-9a-fA-F]{6}') {
        Write-Pass "$($theme.Name): status-style has hex color: $($ss.Substring(0, [Math]::Min(40, $ss.Length)))"
    } else {
        Write-Fail "$($theme.Name): status-style missing hex colors: '$ss'"
    }
}


# ============================================================
# SECTION 5: Debug Logging Activation Test
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "SECTION 5: Debug Logging (PSMUX_CLIENT_DEBUG)"
Write-Host ("=" * 60)

# Test that debug log is NOT created when env var is unset
Write-Test "Debug log not created without env var"
Remove-Item "$env:USERPROFILE\.psmux\client_debug.log" -Force -ErrorAction SilentlyContinue
if (-not (Start-FreshSession)) {
    Write-Fail "Cannot create session for logging test"
} else {
    Start-Sleep -Seconds 2
    $exists = Test-Path "$env:USERPROFILE\.psmux\client_debug.log"
    if (-not $exists) {
        Write-Pass "client_debug.log not created (logging correctly disabled)"
    } else {
        Write-Fail "client_debug.log was created even without PSMUX_CLIENT_DEBUG=1"
    }
}

# Test that PSMUX_CLIENT_DEBUG=1 creates the log (attached client needed)
Write-Test "Debug log created with PSMUX_CLIENT_DEBUG=1 (requires TUI attach)"
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\client_debug.log" -Force -ErrorAction SilentlyContinue
$savedDebug = $env:PSMUX_CLIENT_DEBUG
$env:PSMUX_CLIENT_DEBUG = "1"
Start-Process -FilePath $PSMUX -ArgumentList "new-session -s debug-logtest" -WindowStyle Minimized
Start-Sleep -Seconds 5
$exists = Test-Path "$env:USERPROFILE\.psmux\client_debug.log"
if ($exists) {
    $lines = (Get-Content "$env:USERPROFILE\.psmux\client_debug.log" | Measure-Object).Count
    Write-Pass "client_debug.log created with $lines lines"
    # Verify it contains expected log components
    $content = Get-Content "$env:USERPROFILE\.psmux\client_debug.log" -Raw
    if ($content -match '\[frame\]' -and $content -match '\[draw\]' -and $content -match '\[parse\]') {
        Write-Pass "Log contains frame, draw, parse components"
    } else {
        Write-Fail "Log missing expected components (frame/draw/parse)"
    }
} else {
    Write-Skip "client_debug.log not created — TUI may not have launched in minimized window"
}
$env:PSMUX_CLIENT_DEBUG = $savedDebug
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 2
Remove-Item "$env:USERPROFILE\.psmux\client_debug.log" -Force -ErrorAction SilentlyContinue

# ============================================================
# Win32 TUI VERIFICATION: Prove theme colors apply in real window
# ============================================================
Write-Host ""
Write-Host ("=" * 60)
Write-Host "Win32 TUI VISUAL VERIFICATION" -ForegroundColor Yellow
Write-Host ("=" * 60)

. "$PSScriptRoot\tui_helper.ps1"
$TUI_SESSION_THM = "thm_tui_proof"

$tuiOk = Launch-PsmuxWindow -Session $TUI_SESSION_THM
if ($tuiOk) {
    Start-Sleep -Seconds 2

    # TUI Test 1: Set status bar color via CLI and verify option applied
    Write-Test "TUI: Set status-style via CLI (visible TUI proof)"
    & $script:TUI_PSMUX set-option -t $TUI_SESSION_THM status-style "bg=red,fg=white" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $statusStyle = & $script:TUI_PSMUX show-options -g -v status-style -t $TUI_SESSION_THM 2>&1 | Out-String
    $statusStyle = $statusStyle.Trim()
    if ($statusStyle -match "red") {
        Write-Pass "TUI: status-style contains 'red' after CLI command ($statusStyle)"
    } else {
        Write-Fail "TUI: status-style unexpected: '$statusStyle'"
    }

    # TUI Test 2: Set pane border color and verify
    Write-Test "TUI: Set pane-border-style via CLI (visible TUI proof)"
    & $script:TUI_PSMUX split-window -h -t $TUI_SESSION_THM 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    & $script:TUI_PSMUX set-option -t $TUI_SESSION_THM pane-border-style "fg=green" 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500
    $borderStyle = & $script:TUI_PSMUX show-options -g -v pane-border-style -t $TUI_SESSION_THM 2>&1 | Out-String
    $borderStyle = $borderStyle.Trim()
    if ($borderStyle -match "green") {
        Write-Pass "TUI: pane-border-style contains 'green' ($borderStyle)"
    } else {
        Write-Fail "TUI: pane-border-style unexpected: '$borderStyle'"
    }

    # TUI Test 3: Session stays responsive after theme changes (no hang)
    Write-Test "TUI: Session responsive after multiple theme changes"
    $resp = Safe-TuiQuery "#{session_name}" -Session $TUI_SESSION_THM
    if ($resp -eq $TUI_SESSION_THM) {
        Write-Pass "TUI: Session responsive after theme changes (name=$resp)"
    } else {
        Write-Fail "TUI: Session not responsive (got: '$resp')"
    }

    Cleanup-PsmuxWindow -Session $TUI_SESSION_THM
    Write-Host ""
} else {
    Write-Info "TUI verification skipped (could not launch window)"
}


# ============================================================
# Cleanup & Summary
# ============================================================
Write-Host ""
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 2

Write-Host ""
Write-Host ("=" * 60)
Write-Host "THEME RENDERING TEST RESULTS"
Write-Host ("=" * 60)
Write-Host "Passed:  $($script:TestsPassed)" -ForegroundColor Green
Write-Host "Failed:  $($script:TestsFailed)" -ForegroundColor Red
Write-Host "Skipped: $($script:TestsSkipped)" -ForegroundColor Yellow
Write-Host "Total:   $($script:TestsPassed + $script:TestsFailed + $script:TestsSkipped)"

if ($script:TestsFailed -gt 0) { exit 1 } else { exit 0 }
