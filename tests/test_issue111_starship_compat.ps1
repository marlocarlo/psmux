# psmux Issue #111 — Starship prompt compatibility
#
# Tests that psmux's CWD sync hook works correctly with Starship prompt.
# Starship uses a dynamic PowerShell module (New-Module) with module-scoped
# functions (Get-Cwd, Invoke-Native). The CWD hook must preserve the module
# scope when wrapping the prompt function.
#
# Requires: Starship installed (winget install Starship.Starship)
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue111_starship_compat.ps1

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

# Check Starship is installed
$starshipPath = (Get-Command starship -ErrorAction SilentlyContinue).Source
if (-not $starshipPath) {
    # Try common install location
    $starshipPath = "C:\Program Files\starship\bin\starship.exe"
    if (-not (Test-Path $starshipPath)) {
        Write-Skip "Starship not installed — skipping all tests"
        Write-Host "`n$($script:TestsSkipped) skipped"
        exit 0
    }
}
Write-Info "Starship: $starshipPath"

# Clean slate
Write-Info "Cleaning up existing sessions..."
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "test_starship"

function Wait-ForSession {
    param($name, $timeout = 15)
    for ($i = 0; $i -lt ($timeout * 2); $i++) {
        & $PSMUX has-session -t $name 2>$null
        if ($LASTEXITCODE -eq 0) { return $true }
        Start-Sleep -Milliseconds 500
    }
    return $false
}

function Cleanup-Session {
    param($name)
    & $PSMUX kill-session -t $name 2>$null
    Start-Sleep -Milliseconds 500
}

function Capture-Pane {
    param($target)
    $raw = & $PSMUX capture-pane -t $target -p 2>&1
    return ($raw | Out-String)
}

function New-TestSession {
    param($name)
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $name" -WindowStyle Hidden
    if (-not (Wait-ForSession $name)) {
        Write-Fail "Could not create session $name"
        return $false
    }
    # Wait longer for Starship to initialize (it invokes the starship binary)
    Start-Sleep -Seconds 5
    return $true
}

# Create a temporary profile that initializes Starship
$tempProfile = Join-Path $env:TEMP "psmux_starship_test_profile.ps1"
$starshipEscaped = $starshipPath -replace '\\', '\\'
Set-Content -Path $tempProfile -Value @"
# Temporary profile for Starship testing
`$env:STARSHIP_CONFIG = "$env:TEMP\psmux_starship_test.toml"
Invoke-Expression (& '$starshipPath' init powershell --print-full-init | Out-String)
"@

# Create a minimal Starship config (fast prompt, no network/git lookups)
$tempStarshipConfig = Join-Path $env:TEMP "psmux_starship_test.toml"
Set-Content -Path $tempStarshipConfig -Value @'
# Minimal Starship config for testing - fast prompt
command_timeout = 500
add_newline = false

[character]
success_symbol = "[STARSHIP_OK>](bold green)"
error_symbol = "[STARSHIP_ERR>](bold red)"

# Disable all modules except character to keep prompt fast
[aws]
disabled = true
[azure]
disabled = true
[battery]
disabled = true
[buf]
disabled = true
[bun]
disabled = true
[c]
disabled = true
[cmake]
disabled = true
[cmd_duration]
disabled = true
[cobol]
disabled = true
[conda]
disabled = true
[container]
disabled = true
[crystal]
disabled = true
[daml]
disabled = true
[dart]
disabled = true
[deno]
disabled = true
[directory]
disabled = true
[docker_context]
disabled = true
[dotnet]
disabled = true
[elixir]
disabled = true
[elm]
disabled = true
[env_var]
disabled = true
[erlang]
disabled = true
[fennel]
disabled = true
[fill]
disabled = true
[fossil_branch]
disabled = true
[fossil_metrics]
disabled = true
[gcloud]
disabled = true
[git_branch]
disabled = true
[git_commit]
disabled = true
[git_metrics]
disabled = true
[git_state]
disabled = true
[git_status]
disabled = true
[golang]
disabled = true
[gradle]
disabled = true
[guix_shell]
disabled = true
[haskell]
disabled = true
[haxe]
disabled = true
[helm]
disabled = true
[hostname]
disabled = true
[java]
disabled = true
[jobs]
disabled = true
[julia]
disabled = true
[kotlin]
disabled = true
[kubernetes]
disabled = true
[line_break]
disabled = true
[localip]
disabled = true
[lua]
disabled = true
[memory_usage]
disabled = true
[meson]
disabled = true
[hg_branch]
disabled = true
[nats]
disabled = true
[nim]
disabled = true
[nix_shell]
disabled = true
[nodejs]
disabled = true
[ocaml]
disabled = true
[odin]
disabled = true
[opa]
disabled = true
[openstack]
disabled = true
[os]
disabled = true
[package]
disabled = true
[perl]
disabled = true
[php]
disabled = true
[pijul_channel]
disabled = true
[pulumi]
disabled = true
[purescript]
disabled = true
[python]
disabled = true
[quarto]
disabled = true
[rlang]
disabled = true
[raku]
disabled = true
[red]
disabled = true
[ruby]
disabled = true
[rust]
disabled = true
[scala]
disabled = true
[shell]
disabled = true
[shlvl]
disabled = true
[singularity]
disabled = true
[solidity]
disabled = true
[spack]
disabled = true
[status]
disabled = true
[sudo]
disabled = true
[swift]
disabled = true
[terraform]
disabled = true
[time]
disabled = true
[typst]
disabled = true
[username]
disabled = true
[vagrant]
disabled = true
[vlang]
disabled = true
[vcsh]
disabled = true
[zig]
disabled = true
'@

Write-Info "Created temp Starship profile: $tempProfile"
Write-Info "Created temp Starship config: $tempStarshipConfig"

# ══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "ISSUE #111: Starship prompt compatibility"
Write-Host ("=" * 60)
# ══════════════════════════════════════════════════════════════════════

# --- Test 1: Starship prompt renders correctly inside psmux ---
Write-Test "1: Starship prompt renders inside psmux pane"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    # Source the Starship profile inside the pane
    & $PSMUX send-keys -t $SESSION ". '$tempProfile'" Enter
    Start-Sleep -Seconds 4

    # Send a command to get a fresh prompt
    & $PSMUX send-keys -t $SESSION "echo test_starship_render" Enter
    Start-Sleep -Seconds 3

    $cap = Capture-Pane $SESSION
    Write-Info "Captured pane output (first 500 chars): $($cap.Substring(0, [Math]::Min(500, $cap.Length)))"

    # Check if Starship prompt marker appears
    if ($cap -match "STARSHIP_OK>|STARSHIP_ERR>") {
        Write-Pass "1: Starship prompt renders correctly in psmux"
    } else {
        Write-Fail "1: Starship prompt NOT rendering. Expected STARSHIP_OK> or STARSHIP_ERR>. Captured:`n$($cap.Substring(0, [Math]::Min(800, $cap.Length)))"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "1: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 2: CWD sync still works with Starship prompt ---
