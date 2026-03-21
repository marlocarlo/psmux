# test_claude_cursor_diag.ps1
$ErrorActionPreference = "Stop"
$SESSION = "cursor_test_$(Get-Random -Maximum 9999)"
$PSMUX = "psmux"

function Log($msg) { Write-Host $msg }
function Pass($name) { Write-Host "  PASS: $name" -ForegroundColor Green }
function Fail($name, $detail) { Write-Host "  FAIL: $name - $detail" -ForegroundColor Red }

# Cleanup
Log "Cleaning up old sessions..."
& $PSMUX kill-server 2>$null
Start-Sleep 2

# Start PSMUX session
Log ""
Log "=== Starting PSMUX session '$SESSION' with -c c:\cctest ==="
& $PSMUX new-session -d -s $SESSION -c "c:\cctest" 2>$null
Start-Sleep 3

# Verify session
$info = (& $PSMUX display-message -t $SESSION -p "#{session_name}" 2>&1 | Out-String).Trim()
Log "  Session info: $info"

# Verify working directory
$capture = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
if ($capture -match "cctest") {
    Pass "Session started in c:\cctest"
} else {
    Fail "Start dir" "capture-pane doesn't show cctest"
}

# Test cursor BEFORE Claude
Log ""
Log "=== Test 1: Cursor in normal shell ==="
$cx = (& $PSMUX display-message -t $SESSION -p "#{cursor_x}" 2>&1 | Out-String).Trim()
$cy = (& $PSMUX display-message -t $SESSION -p "#{cursor_y}" 2>&1 | Out-String).Trim()
Log "  Cursor pos in shell: x=$cx, y=$cy"

# Launch Claude
Log ""
Log "=== Launching Claude inside PSMUX ==="
& $PSMUX send-keys -t $SESSION "claude" Enter 2>$null
Log "  Waiting 10s for Claude to start..."
Start-Sleep 10

# Test cursor with Claude running
Log ""
Log "=== Test 2: Cursor with Claude running ==="
$cx = (& $PSMUX display-message -t $SESSION -p "#{cursor_x}" 2>&1 | Out-String).Trim()
$cy = (& $PSMUX display-message -t $SESSION -p "#{cursor_y}" 2>&1 | Out-String).Trim()
Log "  Cursor pos with Claude: x=$cx, y=$cy"

# Capture pane to see Claude's UI
$capture = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
if ($capture -match "Claude") {
    Pass "Claude is running"
    $lines = $capture -split "`n"
    Log "  First 10 lines:"
    for ($i = 0; $i -lt [Math]::Min(10, $lines.Count); $i++) {
        Log "    $($lines[$i])"
    }
} else {
    Fail "Claude start" "Claude doesn't appear to be running"
    $lines = $capture -split "`n"
    for ($i = 0; $i -lt [Math]::Min(5, $lines.Count); $i++) {
        Log "    $($lines[$i])"
    }
}

# Type text into Claude's input
Log ""
Log "=== Test 3: Type into Claude input box ==="
& $PSMUX send-keys -t $SESSION "hello cursor test" 2>$null
Start-Sleep 2

$cx_after = (& $PSMUX display-message -t $SESSION -p "#{cursor_x}" 2>&1 | Out-String).Trim()
$cy_after = (& $PSMUX display-message -t $SESSION -p "#{cursor_y}" 2>&1 | Out-String).Trim()
Log "  Cursor after typing: x=$cx_after, y=$cy_after"

$cx_int = 0
[int]::TryParse($cx_after, [ref]$cx_int) | Out-Null
if ($cx_int -gt 0) {
    Pass "Cursor X position moved after typing (x=$cx_int)"
} else {
    Fail "Cursor X" "Cursor X is 0 after typing"
}

# Capture pane to verify text is visible
$capture2 = & $PSMUX capture-pane -t $SESSION -p 2>&1 | Out-String
if ($capture2 -match "hello cursor test") {
    Pass "Typed text is visible in pane"
} else {
    Fail "Typed text" "Text not found in capture"
}

# Check cursor shape via dump-state
Log ""
Log "=== Test 4: Cursor shape via dump-state ==="
$home = $env:USERPROFILE
$portFile = "$home\.psmux\$SESSION.port"
$keyFile = "$home\.psmux\$SESSION.key"
if (Test-Path $portFile) {
    $port = (Get-Content $portFile).Trim()
    $key = (Get-Content $keyFile).Trim()
    Log "  Server port: $port"
    
    try {
        $tcp = New-Object System.Net.Sockets.TcpClient
        $tcp.Connect("127.0.0.1", [int]$port)
        $stream = $tcp.GetStream()
        $stream.ReadTimeout = 3000
        $writer = New-Object System.IO.StreamWriter($stream)
        $reader = New-Object System.IO.StreamReader($stream)
        $writer.AutoFlush = $true
        
        # Authenticate
        $writer.WriteLine("AUTH $key")
        $authResp = $reader.ReadLine()
        Log "  Auth: $authResp"
        
        # Get dump state
        $writer.WriteLine("dump-state")
        Start-Sleep -Milliseconds 500
        
        $jsonLine = $reader.ReadLine()
        if ($jsonLine -and $jsonLine.Length -gt 10) {
            Log "  Got dump-state JSON (length=$($jsonLine.Length))"
            
            $json = $jsonLine | ConvertFrom-Json -ErrorAction SilentlyContinue
            if ($json) {
                $leaf = $null
                if ($json.type -eq "leaf") { $leaf = $json }
                elseif ($json.children) {
                    foreach ($c in $json.children) {
                        if ($c.type -eq "leaf") { $leaf = $c; break }
                    }
                }
                
                if ($leaf) {
                    Log "  cursor_row=$($leaf.cursor_row), cursor_col=$($leaf.cursor_col)"
                    Log "  hide_cursor=$($leaf.hide_cursor)"
                    Log "  cursor_shape=$($leaf.cursor_shape)"
                    Log "  alternate_screen=$($leaf.alternate_screen)"
                    Log "  active=$($leaf.active)"
                    
                    if ($null -ne $leaf.cursor_shape) {
                        if ($leaf.cursor_shape -ge 0 -and $leaf.cursor_shape -le 6) {
                            Pass "cursor_shape is valid ($($leaf.cursor_shape))"
                        } elseif ($leaf.cursor_shape -eq 255) {
                            Fail "cursor_shape" "Still UNSET (255)"
                        } else {
                            Fail "cursor_shape" "Unexpected value: $($leaf.cursor_shape)"
                        }
                    } else {
                        Fail "cursor_shape" "Not present in dump state"
                    }
                    
                    Log "  hide_cursor=$($leaf.hide_cursor) (we always show cursor for active pane now)"
                } else {
                    Fail "leaf" "Could not find leaf node"
                }
            } else {
                Log "  Failed to parse JSON"
            }
        } else {
            Log "  No dump-state response"
        }
        
        $tcp.Close()
    }
    catch {
        Log "  Error: $_"
    }
} else {
    Log "  Port file not found: $portFile"
}

# Cleanup
Log ""
Log "=== Cleanup ==="
& $PSMUX send-keys -t $SESSION Escape 2>$null
Start-Sleep 1
& $PSMUX send-keys -t $SESSION "/exit" Enter 2>$null
Start-Sleep 3
& $PSMUX kill-server 2>$null
Log "Done."
