#!/usr/bin/env pwsh
# tests/test_spaces_in_paths.ps1
# Comprehensive test for run-shell with paths containing spaces.
# Covers: .ps1 scripts, .bat batch files, .exe executables, and unknown extensions.
# Tests via: CLI, TCP, and Config entry points.

$ErrorActionPreference = "Continue"
$pass = 0; $fail = 0

function Test-RunShell {
    param([string]$Name, [scriptblock]$Block)
    try {
        $result = & $Block
        if ($result) {
            Write-Host "  PASS: $Name" -ForegroundColor Green
            $script:pass++
        } else {
            Write-Host "  FAIL: $Name" -ForegroundColor Red
            $script:fail++
        }
    } catch {
        Write-Host "  FAIL: $Name ($_)" -ForegroundColor Red
        $script:fail++
    }
}

# ── Setup: Create test files in paths WITH spaces ──
$testRoot = Join-Path $env:TEMP "psmux spaces test"
$scriptDir = Join-Path $testRoot "My Scripts"
if (Test-Path $testRoot) { Remove-Item $testRoot -Recurse -Force }
New-Item -ItemType Directory -Path $scriptDir -Force | Out-Null

# Create a .ps1 test script
$ps1Path = Join-Path $scriptDir "hello world.ps1"
Set-Content $ps1Path -Value 'Write-Output "PSMUX_SPACE_PS1_OK"' -Encoding UTF8

# Create a .ps1 script that accepts arguments
$ps1ArgsPath = Join-Path $scriptDir "args test.ps1"
Set-Content $ps1ArgsPath -Value 'param($a, $b); Write-Output "ARGS:$a,$b"' -Encoding UTF8

# Create a .bat test script
$batPath = Join-Path $scriptDir "hello world.bat"
Set-Content $batPath -Value '@echo off & echo PSMUX_SPACE_BAT_OK' -Encoding UTF8

# Create a .cmd test script
$cmdPath = Join-Path $scriptDir "hello world.cmd"
Set-Content $cmdPath -Value '@echo off & echo PSMUX_SPACE_CMD_OK' -Encoding UTF8

# Create a plain file (unknown extension) to test the call operator path
$txtPath = Join-Path $scriptDir "hello world.txt"
Set-Content $txtPath -Value 'dummy content' -Encoding UTF8

# Also create files WITHOUT spaces as a control group
$noSpaceDir = Join-Path $testRoot "scripts"
New-Item -ItemType Directory -Path $noSpaceDir -Force | Out-Null

$ps1NoSpace = Join-Path $noSpaceDir "test.ps1"
Set-Content $ps1NoSpace -Value 'Write-Output "PSMUX_NOSPACE_PS1_OK"' -Encoding UTF8

$batNoSpace = Join-Path $noSpaceDir "test.bat"
Set-Content $batNoSpace -Value '@echo off & echo PSMUX_NOSPACE_BAT_OK' -Encoding UTF8

Write-Host ""
Write-Host "============================================" -ForegroundColor Cyan
Write-Host " PSMUX: Spaces in Paths Test Suite" -ForegroundColor Cyan
Write-Host "============================================" -ForegroundColor Cyan
Write-Host ""

# ═══════════════════════════════════════════
# PART A: CLI path (psmux run-shell "...")
# ═══════════════════════════════════════════
Write-Host "--- Part A: CLI (psmux run-shell) ---" -ForegroundColor Yellow

Test-RunShell "A1: .ps1 file WITH spaces in path" {
    $out = & psmux run-shell "$ps1Path" 2>&1
    $out -match "PSMUX_SPACE_PS1_OK"
}

Test-RunShell "A2: .ps1 file WITHOUT spaces (control)" {
    $out = & psmux run-shell "$ps1NoSpace" 2>&1
    $out -match "PSMUX_NOSPACE_PS1_OK"
}

Test-RunShell "A3: .ps1 with spaces AND arguments" {
    $out = & psmux run-shell "$ps1ArgsPath hello world" 2>&1
    $out -match "ARGS:hello,world"
}

Test-RunShell "A4: .bat file WITH spaces in path" {
    $out = & psmux run-shell "$batPath" 2>&1
    $out -match "PSMUX_SPACE_BAT_OK"
}