Write-Test "2: CWD sync works with Starship prompt active"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    # Source the Starship profile
    & $PSMUX send-keys -t $SESSION ". '$tempProfile'" Enter
    Start-Sleep -Seconds 4

    # Create a test directory and cd to it
    $testDir = Join-Path $env:TEMP "psmux_starship_cwd_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    & $PSMUX send-keys -t $SESSION "cd `"$testDir`"" Enter
    Start-Sleep -Seconds 2

    # Check #{pane_current_path} via display-message
    $cwdResult = (& $PSMUX display-message -t $SESSION -p '#{pane_current_path}' 2>&1) | Out-String
    $cwdResult = $cwdResult.Trim()
    $dirName = Split-Path $testDir -Leaf

    Write-Info "pane_current_path: $cwdResult"

    if ($cwdResult -match [regex]::Escape($dirName)) {
        Write-Pass "2: #{pane_current_path} correct with Starship ($cwdResult)"
    } else {
        Write-Fail "2: #{pane_current_path} wrong. Expected dir containing '$dirName', got: $cwdResult"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "2: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    if ($testDir) { Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue }
}

# --- Test 3: split-window -c #{pane_current_path} works with Starship ---
Write-Test "3: split-window -c #{pane_current_path} with Starship"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    # Source the Starship profile
    & $PSMUX send-keys -t $SESSION ". '$tempProfile'" Enter
    Start-Sleep -Seconds 4

    # cd to test dir
    $testDir = Join-Path $env:TEMP "psmux_starship_split_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    & $PSMUX send-keys -t $SESSION "cd `"$testDir`"" Enter
    Start-Sleep -Seconds 2

    # Split pane using #{pane_current_path}
    & $PSMUX split-window -h -c '#{pane_current_path}' -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 5

    # Check CWD in new pane
    & $PSMUX send-keys -t $SESSION 'Write-Output "SPLIT_CWD=$($PWD.Path)"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION
    $capFlat = ($cap -replace "`r?`n", "")
    $dirName = Split-Path $testDir -Leaf

    if ($capFlat -match "SPLIT_CWD=.*$([regex]::Escape($dirName))") {
        Write-Pass "3: split-window with Starship preserved CWD"
    } else {
        Write-Fail "3: CWD not preserved with Starship. Got:`n$($cap.Substring(0, [Math]::Min(500, $cap.Length)))"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "3: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    if ($testDir) { Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue }
}

