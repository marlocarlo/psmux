# test_issue52_cursor.ps1 — Diagnostic test for GitHub Issue #52
# "Cursor not in the right position in Claude session"
#
# Tests:
#  1. CSI s/u (SCOSC/SCORC) — cursor save/restore via CSI sequences
#  2. hide_cursor flag propagation — cursor hidden during render cycles
#  3. End-to-end cursor position validation with TUI-like rendering
#
# Requires: psmux built and in PATH (or cargo build --release done)

$ErrorActionPreference = "Stop"
$script:pass = 0
$script:fail = 0
$script:results = @()

function Log($msg) { Write-Host $msg }
function Pass($name) { $script:pass++; $script:results += "PASS: $name"; Write-Host "  PASS: $name" -ForegroundColor Green }
function Fail($name, $detail) { $script:fail++; $script:results += "FAIL: $name — $detail"; Write-Host "  FAIL: $name — $detail" -ForegroundColor Red }

# Kill any existing psmux server
psmux kill-server 2>$null
Start-Sleep 1

# Start a detached session
Log "Starting psmux session..."
psmux new-session -d 2>$null
Start-Sleep 2

# ─── Test 1: CSI s / CSI u (save/restore cursor) ────────────────────────────
Log ""
Log "=== Test 1: CSI s / CSI u (save/restore cursor) ==="
Log "  Sending: move to (5,10), save, move to (1,1), restore, query position"

# Move cursor to row 5, col 10, then save with CSI s
# Then move to row 1, col 1, then restore with CSI u
# The cursor should be back at row 5, col 10
$script = @'
printf '\x1b[5;10H'
printf '\x1b[s'
printf '\x1b[1;1H'
printf '\x1b[u'
printf 'CURSOR_RESTORED'
'@
# Use send-keys to write a test to the pane
psmux send-keys "printf '\x1b[5;10H'" Enter 2>$null
Start-Sleep -Milliseconds 500
psmux send-keys "printf '\x1b[s'" Enter 2>$null
Start-Sleep -Milliseconds 500
psmux send-keys "printf '\x1b[1;1H'" Enter 2>$null
Start-Sleep -Milliseconds 500
psmux send-keys "printf '\x1b[u'" Enter 2>$null
Start-Sleep -Milliseconds 500

# Capture the pane and check cursor position
$capture = psmux capture-pane -p 2>&1 | Out-String
Log "  Capture after CSI s/u: $($capture.Length) chars"

# We can't directly query cursor position from outside, so let's use a different approach
# Write a Python script that tests the vt100 parser directly
Log ""
Log "=== Test 1b: Direct vt100 parser CSI s/u test ==="

# Create a simple test program
$testCode = @'
use std::sync::{Arc, Mutex};

