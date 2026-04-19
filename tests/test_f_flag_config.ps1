# psmux Issue #119 -- `-f <file>` no longer works as a global option
#
# Tests that -f <file> is correctly parsed as the alternate config file.
#
# Run: pwsh -NoProfile -ExecutionPolicy Bypass -File tests\test_f_flag_config.ps1

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

$SESSION = "test_119"

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

# ======================================================================
Write-Host ""
Write-Host ("=" * 70)
Write-Host "ISSUE #119: -f <file> as global config option"
Write-Host ("=" * 70)
# ======================================================================

# --- Test 1: -f /dev/null should not error with "unknown command" ---
Write-Test "1: -f NUL does not produce 'unknown command' error"
try {
    # Use NUL on Windows (equivalent of /dev/null)
    $output = & $PSMUX -f NUL new-session -d -s $SESSION 2>&1 | Out-String
    $exitCode = $LASTEXITCODE

    if ($output -match "unknown command.*NUL|unknown command.*/dev/null") {
        Write-Fail "1: -f argument treated as command name: $output"
    } else {
        # Check if session was created (meaning -f was parsed correctly)
        if (Wait-ForSession $SESSION 8) {
            Write-Pass "1: -f NUL parsed correctly, session created"
        } else {
            # Even if session didn't start for other reasons, the important
            # thing is -f didn't cause an "unknown command" error
            if ($exitCode -eq 0 -or -not ($output -match "unknown command")) {
                Write-Pass "1: -f NUL parsed correctly (no 'unknown command' error)"
            } else {
                Write-Fail "1: -f NUL failed: exit=$exitCode output=$output"
            }
        }
    }
} catch {
    Write-Fail "1: Exception: $_"
} finally {
    Cleanup-Session $SESSION
}

# --- Test 2: -f with a real config file applies settings ---
Write-Test "2: -f <file> loads config from the specified file"
try {
    # Ensure no server is running so the new session spawns a fresh server
    & $PSMUX kill-server 2>$null
    Start-Sleep -Seconds 3
    Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
    Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

    $configDir = Join-Path $env:TEMP "psmux_test_119_$(Get-Random)"
    New-Item -Path $configDir -ItemType Directory -Force | Out-Null
    $configFile = Join-Path $configDir "test.conf"
    # A config that sets a known option
    Set-Content -Path $configFile -Value 'set -g status-right "TEST119OK"'

    Start-Process -FilePath $PSMUX -ArgumentList "-f `"$configFile`" new-session -d -s $SESSION" -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION 10)) {
        Write-Fail "2: Could not create session with -f"
    } else {
        Start-Sleep -Seconds 2
        $val = & $PSMUX show-options -t $SESSION -g 2>&1 | Out-String
        if ($val -match "TEST119OK") {
            Write-Pass "2: -f loaded config file and applied settings"
        } else {
            Write-Fail "2: Config setting not applied. show-options output:`n$val"
        }
    }
} catch {
    Write-Fail "2: Exception: $_"
} finally {
    Cleanup-Session $SESSION
    Remove-Item $configDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 3: -f with empty config file (no settings) still starts session ---
Write-Test "3: -f with empty file starts session without errors"
try {
    & $PSMUX kill-server 2>$null
    Start-Sleep -Seconds 3
    Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
    Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

    $configDir = Join-Path $env:TEMP "psmux_test_119e_$(Get-Random)"
    New-Item -Path $configDir -ItemType Directory -Force | Out-Null
    $configFile = Join-Path $configDir "empty.conf"
    Set-Content -Path $configFile -Value ""

    Start-Process -FilePath $PSMUX -ArgumentList "-f `"$configFile`" new-session -d -s $SESSION" -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION 10)) {
        Write-Fail "3: Could not create session with -f empty.conf"
    } else {
        Write-Pass "3: -f with empty config starts session fine"
    }
} catch {
    Write-Fail "3: Exception: $_"
} finally {
    Cleanup-Session $SESSION
    Remove-Item $configDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 4: -f before various subcommands ---
Write-Test "4: -f works before different subcommands"
try {
    & $PSMUX kill-server 2>$null
    Start-Sleep -Seconds 3
    Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
    Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

    # Start a session first
    Start-Process -FilePath $PSMUX -ArgumentList "new-session -d -s $SESSION" -WindowStyle Hidden
    if (-not (Wait-ForSession $SESSION 10)) {
        Write-Fail "4: Could not create test session"
        throw "skip"
    }
    Start-Sleep -Seconds 2

    $configDir = Join-Path $env:TEMP "psmux_test_119s_$(Get-Random)"
    New-Item -Path $configDir -ItemType Directory -Force | Out-Null
    $configFile = Join-Path $configDir "test.conf"
    Set-Content -Path $configFile -Value ""

    # Test -f with list-sessions
    $lsOutput = & $PSMUX -f $configFile list-sessions 2>&1 | Out-String
    if ($lsOutput -match "unknown command") {
        Write-Fail "4: -f broke list-sessions: $lsOutput"
    } elseif ($lsOutput -match $SESSION) {
        Write-Pass "4: -f works before list-sessions"
    } else {
        Write-Fail "4: list-sessions output unexpected: $lsOutput"
    }
} catch {
    if ($_.ToString() -ne "skip") { Write-Fail "4: Exception: $_" }
} finally {
    Cleanup-Session $SESSION
    Remove-Item $configDir -Recurse -Force -ErrorAction SilentlyContinue
}

# --- Test 5: -f combined with -L still works ---
Write-Test "5: -f combined with -L flag"
try {
    & $PSMUX kill-server 2>$null
    Start-Sleep -Seconds 3
    Remove-Item "$env:USERPROFILE\.psmux\*.port" -Force -ErrorAction SilentlyContinue
    Remove-Item "$env:USERPROFILE\.psmux\*.key" -Force -ErrorAction SilentlyContinue

    $configDir = Join-Path $env:TEMP "psmux_test_119c_$(Get-Random)"
    New-Item -Path $configDir -ItemType Directory -Force | Out-Null
    $configFile = Join-Path $configDir "test.conf"
    Set-Content -Path $configFile -Value ""

    $nsSession = "test_119_ns"
    Start-Process -FilePath $PSMUX -ArgumentList "-f `"$configFile`" -L test119ns new-session -d -s $nsSession" -WindowStyle Hidden
    if (-not (Wait-ForSession "test119ns__$nsSession" 10)) {
        # Try without namespace prefix
        if (-not (Wait-ForSession $nsSession 10)) {
            Write-Fail "5: Could not create session with -f + -L"
        } else {
            Write-Pass "5: -f + -L works"
        }
    } else {
        Write-Pass "5: -f + -L works"
    }
} catch {
    Write-Fail "5: Exception: $_"
} finally {
    & $PSMUX -L test119ns kill-session -t $nsSession 2>$null
    Cleanup-Session $nsSession
    Remove-Item $configDir -Recurse -Force -ErrorAction SilentlyContinue
}

# ======================================================================
# Final cleanup
& $PSMUX kill-server 2>$null
Start-Sleep -Seconds 1

Write-Host ""
Write-Host ("=" * 70)
Write-Host "Results: $($script:TestsPassed) passed, $($script:TestsFailed) failed, $($script:TestsSkipped) skipped"
Write-Host ("=" * 70)
if ($script:TestsFailed -gt 0) { exit 1 } else { exit 0 }