# --- Test 4: Starship prompt survives after multiple commands ---
Write-Test "4: Starship prompt persists after multiple commands"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    # Source the Starship profile
    & $PSMUX send-keys -t $SESSION ". '$tempProfile'" Enter
    Start-Sleep -Seconds 4

    # Run several commands
    & $PSMUX send-keys -t $SESSION "Get-Date" Enter
    Start-Sleep -Seconds 2
    & $PSMUX send-keys -t $SESSION "echo hello_world" Enter
    Start-Sleep -Seconds 2
    & $PSMUX send-keys -t $SESSION "1+1" Enter
    Start-Sleep -Seconds 2

    $cap = Capture-Pane $SESSION

    # Count Starship prompt markers — should have multiple
    $matches = [regex]::Matches($cap, "STARSHIP_OK>|STARSHIP_ERR>")
    Write-Info "Found $($matches.Count) Starship prompt markers"

    if ($matches.Count -ge 2) {
        Write-Pass "4: Starship prompt persists ($($matches.Count) renders)"
    } else {
        Write-Fail "4: Starship prompt not persisting. Only $($matches.Count) markers found. Captured:`n$($cap.Substring(0, [Math]::Min(800, $cap.Length)))"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "4: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 5: Module-scoped prompt (simulates Starship pattern without Starship binary) ---
Write-Test "5: Module-scoped prompt function preserved by CWD hook"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    # Create a module-scoped prompt (same pattern as Starship) directly in the pane
    # This tests the core issue: Get-Content + scriptblock::Create() loses module scope
    $moduleCode = @'
$null = New-Module test_prompt_module {
    function Get-TestPromptData { return "MODULE_SCOPE_OK" }
    function global:prompt {
        $data = Get-TestPromptData
        return "${data}> "
    }
    Export-ModuleMember -Function @()
}
'@
    # Send the module code line by line
    foreach ($line in ($moduleCode -split "`n")) {
        $trimmed = $line.TrimEnd("`r")
        if ($trimmed) {
            & $PSMUX send-keys -t $SESSION $trimmed Enter
            Start-Sleep -Milliseconds 200
        }
    }
    Start-Sleep -Seconds 3

    # Run a command to trigger the prompt
    & $PSMUX send-keys -t $SESSION "echo verify_module_prompt" Enter
    Start-Sleep -Seconds 3

    $cap = Capture-Pane $SESSION
    Write-Info "Captured: $($cap.Substring(0, [Math]::Min(500, $cap.Length)))"

    if ($cap -match "MODULE_SCOPE_OK>") {
        Write-Pass "5: Module-scoped prompt preserved by CWD hook"
    } else {
        Write-Fail "5: Module-scoped prompt BROKEN. Expected 'MODULE_SCOPE_OK>'. Captured:`n$($cap.Substring(0, [Math]::Min(800, $cap.Length)))"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "5: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
}

# --- Test 6: CWD sync after `. $PROFILE` reload with Starship ---
Write-Test "6: CWD sync survives profile reload with Starship"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    # Source the Starship profile
    & $PSMUX send-keys -t $SESSION ". '$tempProfile'" Enter
    Start-Sleep -Seconds 4

    # cd to test dir
    $testDir = Join-Path $env:TEMP "psmux_starship_reload_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    & $PSMUX send-keys -t $SESSION "cd `"$testDir`"" Enter
    Start-Sleep -Seconds 2

    # Reload profile (this is what the user reported breaks CWD sync)
    & $PSMUX send-keys -t $SESSION ". '$tempProfile'" Enter
    Start-Sleep -Seconds 4

    # cd to a different dir
    $testDir2 = Join-Path $env:TEMP "psmux_starship_reload2_$(Get-Random)"
    New-Item -Path $testDir2 -ItemType Directory -Force | Out-Null

    & $PSMUX send-keys -t $SESSION "cd `"$testDir2`"" Enter
    Start-Sleep -Seconds 2

    $cwdResult = (& $PSMUX display-message -t $SESSION -p '#{pane_current_path}' 2>&1) | Out-String
    $cwdResult = $cwdResult.Trim()
    $dirName2 = Split-Path $testDir2 -Leaf

    Write-Info "After profile reload + cd, pane_current_path: $cwdResult"

    if ($cwdResult -match [regex]::Escape($dirName2)) {
        Write-Pass "6: CWD sync survived profile reload ($cwdResult)"
    } else {
        Write-Fail "6: CWD sync broken after profile reload. Expected '$dirName2', got: $cwdResult"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "6: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    if ($testDir) { Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue }
    if ($testDir2) { Remove-Item $testDir2 -Recurse -Force -ErrorAction SilentlyContinue }
}

# ══════════════════════════════════════════════════════════════════════
# Cleanup
# ══════════════════════════════════════════════════════════════════════
Write-Info "Final cleanup..."
& $PSMUX kill-server 2>$null
Remove-Item $tempProfile -Force -ErrorAction SilentlyContinue
Remove-Item $tempStarshipConfig -Force -ErrorAction SilentlyContinue

Write-Host ""
Write-Host ("=" * 60)
$total = $script:TestsPassed + $script:TestsFailed + $script:TestsSkipped
Write-Host "Results: $script:TestsPassed passed, $script:TestsFailed failed, $script:TestsSkipped skipped / $total total"
if ($script:TestsFailed -gt 0) {
    Write-Host "SOME TESTS FAILED" -ForegroundColor Red
    exit 1
} else {
    Write-Host "ALL TESTS PASSED" -ForegroundColor Green
    exit 0
}