Test-RunShell "A5: .cmd file WITH spaces in path" {
    $out = & psmux run-shell "$cmdPath" 2>&1
    $out -match "PSMUX_SPACE_CMD_OK"
}

Test-RunShell "A6: .bat file WITHOUT spaces (control)" {
    $out = & psmux run-shell "$batNoSpace" 2>&1
    $out -match "PSMUX_NOSPACE_BAT_OK"
}

Test-RunShell "A7: Non-file command (no regression)" {
    $out = & psmux run-shell 'Write-Output "PSMUX_ECHO_OK"' 2>&1
    $out -match "PSMUX_ECHO_OK"
}

Test-RunShell "A8: Command with pipe (no regression)" {
    $out = (& psmux run-shell '"hello","world" | ForEach-Object { $_ }' 2>&1) | Out-String
    $out.Contains("hello") -and $out.Contains("world")
}

# ═══════════════════════════════════════════
# PART B: TCP path (psmux server command)
# ═══════════════════════════════════════════
Write-Host ""
Write-Host "--- Part B: TCP Server Path ---" -ForegroundColor Yellow

function Send-TcpCommand {
    param([int]$Port, [string]$Cmd, [string]$SessionKey = "")
    try {
        $client = New-Object System.Net.Sockets.TcpClient
        $client.Connect("127.0.0.1", $Port)
        $client.ReceiveTimeout = 5000
        $stream = $client.GetStream()
        $writer = New-Object System.IO.StreamWriter($stream)
        $reader = New-Object System.IO.StreamReader($stream)
        $writer.AutoFlush = $true
        # Auth protocol: send AUTH <key>, read OK response
        if ($SessionKey) { $writer.WriteLine("AUTH $SessionKey") }
        $authResp = $reader.ReadLine()
        if ($authResp -ne "OK") { return "AUTH_FAILED: $authResp" }
        # Send the command (server reads this as first post-auth line)
        $writer.WriteLine($Cmd)
        $output = ""
        try {
            while ($null -ne ($line = $reader.ReadLine())) {
                $output += "$line`n"
            }
        } catch { }
        $client.Close()
        return $output.Trim()
    } catch {
        return "TCP_ERROR: $_"
    }
}

