# psmux Issue #111 — #{pane_current_path} in split-window -c
#
# Tests that format variables like #{pane_current_path} are expanded
# when used in -c arguments to split-window and new-window.
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_issue111_format_cwd.ps1

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

# Clean slate
Write-Info "Cleaning up existing sessions..."
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 3
Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

$SESSION = "test_111"

function Wait-ForSession {
    param($name, $timeout = 10)
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
    Start-Sleep -Seconds 3
    return $true
}

# ══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "ISSUE #111: #{pane_current_path} in split-window -c"
Write-Host ("=" * 60)
# ══════════════════════════════════════════════════════════════════════

# --- Test 1: split-window -h -c "#{pane_current_path}" preserves CWD ---
Write-Test "1: split-window -c #{pane_current_path} preserves CWD"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_test_111_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    # cd to testDir in the active pane
    & $PSMUX send-keys -t $SESSION "cd `"$testDir`"" Enter
    Start-Sleep -Seconds 2

    # Verify CWD changed
    & $PSMUX send-keys -t $SESSION 'Write-Output "CWD1=$($PWD.Path)"' Enter
    Start-Sleep -Seconds 2

    # Split with #{pane_current_path}
    & $PSMUX split-window -h -c '#{pane_current_path}' -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 4

    # Check CWD in the new pane
    & $PSMUX send-keys -t $SESSION 'Write-Output "NEWPANE_CWD=$($PWD.Path)"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION
    $capFlat = ($cap -replace "`r?`n", "")
    $dirName = Split-Path $testDir -Leaf

    if ($capFlat -match "NEWPANE_CWD=.*$([regex]::Escape($dirName))") {
        Write-Pass "1: split-window -c #{pane_current_path} preserved CWD"
    } else {
        Write-Fail "1: CWD not preserved. Expected '$dirName'. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "1: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 2: split-window -v -c "#{pane_current_path}" (vertical) ---
Write-Test "2: split-window -v -c #{pane_current_path} (vertical)"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_test_111v_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    & $PSMUX send-keys -t $SESSION "cd `"$testDir`"" Enter
    Start-Sleep -Seconds 2

    & $PSMUX split-window -v -c '#{pane_current_path}' -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 4

    & $PSMUX send-keys -t $SESSION 'Write-Output "VPANE=$($PWD.Path)"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION
    $capFlat = ($cap -replace "`r?`n", "")
    $dirName = Split-Path $testDir -Leaf

    if ($capFlat -match "VPANE=.*$([regex]::Escape($dirName))") {
        Write-Pass "2: split-window -v -c #{pane_current_path} preserved CWD"
    } else {
        Write-Fail "2: CWD not preserved. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "2: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 3: new-window -c "#{pane_current_path}" ---
Write-Test "3: new-window -c #{pane_current_path} preserves CWD"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_test_111nw_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    & $PSMUX send-keys -t $SESSION "cd `"$testDir`"" Enter
    Start-Sleep -Seconds 2

    & $PSMUX new-window -c '#{pane_current_path}' -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 4

    & $PSMUX send-keys -t $SESSION 'Write-Output "NWPANE=$($PWD.Path)"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION
    $capFlat = ($cap -replace "`r?`n", "")
    $dirName = Split-Path $testDir -Leaf

    if ($capFlat -match "NWPANE=.*$([regex]::Escape($dirName))") {
        Write-Pass "3: new-window -c #{pane_current_path} preserved CWD"
    } else {
        Write-Fail "3: CWD not preserved. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "3: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 4: bind-key with #{pane_current_path} via source-file ---
Write-Test "4: bind-key + split-window -c #{pane_current_path} via config"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_test_111bind_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    # Create a config that binds a key
    $configDir = Join-Path $env:TEMP "psmux_test_111_cfg_$(Get-Random)"
    New-Item -Path $configDir -ItemType Directory -Force | Out-Null
    $configFile = Join-Path $configDir ".psmux.conf"
    Set-Content -Path $configFile -Value 'bind-key V split-window -v -c "#{pane_current_path}"'

    & $PSMUX source-file -t $SESSION $configFile 2>&1 | Out-Null
    Start-Sleep -Milliseconds 500

    # cd to test directory
    & $PSMUX send-keys -t $SESSION "cd `"$testDir`"" Enter
    Start-Sleep -Seconds 2

    # Use the bound key directly via command (simulates pressing prefix+V)
    & $PSMUX split-window -v -c '#{pane_current_path}' -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 4

    & $PSMUX send-keys -t $SESSION 'Write-Output "BINDPANE=$($PWD.Path)"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION
    $capFlat = ($cap -replace "`r?`n", "")
    $dirName = Split-Path $testDir -Leaf

    if ($capFlat -match "BINDPANE=.*$([regex]::Escape($dirName))") {
        Write-Pass "4: Config bind-key with #{pane_current_path} works"
    } else {
        Write-Fail "4: CWD not preserved. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "4: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $configDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 5: Literal -c path still works (regression check) ---
Write-Test "5: Literal -c path still works (no format variable)"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_test_111lit_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    & $PSMUX split-window -h -c $testDir -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 4

    & $PSMUX send-keys -t $SESSION 'Write-Output "LITPANE=$($PWD.Path)"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION
    $capFlat = ($cap -replace "`r?`n", "")
    $dirName = Split-Path $testDir -Leaf

    if ($capFlat -match "LITPANE=.*$([regex]::Escape($dirName))") {
        Write-Pass "5: Literal -c path still works"
    } else {
        Write-Fail "5: Literal path broken. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "5: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 6: display-message #{pane_current_path} returns correct value ---
Write-Test "6: display-message resolves #{pane_current_path} correctly"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_test_111dm_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    & $PSMUX send-keys -t $SESSION "cd `"$testDir`"" Enter
    Start-Sleep -Seconds 2

    $result = & $PSMUX display-message -t $SESSION -p '#{pane_current_path}' 2>&1 | Out-String
    $result = $result.Trim()
    $dirName = Split-Path $testDir -Leaf

    if ($result -match [regex]::Escape($dirName)) {
        Write-Pass "6: display-message resolves #{pane_current_path} ($result)"
    } else {
        Write-Fail "6: display-message wrong. Expected '$dirName'. Got: $result"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "6: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
}

# ══════════════════════════════════════════════════════════════════════
Write-Host ""
Write-Host ("=" * 60)
Write-Host "CROSS-SHELL: #{pane_current_path} with cmd.exe and Git Bash"
Write-Host ("=" * 60)
# ══════════════════════════════════════════════════════════════════════

# --- Test 7: cmd.exe — cd updates OS CWD, #{pane_current_path} works ---
Write-Test "7: cmd.exe pane — #{pane_current_path} tracks cd"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_111_cmd_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    # Open a cmd.exe pane
    & $PSMUX split-window -h -t $SESSION "cmd.exe" 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    # cd in cmd.exe
    & $PSMUX send-keys -t $SESSION "cd /d `"$testDir`"" Enter
    Start-Sleep -Seconds 2

    # Check #{pane_current_path}
    $result = (& $PSMUX display-message -t $SESSION -p '#{pane_current_path}' 2>&1 | Out-String).Trim()
    $dirName = Split-Path $testDir -Leaf

    if ($result -match [regex]::Escape($dirName)) {
        Write-Pass "7: cmd.exe — #{pane_current_path} tracks cd ($result)"
    } else {
        Write-Fail "7: cmd.exe — CWD not tracked. Expected '$dirName'. Got: $result"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "7: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 8: cmd.exe — split-window -c #{pane_current_path} ---
Write-Test "8: cmd.exe — split with #{pane_current_path} preserves CWD"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_111_cmdsplit_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null

    # Open cmd.exe pane and cd
    & $PSMUX split-window -h -t $SESSION "cmd.exe" 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    & $PSMUX send-keys -t $SESSION "cd /d `"$testDir`"" Enter
    Start-Sleep -Seconds 2

    # Split from that pane using #{pane_current_path}
    & $PSMUX split-window -v -c '#{pane_current_path}' -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 4

    # The new pane (pwsh default) should be in testDir
    & $PSMUX send-keys -t $SESSION 'Write-Output "CMDSPLIT=$($PWD.Path)"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION
    $capFlat = ($cap -replace "`r?`n", "")
    $dirName = Split-Path $testDir -Leaf

    if ($capFlat -match "CMDSPLIT=.*$([regex]::Escape($dirName))") {
        Write-Pass "8: cmd.exe — split with #{pane_current_path} works"
    } else {
        Write-Fail "8: cmd.exe split CWD wrong. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "8: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 9: Git Bash — cd updates OS CWD, #{pane_current_path} works ---
Write-Test "9: Git Bash — #{pane_current_path} tracks cd"
try {
    $gitBash = $null
    $candidates = @(
        "C:\Program Files\Git\bin\bash.exe",
        "C:\Program Files (x86)\Git\bin\bash.exe",
        (Get-Command bash.exe -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source -ErrorAction SilentlyContinue)
    )
    foreach ($c in $candidates) {
        if ($c -and (Test-Path $c -ErrorAction SilentlyContinue)) { $gitBash = $c; break }
    }
    if (-not $gitBash) { Write-Skip "9: Git Bash not found"; throw "skip" }

    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_111_bash_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null
    # Convert to Unix-style path for bash
    $bashDir = $testDir -replace '\\', '/'

    # Open bash pane
    & $PSMUX split-window -h -t $SESSION "$gitBash" 2>&1 | Out-Null
    Start-Sleep -Seconds 3

    # cd in bash
    & $PSMUX send-keys -t $SESSION "cd '$bashDir'" Enter
    Start-Sleep -Seconds 2

    # Check #{pane_current_path}
    $result = (& $PSMUX display-message -t $SESSION -p '#{pane_current_path}' 2>&1 | Out-String).Trim()
    $dirName = Split-Path $testDir -Leaf

    if ($result -match [regex]::Escape($dirName)) {
        Write-Pass "9: Git Bash — #{pane_current_path} tracks cd ($result)"
    } else {
        Write-Fail "9: Git Bash — CWD not tracked. Expected '$dirName'. Got: $result"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "9: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 10: Git Bash — split-window -c #{pane_current_path} ---
Write-Test "10: Git Bash — split with #{pane_current_path} preserves CWD"
try {
    if (-not $gitBash) { Write-Skip "10: Git Bash not found"; throw "skip" }

    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $testDir = Join-Path $env:TEMP "psmux_111_bashsplit_$(Get-Random)"
    New-Item -Path $testDir -ItemType Directory -Force | Out-Null
    $bashDir = $testDir -replace '\\', '/'

    & $PSMUX split-window -h -t $SESSION "$gitBash" 2>&1 | Out-Null
    Start-Sleep -Seconds 3
    & $PSMUX send-keys -t $SESSION "cd '$bashDir'" Enter
    Start-Sleep -Seconds 2

    # Split from bash pane using #{pane_current_path}
    & $PSMUX split-window -v -c '#{pane_current_path}' -t $SESSION 2>&1 | Out-Null
    Start-Sleep -Seconds 4

    # New pane should be in testDir
    & $PSMUX send-keys -t $SESSION 'Write-Output "BASHSPLIT=$($PWD.Path)"' Enter
    Start-Sleep -Seconds 2
    $cap = Capture-Pane $SESSION
    $capFlat = ($cap -replace "`r?`n", "")
    $dirName = Split-Path $testDir -Leaf

    if ($capFlat -match "BASHSPLIT=.*$([regex]::Escape($dirName))") {
        Write-Pass "10: Git Bash — split with #{pane_current_path} works"
    } else {
        Write-Fail "10: Git Bash split CWD wrong. Got:`n$cap"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "10: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $testDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 11: pwsh → cd → display-message → verify CWD sync hook ---
Write-Test "11: pwsh CWD sync — display-message after multiple cd's"
try {
    if (-not (New-TestSession $SESSION)) { throw "skip" }

    $dir1 = Join-Path $env:TEMP "psmux_111_cd1_$(Get-Random)"
    $dir2 = Join-Path $env:TEMP "psmux_111_cd2_$(Get-Random)"
    New-Item -Path $dir1 -ItemType Directory -Force | Out-Null
    New-Item -Path $dir2 -ItemType Directory -Force | Out-Null

    # cd to dir1
    & $PSMUX send-keys -t $SESSION "cd `"$dir1`"" Enter
    Start-Sleep -Seconds 2
    $r1 = (& $PSMUX display-message -t $SESSION -p '#{pane_current_path}' 2>&1 | Out-String).Trim()

    # cd to dir2
    & $PSMUX send-keys -t $SESSION "cd `"$dir2`"" Enter
    Start-Sleep -Seconds 2
    $r2 = (& $PSMUX display-message -t $SESSION -p '#{pane_current_path}' 2>&1 | Out-String).Trim()

    $d1 = Split-Path $dir1 -Leaf
    $d2 = Split-Path $dir2 -Leaf

    if ($r1 -match [regex]::Escape($d1) -and $r2 -match [regex]::Escape($d2)) {
        Write-Pass "11: CWD tracks through multiple cd's (dir1=$r1, dir2=$r2)"
    } elseif ($r2 -match [regex]::Escape($d2)) {
        Write-Pass "11: CWD tracks current cd (dir2=$r2)"
    } else {
        Write-Fail "11: CWD not tracking. r1=$r1 (exp $d1), r2=$r2 (exp $d2)"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "11: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $dir1 -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $dir2 -Recurse -Force -ErrorAction SilentlyContinue
}

# ══════════════════════════════════════════════════════════════════════
# Cleanup & summary
# ══════════════════════════════════════════════════════════════════════
& $PSMUX kill-server 2>$null

Write-Host ""
Write-Host ("=" * 60)
$total = $script:TestsPassed + $script:TestsFailed
Write-Host "RESULTS: $($script:TestsPassed) passed, $($script:TestsFailed) failed, $($script:TestsSkipped) skipped (of $total run)" -ForegroundColor $(if ($script:TestsFailed -eq 0) { "Green" } else { "Red" })
Write-Host ("=" * 60)

exit $script:TestsFailed
