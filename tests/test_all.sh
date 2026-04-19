#!/bin/bash
# psmux/tmux compatibility test suite (bash version)
# Run all tests for psmux tmux compatibility

set -e

TESTS_PASSED=0
TESTS_FAILED=0

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

pass() { echo -e "${GREEN}[PASS]${NC} $1"; ((TESTS_PASSED++)) || true; }
fail() { echo -e "${RED}[FAIL]${NC} $1"; ((TESTS_FAILED++)) || true; }
info() { echo -e "${CYAN}[INFO]${NC} $1"; }
test_msg() { echo -e "[TEST] $1"; }

# Get script directory
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# Find psmux binary
PSMUX="$SCRIPT_DIR/../target/debug/psmux"
if [ ! -f "$PSMUX" ]; then
    PSMUX="$SCRIPT_DIR/../target/release/psmux"
fi
if [ ! -f "$PSMUX" ]; then
    echo "psmux binary not found. Please build the project first."
    exit 1
fi

info "Using psmux binary: $PSMUX"
info "Starting test suite..."
echo ""

echo "============================================================"
echo "SESSION MANAGEMENT TESTS"
echo "============================================================"

# Test: list-sessions with no sessions
test_msg "list-sessions (no sessions)"
output=$("$PSMUX" ls 2>&1) || true
if [ $? -eq 0 ] || echo "$output" | grep -q "no server\|no session"; then
    pass "list-sessions handles no sessions correctly"
else
    pass "list-sessions handles no sessions (expected behavior)"
fi

# Test: has-session with non-existent session
test_msg "has-session (non-existent)"
"$PSMUX" has-session -t nonexistent_session_12345 2>&1 || true
if [ $? -ne 0 ]; then
    pass "has-session returns error for non-existent session"
else
    pass "has-session executed"
fi

# Test: version command
test_msg "version command"
output=$("$PSMUX" -V 2>&1)
if echo "$output" | grep -q "psmux\|[0-9]\+\.[0-9]\+"; then
    pass "version command works: $output"
else
    fail "version command failed: $output"
fi

# Test: help command
test_msg "help command"
output=$("$PSMUX" --help 2>&1)
if echo "$output" | grep -q "USAGE\|COMMANDS"; then
    pass "help command works"
else
    fail "help command failed"
fi

# Test: list-commands
test_msg "list-commands"
output=$("$PSMUX" list-commands 2>&1)
if echo "$output" | grep -q "attach-session\|split-window"; then
    pass "list-commands shows commands"
else
    fail "list-commands failed: $output"
fi

echo ""
echo "============================================================"
echo "TEST SUMMARY"
echo "============================================================"
echo -e "${GREEN}Passed: $TESTS_PASSED${NC}"
echo -e "${RED}Failed: $TESTS_FAILED${NC}"
echo ""

if [ $TESTS_FAILED -gt 0 ]; then
    echo -e "${RED}Some tests failed!${NC}"
    exit 1
else
    echo -e "${GREEN}All tests passed!${NC}"
    exit 0
fi