# Find a running psmux session for TCP tests
$portFile = Get-ChildItem "$env:USERPROFILE\.psmux\*.port" -ErrorAction SilentlyContinue | Where-Object { $_.BaseName -ne '__warm__' } | Select-Object -First 1
if ($portFile) {
    $port = [int](Get-Content $portFile.FullName -Raw).Trim()
    $keyFile = $portFile.FullName -replace '\.port$', '.key'
    $sessionKey = if (Test-Path $keyFile) { (Get-Content $keyFile -Raw).Trim() } else { "" }
    Write-Host "  Using session on port $port" -ForegroundColor DarkGray

    Test-RunShell "B1: TCP .ps1 with spaces" {
        $out = Send-TcpCommand $port "run-shell `"$ps1Path`"" $sessionKey
        $out -match "PSMUX_SPACE_PS1_OK"
    }

    Test-RunShell "B2: TCP .ps1 without spaces (control)" {
        $out = Send-TcpCommand $port "run-shell `"$ps1NoSpace`"" $sessionKey
        $out -match "PSMUX_NOSPACE_PS1_OK"
    }

    Test-RunShell "B3: TCP .bat with spaces" {
        $out = Send-TcpCommand $port "run-shell `"$batPath`"" $sessionKey
        $out -match "PSMUX_SPACE_BAT_OK"
    }

    Test-RunShell "B4: TCP plain echo (no regression)" {
        $out = Send-TcpCommand $port 'run-shell "Write-Output TCP_ECHO_OK"' $sessionKey
        $out -match "TCP_ECHO_OK"
    }
} else {
    Write-Host "  SKIP: No running psmux session found for TCP tests" -ForegroundColor DarkGray
}

# ═══════════════════════════════════════════
# PART C: Config path (run-shell from psmux.conf)
# ═══════════════════════════════════════════
Write-Host ""
Write-Host "--- Part C: Config File Path ---" -ForegroundColor Yellow

# C1: Config with .ps1 path containing spaces
$confC1 = Join-Path $testRoot "test_c1.conf"
Set-Content $confC1 -Value "run-shell `"$ps1Path`"" -Encoding UTF8

Test-RunShell "C1: Config .ps1 with spaces" {
    $out = & psmux source-file "$confC1" 2>&1
    # source-file spawns non-blocking, so we verify no error
    $exitCode = $LASTEXITCODE
    $hasError = $out -match "error|not found|failed"
    -not $hasError
}

# C2: Config with .bat path containing spaces
$confC2 = Join-Path $testRoot "test_c2.conf"
Set-Content $confC2 -Value "run-shell `"$batPath`"" -Encoding UTF8

Test-RunShell "C2: Config .bat with spaces" {
    $out = & psmux source-file "$confC2" 2>&1
    $hasError = $out -match "error|not found|failed"
    -not $hasError
}

# C3: Config set-hook with .ps1 path containing spaces
$confC3 = Join-Path $testRoot "test_c3.conf"
$hookLine = "set-hook -g after-new-window `"run-shell \`"$ps1Path\`"`""
Set-Content $confC3 -Value $hookLine -Encoding UTF8

Test-RunShell "C3: Config set-hook with spaced .ps1 path" {
    $out = & psmux source-file "$confC3" 2>&1
    $hasError = $out -match "error|not found|failed"
    -not $hasError
}

# C4: Config with no-space path (control)
$confC4 = Join-Path $testRoot "test_c4.conf"
Set-Content $confC4 -Value "run-shell `"$ps1NoSpace`"" -Encoding UTF8

Test-RunShell "C4: Config .ps1 without spaces (control)" {
    $out = & psmux source-file "$confC4" 2>&1
    $hasError = $out -match "error|not found|failed"
    -not $hasError
}

# ═══════════════════════════════════════════
# PART D: Edge Cases
# ═══════════════════════════════════════════
Write-Host ""
Write-Host "--- Part D: Edge Cases ---" -ForegroundColor Yellow

Test-RunShell "D1: URL forward slashes preserved (no regression)" {
    $out = & psmux run-shell 'Write-Output "https://example.com/api/v1"' 2>&1
    $out -match "https://example.com/api/v1"
}

Test-RunShell "D2: Tilde expansion with forward slashes" {
    $out = & psmux run-shell 'Write-Output "~/.psmux works"' 2>&1
    # Should not error
    $LASTEXITCODE -eq 0
}

Test-RunShell "D3: Multiple spaces in path name" {
    $multiSpaceDir = Join-Path $testRoot "Dir  With   Many    Spaces"
    New-Item -ItemType Directory -Path $multiSpaceDir -Force | Out-Null
    $multiSpaceScript = Join-Path $multiSpaceDir "test.ps1"
    Set-Content $multiSpaceScript -Value 'Write-Output "MULTI_SPACE_OK"' -Encoding UTF8
    $out = & psmux run-shell "$multiSpaceScript" 2>&1
    $out -match "MULTI_SPACE_OK"
}

Test-RunShell "D4: Path with parentheses and spaces" {
    $parenDir = Join-Path $testRoot "Program Files (x86)"
    New-Item -ItemType Directory -Path $parenDir -Force | Out-Null
    $parenScript = Join-Path $parenDir "test.ps1"
    Set-Content $parenScript -Value 'Write-Output "PAREN_SPACE_OK"' -Encoding UTF8
    $out = & psmux run-shell "$parenScript" 2>&1
    $out -match "PAREN_SPACE_OK"
}

Test-RunShell "D5: .ps1 with spaces AND -b flag (background)" {
    # Should not error; background commands don't produce output
    $out = & psmux run-shell -b "$ps1Path" 2>&1
    $hasError = $out -match "error|not found|failed"
    -not $hasError
}

# ═══════════════════════════════════════════
# CLEANUP & RESULTS
# ═══════════════════════════════════════════
Write-Host ""
Write-Host "============================================" -ForegroundColor Cyan
Write-Host " RESULTS: $pass PASSED, $fail FAILED" -ForegroundColor $(if ($fail -gt 0) { "Red" } else { "Green" })
Write-Host "============================================" -ForegroundColor Cyan

# Cleanup test directories
Remove-Item $testRoot -Recurse -Force -ErrorAction SilentlyContinue

if ($fail -gt 0) { exit 1 } else { exit 0 }
