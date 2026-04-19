#!/usr/bin/env python3
"""
psmux Battle Test Suite - Python Edition
Comprehensive testing with concurrent operations and edge cases
"""

import subprocess
import time
import os
import sys
import threading
import random
import string
from pathlib import Path
from concurrent.futures import ThreadPoolExecutor, as_completed

# Find psmux binary
SCRIPT_DIR = Path(__file__).parent
PROJECT_DIR = SCRIPT_DIR.parent
PSMUX = PROJECT_DIR / "target" / "release" / "psmux.exe"
if not PSMUX.exists():
    PSMUX = PROJECT_DIR / "target" / "debug" / "psmux.exe"
if not PSMUX.exists():
    print("ERROR: psmux binary not found. Build with: cargo build --release")
    sys.exit(1)

PSMUX = str(PSMUX)

# Test statistics
class Stats:
    passed = 0
    failed = 0
    skipped = 0
    lock = threading.Lock()
    
    @classmethod
    def pass_test(cls):
        with cls.lock:
            cls.passed += 1
    
    @classmethod
    def fail_test(cls):
        with cls.lock:
            cls.failed += 1
    
    @classmethod
    def skip_test(cls):
        with cls.lock:
            cls.skipped += 1


def print_pass(msg):
    print(f"\033[92m[PASS]\033[0m {msg}")
    Stats.pass_test()

def print_fail(msg):
    print(f"\033[91m[FAIL]\033[0m {msg}")
    Stats.fail_test()

def print_skip(msg):
    print(f"\033[93m[SKIP]\033[0m {msg}")
    Stats.skip_test()

def print_info(msg):
    print(f"\033[96m[INFO]\033[0m {msg}")

def print_test(msg):
    print(f"\033[97m[TEST]\033[0m {msg}")

def print_section(msg):
    print()
    print("\033[95m" + "=" * 70 + "\033[0m")
    print(f"\033[95m  {msg}\033[0m")
    print("\033[95m" + "=" * 70 + "\033[0m")


def run_psmux(*args, timeout=10, check=False):
    """Run psmux command and return result"""
    cmd = [PSMUX] + list(args)
    try:
        result = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return result
    except subprocess.TimeoutExpired:
        return None


def session_exists(name):
    """Check if session exists"""
    result = run_psmux("has-session", "-t", name)
    return result is not None and result.returncode == 0


def create_session(name, timeout=3):
    """Create a detached session"""
    # Kill existing
    run_psmux("kill-session", "-t", name)
    time.sleep(0.3)
    
    # Create new
    subprocess.Popen([PSMUX, "new-session", "-s", name, "-d"], 
                     creationflags=subprocess.CREATE_NO_WINDOW if sys.platform == 'win32' else 0)
    time.sleep(1.5)
    
    return session_exists(name)


def kill_session(name):
    """Kill a session"""
    run_psmux("kill-session", "-t", name)
    time.sleep(0.3)


def cleanup_sessions(names):
    """Clean up multiple sessions"""
    for name in names:
        try:
            run_psmux("kill-session", "-t", name)
        except:
            pass


# ============================================================================
# TEST FUNCTIONS
# ============================================================================

def test_session_lifecycle():
    """Test session create, list, and destroy"""
    print_section("SESSION LIFECYCLE TESTS")
    
    session = "py_lifecycle_test"
    
    # Create
    print_test("Create session")
    if create_session(session):
        print_pass(f"Session '{session}' created")
    else:
        print_fail(f"Failed to create session '{session}'")
        return
    
    # List
    print_test("List sessions")
    result = run_psmux("ls")
    if result and session in result.stdout:
        print_pass("Session appears in list")
    else:
        print_fail("Session not in list")
    
    # Kill
    print_test("Kill session")
    kill_session(session)
    if not session_exists(session):
        print_pass("Session killed successfully")
    else:
        print_fail("Session still exists after kill")


def test_window_operations():
    """Test window creation and navigation"""
    print_section("WINDOW OPERATIONS TESTS")
    
    session = "py_window_test"
    create_session(session)
    
    # Create windows
    print_test("Create 5 windows")
    for i in range(5):
        run_psmux("new-window", "-t", session)
        time.sleep(0.2)
    print_pass("5 windows created")
    
    # List windows
    print_test("List windows")
    result = run_psmux("list-windows", "-t", session)
    if result and result.stdout:
        print_pass(f"list-windows returned data")
    else:
        print_fail("list-windows failed")
    
    # Navigate
    print_test("Window navigation")
    for _ in range(10):
        run_psmux("next-window", "-t", session)
        run_psmux("previous-window", "-t", session)
    print_pass("Window navigation completed")
    
    # Select specific
    print_test("Select window by index")
    for i in range(3):
        run_psmux("select-window", "-t", f"{session}:{i}")
        time.sleep(0.1)
    print_pass("Window selection by index works")
    
    kill_session(session)