fn main() {
    let parser = Arc::new(Mutex::new(vt100::Parser::new(24, 80, 0)));
    let mut p = parser.lock().unwrap();
    
    // Move cursor to row 5, col 10 (1-based: 6, 11)
    p.process(b"\x1b[6;11H");
    let (r, c) = p.screen().cursor_position();
    println!("After CUP(6,11): row={}, col={}", r, c);
    assert_eq!((r, c), (5, 10), "CUP positioning failed");
    
    // Save cursor with CSI s
    p.process(b"\x1b[s");
    
    // Move to a different position
    p.process(b"\x1b[1;1H");
    let (r, c) = p.screen().cursor_position();
    println!("After CUP(1,1): row={}, col={}", r, c);
    assert_eq!((r, c), (0, 0), "CUP to origin failed");
    
    // Restore cursor with CSI u
    p.process(b"\x1b[u");
    let (r, c) = p.screen().cursor_position();
    println!("After CSI u restore: row={}, col={}", r, c);
    
    if (r, c) == (5, 10) {
        println!("TEST1_PASS: CSI s/u cursor save/restore works correctly");
    } else {
        println!("TEST1_FAIL: CSI u restored to ({}, {}), expected (5, 10)", r, c);
    }
    
    // Test 2: CSI f (HVP - same as CUP)
    p.process(b"\x1b[10;20f");
    let (r, c) = p.screen().cursor_position();
    println!("After HVP(10,20): row={}, col={}", r, c);
    if (r, c) == (9, 19) {
        println!("TEST2_PASS: CSI f (HVP) works correctly");
    } else {
        println!("TEST2_FAIL: HVP set cursor to ({}, {}), expected (9, 19)", r, c);
    }
    
    // Test 3: hide_cursor flag
    p.process(b"\x1b[?25l");  // Hide cursor
    let hidden = p.screen().hide_cursor();
    println!("After CSI ?25l: hide_cursor={}", hidden);
    if hidden {
        println!("TEST3a_PASS: hide_cursor correctly set");
    } else {
        println!("TEST3a_FAIL: hide_cursor should be true");
    }
    
    p.process(b"\x1b[?25h");  // Show cursor
    let hidden = p.screen().hide_cursor();
    println!("After CSI ?25h: hide_cursor={}", hidden);
    if !hidden {
        println!("TEST3b_PASS: show_cursor correctly set");
    } else {
        println!("TEST3b_FAIL: hide_cursor should be false");
    }
    
    // Test 4: Simulate Claude-like TUI rendering pattern
    // Claude: hide cursor -> render -> position cursor at input -> show cursor
    p.process(b"\x1b[?25l");          // Hide cursor
    p.process(b"\x1b[1;1H");          // Move to top for rendering
    p.process(b"\x1b[2J");            // Clear screen
    // Simulate rendering header
    p.process(b"\x1b[1;1H");
    p.process(b"Claude Code CLI");
    // Simulate rendering content area
    p.process(b"\x1b[5;1H");
    p.process(b"Some conversation text...");
    // Position cursor at input box (row 20, col 3)
    p.process(b"\x1b[20;3H");
    p.process(b"\x1b[?25h");          // Show cursor
    
    let (r, c) = p.screen().cursor_position();
    let hidden = p.screen().hide_cursor();
    println!("After TUI render: cursor=({}, {}), hidden={}", r, c, hidden);
    if (r, c) == (19, 2) && !hidden {
        println!("TEST4_PASS: TUI render cursor position correct");
    } else {
        println!("TEST4_FAIL: cursor=({}, {}), hidden={}, expected (19, 2, false)", r, c, hidden);
    }
    
    // Test 5: CSI s/u in TUI pattern (Ink-style on Windows)
    // Ink on Windows uses CSI s/u for cursor save/restore
    p.process(b"\x1b[?25l");          // Hide cursor
    p.process(b"\x1b[s");             // Save cursor position (at input box)
    p.process(b"\x1b[1;1H");          // Move to top for status update
    p.process(b"[Updated status]");
    p.process(b"\x1b[u");             // Restore cursor (should go back to input box)
    p.process(b"\x1b[?25h");          // Show cursor
    
    let (r, c) = p.screen().cursor_position();
    let hidden = p.screen().hide_cursor();
    println!("After Ink-style s/u: cursor=({}, {}), hidden={}", r, c, hidden);
    if (r, c) == (19, 2) && !hidden {
        println!("TEST5_PASS: Ink-style CSI s/u works correctly");
    } else {
        println!("TEST5_FAIL: cursor=({}, {}), hidden={}, expected (19, 2, false)", r, c, hidden);
    }
    
    println!("\n=== All tests complete ===");
}
'@

# Write the test as a Rust example
$testDir = Join-Path $PSScriptRoot ".." "examples"
$testFile = Join-Path $testDir "test_cursor_issue52.rs"
Set-Content -Path $testFile -Value $testCode -Encoding UTF8

Log "  Running vt100 parser cursor tests..."
$output = cargo run --example test_cursor_issue52 --release 2>&1 | Out-String
Log $output

# Parse results
if ($output -match "TEST1_PASS") { Pass "CSI s/u save/restore" } else { Fail "CSI s/u save/restore" "CSI s/u not handled by vt100 parser" }
if ($output -match "TEST2_PASS") { Pass "CSI f (HVP)" } else { Fail "CSI f (HVP)" "HVP not handled by vt100 parser" }
if ($output -match "TEST3a_PASS") { Pass "hide_cursor set" } else { Fail "hide_cursor set" "hide_cursor flag not working" }
if ($output -match "TEST3b_PASS") { Pass "show_cursor set" } else { Fail "show_cursor set" "show_cursor flag not working" }
if ($output -match "TEST4_PASS") { Pass "TUI render cursor" } else { Fail "TUI render cursor" "TUI render cursor position wrong" }
if ($output -match "TEST5_PASS") { Pass "Ink-style CSI s/u" } else { Fail "Ink-style CSI s/u" "Ink-style save/restore dropped — ROOT CAUSE of issue #52" }

# ─── Summary ─────────────────────────────────────────────────────────────────
Log ""
Log "═══════════════════════════════════════"
Log "  Results: $($script:pass) passed, $($script:fail) failed"
Log "═══════════════════════════════════════"
foreach ($r in $script:results) { Log "  $r" }
Log ""

# Cleanup
psmux kill-server 2>$null
Remove-Item $testFile -ErrorAction SilentlyContinue

if ($script:fail -gt 0) {
    Log "SOME TESTS FAILED — Issue #52 is reproducible"
    exit 1
} else {
    Log "ALL TESTS PASSED"
    exit 0
}
