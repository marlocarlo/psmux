$bin = (Resolve-Path "$PSScriptRoot\..\target\release\psmux.exe" -ErrorAction SilentlyContinue).Path
if (-not $bin) { $bin = (Resolve-Path "$PSScriptRoot\..\target\debug\psmux.exe" -ErrorAction SilentlyContinue).Path }
if (-not $bin) { $bin = "psmux" }
$pass_count = 0
$fail_count = 0

# Setup: create the fmttest session with a split pane
& $bin kill-session -t fmttest 2>$null
& $bin new-session -d -s fmttest 2>&1 | Out-Null
Start-Sleep -Seconds 2
& $bin split-window -t fmttest -h 2>&1 | Out-Null
Start-Sleep -Seconds 1

function Run-Test($num, $label, $format, $target, $validator) {
    $raw = & $bin display-message -t $target -p $format 2>&1
    $o = ($raw | Out-String).Trim()
    $ok = & $validator $o
    if ($ok) {
        $script:pass_count++
        Write-Host "PASS  #$num $label = '$o'"
    } else {
        $script:fail_count++
        Write-Host "FAIL  #$num $label = '$o'"
    }
}

# 1
Run-Test 1 "session_name" '#{session_name}' "fmttest" { param($v) $v -eq 'fmttest' }

# 2
Run-Test 2 "session_id" '#{session_id}' "fmttest" { param($v) $v -match '^\$\d+$' }

# 3
Run-Test 3 "window_index" '#{window_index}' "fmttest" { param($v) $v -match '^\d+$' }

# 4
Run-Test 4 "window_name" '#{window_name}' "fmttest" { param($v) $v.Length -gt 0 }

# 5
Run-Test 5 "window_id" '#{window_id}' "fmttest" { param($v) $v -match '^@\d+$' }

# 6
Run-Test 6 "pane_index" '#{pane_index}' "fmttest" { param($v) $v -match '^\d+$' }

# 7
Run-Test 7 "pane_id" '#{pane_id}' "fmttest" { param($v) $v -match '^%\d+$' }

# 8
Run-Test 8 "pane_width" '#{pane_width}' "fmttest" { param($v) $v -match '^\d+$' -and [int]$v -gt 0 }

# 9
Run-Test 9 "pane_height" '#{pane_height}' "fmttest" { param($v) $v -match '^\d+$' -and [int]$v -gt 0 }

# 10
Run-Test 10 "pane_current_command" '#{pane_current_command}' "fmttest" { param($v) $v.Length -gt 0 }

# 11
Run-Test 11 "pane_pid" '#{pane_pid}' "fmttest" { param($v) $v -match '^\d+$' -and [int]$v -gt 0 }

# 12
Run-Test 12 "pane_in_mode" '#{pane_in_mode}' "fmttest" { param($v) $v -eq '0' }

# 13
Run-Test 13 "cursor_x" '#{cursor_x}' "fmttest" { param($v) $v -match '^\d+$' }

# 14
Run-Test 14 "cursor_y" '#{cursor_y}' "fmttest" { param($v) $v -match '^\d+$' }

# 15
Run-Test 15 "session_windows" '#{session_windows}' "fmttest" { param($v) $v -eq '1' }

# 16
Run-Test 16 "window_panes" '#{window_panes}' "fmttest" { param($v) $v -eq '2' }

# 17
Run-Test 17 "pane_current_path" '#{pane_current_path}' "fmttest" { param($v) $v.Length -gt 0 -and ($v -match '\\' -or $v -match '/') }

# 18 - combined #S:#W.#P
Run-Test 18 "combined #S:#W.#P" '#S:#W.#P' "fmttest" { param($v) $v -match '^fmttest:.+\.\d+$' }

# 19 - combined W:index P:index
Run-Test 19 "combined W:index P:index" 'W:#{window_index} P:#{pane_index}' "fmttest" { param($v) $v -match '^W:\d+ P:\d+$' }

# 20 - literal text
Run-Test 20 "literal text" 'hello world' "fmttest" { param($v) $v -eq 'hello world' }

# 21 - host
Run-Test 21 "host" '#{host}' "fmttest" { param($v) $v.Length -gt 0 }

# 22 - session_created
Run-Test 22 "session_created" '#{session_created}' "fmttest" { param($v) $v -match '^\d+$' -and [long]$v -gt 0 }

# 23 - window_active
Run-Test 23 "window_active" '#{window_active}' "fmttest" { param($v) $v -eq '1' }

# 24 - target specific pane 0.0
Run-Test 24 "target pane 0.0" '#{pane_index}' "fmttest:0.0" { param($v) $v -eq '0' }

# 25 - target specific pane 0.1
Run-Test 25 "target pane 0.1" '#{pane_index}' "fmttest:0.1" { param($v) $v -eq '1' }

Write-Host ""
Write-Host "====================================="
Write-Host "TOTAL: $($pass_count + $fail_count) tests, $pass_count PASS, $fail_count FAIL"
Write-Host "====================================="

# Cleanup
& $bin kill-session -t fmttest 2>&1 | Out-Null
Write-Host "Cleanup: killed session fmttest"