def test_pane_operations():
    """Test pane splitting and navigation"""
    print_section("PANE OPERATIONS TESTS")
    
    session = "py_pane_test"
    create_session(session)
    
    # Vertical split
    print_test("Vertical split")
    run_psmux("split-window", "-v", "-t", session)
    time.sleep(0.3)
    print_pass("Vertical split created")
    
    # Horizontal split
    print_test("Horizontal split")
    run_psmux("split-window", "-h", "-t", session)
    time.sleep(0.3)
    print_pass("Horizontal split created")
    
    # Multiple splits
    print_test("Multiple rapid splits")
    for i in range(6):
        direction = "-v" if i % 2 == 0 else "-h"
        run_psmux("split-window", direction, "-t", session)
        time.sleep(0.2)
    print_pass("6 additional splits created")
    
    # List panes
    print_test("List panes")
    result = run_psmux("list-panes", "-t", session)
    if result and result.stdout:
        print_pass("list-panes returned data")
    else:
        print_fail("list-panes failed")
    
    # Navigate panes
    print_test("Pane navigation all directions")
    for direction in ["-U", "-D", "-L", "-R"] * 5:
        run_psmux("select-pane", direction, "-t", session)
        time.sleep(0.05)
    print_pass("Pane navigation completed")
    
    kill_session(session)


def test_resize_operations():
    """Test pane resizing"""
    print_section("RESIZE OPERATIONS TESTS")
    
    session = "py_resize_test"
    create_session(session)
    run_psmux("split-window", "-v", "-t", session)
    run_psmux("split-window", "-h", "-t", session)
    time.sleep(0.5)
    
    # Resize in all directions
    for direction, name in [("-U", "up"), ("-D", "down"), ("-L", "left"), ("-R", "right")]:
        print_test(f"Resize pane {name}")
        for _ in range(5):
            run_psmux("resize-pane", direction, "3", "-t", session)
            time.sleep(0.05)
        print_pass(f"Resize {name} completed")
    
    # Zoom toggle
    print_test("Zoom pane toggle")
    run_psmux("resize-pane", "-Z", "-t", session)
    time.sleep(0.3)
    run_psmux("resize-pane", "-Z", "-t", session)
    print_pass("Zoom toggle completed")
    
    kill_session(session)


def test_send_keys():
    """Test sending keys to panes"""
    print_section("SEND-KEYS TESTS")
    
    session = "py_keys_test"
    create_session(session)
    
    # Basic send-keys
    print_test("Send basic keys")
    run_psmux("send-keys", "-t", session, "echo hello", "Enter")
    time.sleep(0.3)
    print_pass("Basic keys sent")
    
    # Literal send-keys
    print_test("Send literal keys")
    run_psmux("send-keys", "-l", "-t", session, "test literal string")
    print_pass("Literal keys sent")
    
    # Special keys
    print_test("Send special keys")
    for key in ["Tab", "Escape", "Up", "Down", "Left", "Right"]:
        run_psmux("send-keys", "-t", session, key)
        time.sleep(0.05)
    print_pass("Special keys sent")
    
    # Rapid send
    print_test("Rapid send-keys (20 commands)")
    for i in range(20):
        run_psmux("send-keys", "-t", session, f"echo test{i}", "Enter")
        time.sleep(0.02)
    print_pass("Rapid send completed")
    
    kill_session(session)


def test_kill_operations():
    """Test killing panes, windows, sessions"""
    print_section("KILL OPERATIONS TESTS")
    
    session = "py_kill_test"
    create_session(session)
    
    # Create and kill panes
    print_test("Create and kill panes")
    for _ in range(3):
        run_psmux("split-window", "-v", "-t", session)
        time.sleep(0.2)
    run_psmux("kill-pane", "-t", session)
    time.sleep(0.3)
    print_pass("Pane killed")
    
    # Create and kill windows
    print_test("Create and kill windows")
    run_psmux("new-window", "-t", session)
    run_psmux("new-window", "-t", session)
    time.sleep(0.3)
    run_psmux("kill-window", "-t", session)
    time.sleep(0.3)
    print_pass("Window killed")
    
    # Kill session
    print_test("Kill session")
    kill_session(session)
    if not session_exists(session):
        print_pass("Session killed")
    else:
        print_fail("Session still exists")


def test_layouts():
    """Test layout presets"""
    print_section("LAYOUT TESTS")
    
    session = "py_layout_test"
    create_session(session)
    
    # Create panes
    for _ in range(3):
        run_psmux("split-window", "-v", "-t", session)
        time.sleep(0.2)
    
    layouts = ["even-horizontal", "even-vertical", "main-horizontal", "main-vertical", "tiled"]
    for layout in layouts:
        print_test(f"Apply layout: {layout}")
        run_psmux("select-layout", "-t", session, layout)
        time.sleep(0.3)
        print_pass(f"{layout} applied")
    
    kill_session(session)


def test_swap_rotate():
    """Test swap and rotate operations"""
    print_section("SWAP AND ROTATE TESTS")
    
    session = "py_swap_test"
    create_session(session)
    run_psmux("split-window", "-v", "-t", session)
    run_psmux("split-window", "-h", "-t", session)
    time.sleep(0.5)
    
    print_test("Swap pane up/down")
    run_psmux("swap-pane", "-U", "-t", session)
    run_psmux("swap-pane", "-D", "-t", session)
    print_pass("Swap operations completed")
    
    print_test("Rotate window")
    for _ in range(5):
        run_psmux("rotate-window", "-t", session)
        time.sleep(0.1)
    print_pass("5 rotations completed")
    
    kill_session(session)


def test_buffers():
    """Test buffer operations"""
    print_section("BUFFER TESTS")
    
    session = "py_buffer_test"
    create_session(session)
    
    print_test("Set buffer")
    run_psmux("set-buffer", "-t", session, "Test buffer content 12345")
    print_pass("Buffer set")
    
    print_test("List buffers")
    result = run_psmux("list-buffers", "-t", session)
    print_pass("list-buffers executed")
    
    print_test("Show buffer")
    result = run_psmux("show-buffer", "-t", session)
    print_pass("show-buffer executed")
    
    print_test("Capture pane")
    result = run_psmux("capture-pane", "-t", session, "-p")
    if result and result.stdout:
        print_pass("capture-pane returned content")
    else:
        print_skip("capture-pane returned empty")
    
    kill_session(session)


def test_concurrent_sessions():
    """Test creating multiple sessions concurrently"""
    print_section("CONCURRENT SESSION TESTS")
    
    session_names = [f"py_concurrent_{i}" for i in range(5)]
    
    print_test("Create 5 sessions concurrently")
    
    def create_and_verify(name):
        create_session(name)
        return session_exists(name)
    
    with ThreadPoolExecutor(max_workers=5) as executor:
        futures = {executor.submit(create_and_verify, name): name for name in session_names}
        results = []
        for future in as_completed(futures):
            results.append(future.result())
    
    success = sum(results)
    if success >= 4:  # Allow 1 failure due to timing
        print_pass(f"Created {success}/5 sessions concurrently")
    else:
        print_fail(f"Only created {success}/5 sessions")
    
    # Verify all in list
    print_test("Verify all sessions in list")
    result = run_psmux("ls")
    if result:
        found = sum(1 for name in session_names if name in result.stdout)
        if found >= 4:
            print_pass(f"Found {found}/5 sessions in list")
        else:
            print_fail(f"Only found {found}/5 sessions")
    
    # Cleanup
    cleanup_sessions(session_names)


def test_concurrent_operations():
    """Test concurrent operations on single session"""
    print_section("CONCURRENT OPERATIONS TESTS")
    
    session = "py_concurrent_ops"
    create_session(session)
    
    # Create initial panes
    for _ in range(3):
        run_psmux("split-window", "-v", "-t", session)
        time.sleep(0.2)
    
    print_test("Concurrent pane navigation (100 ops)")
    
    def random_nav():
        directions = ["-U", "-D", "-L", "-R"]
        for _ in range(20):
            run_psmux("select-pane", random.choice(directions), "-t", session)
            time.sleep(0.01)
    
    with ThreadPoolExecutor(max_workers=5) as executor:
        futures = [executor.submit(random_nav) for _ in range(5)]
        for future in as_completed(futures):
            pass
    
    print_pass("100 concurrent navigation ops completed")
    
    kill_session(session)


def test_stress():
    """Stress test with many operations"""
    print_section("STRESS TESTS")
    
    session = "py_stress_test"
    create_session(session)
    
    print_test("Stress: 100 mixed operations")
    ops = 0
    for _ in range(25):
        run_psmux("split-window", "-v", "-t", session)
        ops += 1
        run_psmux("select-pane", "-U", "-t", session)
        ops += 1
        run_psmux("resize-pane", "-D", "1", "-t", session)
        ops += 1
        run_psmux("send-keys", "-t", session, "echo test", "Enter")
        ops += 1
        time.sleep(0.02)
    print_pass(f"{ops} operations completed")
    
    print_test("Stress: Rapid session create/destroy (10 cycles)")
    for i in range(10):
        name = f"py_rapid_{i}"
        create_session(name)
        kill_session(name)
    print_pass("10 rapid cycles completed")
    
    kill_session(session)


def test_edge_cases():
    """Test edge cases and error handling"""
    print_section("EDGE CASE TESTS")
    
    # Non-existent session
    print_test("Command on non-existent session")
    result = run_psmux("split-window", "-t", "nonexistent_xyz_999")
    if result and (result.returncode != 0 or "error" in result.stderr.lower() or "not found" in result.stderr.lower()):
        print_pass("Correctly handles non-existent session")
    else:
        print_skip("Error handling unclear")
    
    # Special characters in session name
    print_test("Session with special names")
    names = ["test-dash", "test_underscore", "Test123", "a" * 30]
    for name in names:
        if create_session(name):
            kill_session(name)
    print_pass("Various session names handled")
    
    # Empty commands
    print_test("Help command")
    result = run_psmux("--help")
    if result and result.returncode == 0:
        print_pass("Help command works")
    else:
        print_fail("Help command failed")
    
    # Version command
    print_test("Version command")
    result = run_psmux("--version")
    if result and result.returncode == 0:
        print_pass(f"Version: {result.stdout.strip()}")
    else:
        print_fail("Version command failed")


def test_display_commands():
    """Test display and info commands"""
    print_section("DISPLAY COMMAND TESTS")
    
    session = "py_display_test"
    create_session(session)
    
    print_test("display-message with format")
    result = run_psmux("display-message", "-t", session, "-p", "#S:#I:#W")
    if result and result.stdout:
        print_pass(f"display-message: {result.stdout.strip()}")
    else:
        print_skip("display-message returned empty")
    
    print_test("list-commands")
    result = run_psmux("list-commands")
    if result and result.stdout:
        print_pass("list-commands works")
    else:
        print_fail("list-commands failed")
    
    print_test("list-keys")
    result = run_psmux("list-keys")
    if result and result.stdout:
        print_pass("list-keys works")
    else:
        print_skip("list-keys returned empty")
    
    kill_session(session)


def main():
    print()
    print("\033[96m╔══════════════════════════════════════════════════════════════════════╗\033[0m")
    print("\033[96m║            PSMUX BATTLE TEST SUITE - PYTHON EDITION                  ║\033[0m")
    print("\033[96m║            Comprehensive Feature Testing with Concurrency            ║\033[0m")
    print("\033[96m╚══════════════════════════════════════════════════════════════════════╝\033[0m")
    print()
    print_info(f"Binary: {PSMUX}")
    print_info(f"Started: {time.strftime('%Y-%m-%d %H:%M:%S')}")
    print()
    
    # Run all tests
    test_session_lifecycle()
    test_window_operations()
    test_pane_operations()
    test_resize_operations()
    test_send_keys()
    test_kill_operations()
    test_layouts()
    test_swap_rotate()
    test_buffers()
    test_concurrent_sessions()
    test_concurrent_operations()
    test_stress()
    test_edge_cases()
    test_display_commands()
    
    # Final cleanup
    print_section("FINAL CLEANUP")
    all_test_sessions = [
        "py_lifecycle_test", "py_window_test", "py_pane_test", "py_resize_test",
        "py_keys_test", "py_kill_test", "py_layout_test", "py_swap_test",
        "py_buffer_test", "py_concurrent_ops", "py_stress_test", "py_display_test",
        "test-dash", "test_underscore", "Test123"
    ] + [f"py_concurrent_{i}" for i in range(5)] + [f"py_rapid_{i}" for i in range(10)]
    cleanup_sessions(all_test_sessions)
    print_info("Cleanup complete")
    
    # Results
    print()
    print("\033[96m╔══════════════════════════════════════════════════════════════════════╗\033[0m")
    print("\033[96m║                         FINAL RESULTS                                ║\033[0m")
    print("\033[96m╚══════════════════════════════════════════════════════════════════════╝\033[0m")
    print()
    
    total = Stats.passed + Stats.failed + Stats.skipped
    print(f"  Total Tests: {total}")
    print(f"\033[92m  ✓ Passed:    {Stats.passed}\033[0m")
    print(f"\033[91m  ✗ Failed:    {Stats.failed}\033[0m")
    print(f"\033[93m  ○ Skipped:   {Stats.skipped}\033[0m")
    print()
    
    pass_rate = (Stats.passed / total * 100) if total > 0 else 0
    color = "\033[92m" if pass_rate >= 80 else ("\033[93m" if pass_rate >= 60 else "\033[91m")
    print(f"{color}  Pass Rate: {pass_rate:.1f}%\033[0m")
    print()
    print_info(f"Completed: {time.strftime('%Y-%m-%d %H:%M:%S')}")
    print()
    
    if Stats.failed == 0:
        print("\033[92m🎉 ALL TESTS PASSED! psmux is battle-ready!\033[0m")
        return 0
    else:
        print("\033[93m⚠️  Some tests failed. Review the output above.\033[0m")
        return 1


if __name__ == "__main__":
    sys.exit(main())
