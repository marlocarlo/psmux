# psmux Testing Methodology

## Quick Reference

```powershell
# Rust unit tests (fast, ~5 seconds)
cargo test

# Full PowerShell test suite (standard tests only, ~45 minutes)
pwsh -NoProfile -ExecutionPolicy Bypass -File tests\run_all_tests.ps1

# Full suite including interactive tests
pwsh -NoProfile -ExecutionPolicy Bypass -File tests\run_all_tests.ps1 -IncludeInteractive

# Single test
pwsh -NoProfile -File tests\test_<name>.ps1

# Build + install before testing
.\scripts\build.ps1
```

## Test Architecture

### Two Test Layers

1. **Rust Unit Tests** (`cargo test`): 385+ in-process tests covering parsing, formatting, layout algorithms, CLI argument handling, VT100 sequences, and internal data structures. These run in under 5 seconds and should ALWAYS pass. Run them after every code change.

2. **PowerShell Integration Tests** (`tests/*.ps1`): 143 test suites exercising end to end behaviour via the psmux CLI. These create real sessions, send keys, capture pane output, and verify results. They are the primary regression gate.

### Test Runner

The runner at `tests/run_all_tests.ps1` handles:
- Automatic server cleanup between test suites (kill server + stale port files)
- Categorisation: General, Issue Fixes, Interactive, UI/Layout, Perf/Stress, Session Mgmt, WSL, Config/Plugin
- Live progress dashboard with ETA
- Per-suite logs at `$env:TEMP\psmux-test-logs\<run_id>\suites\<name>.log`
- Summary report with pass/fail/skip counts

### Categories (and what gets skipped by default)

| Category | Skipped by Default | Flag to Include |
|----------|-------------------|-----------------|
| Interactive (TUI, mouse, cursor) | Yes | `-IncludeInteractive` |
| WSL | Yes | `-IncludeWSL` |
| Perf/Stress | Yes | (always skipped unless `-SkipPerf` not passed) |
| All others | No | N/A |

## Writing Reliable Tests

### Session Creation

ALWAYS use explicit dimensions when creating detached sessions:
```powershell
psmux new-session -d -s $SESSION -x 120 -y 30
```

Without `-x` and `-y`, `init_size` is `None`, which can cause:
- Warm pane spawn issues
- ConPTY buffer sizing problems
- Server exit when shell fails to start

### Timing Guidelines

PowerShell test timing is the most common source of flaky failures. These minimum waits are based on observed ConPTY behaviour:

| Operation | Minimum Wait |
|-----------|-------------|
| After `new-session -d` | 3 seconds |
| After `split-window` | 2 seconds |
| After `new-window` | 3 seconds |
| After `send-keys echo ...` | 1.5 seconds |
| After `send-keys <TUI command>` | 4 seconds |
| After TUI exit (clean RMCUP) | 6 seconds |
| After force-kill TUI (no RMCUP) | 8 seconds |
| After `kill-session` / `kill-server` | 1 second |

### ConPTY Behaviour on Windows

Key facts about ConPTY (the Windows pseudo-terminal layer):

1. **ConPTY eats SMCUP/RMCUP**: ESC[?1049h and ESC[?1049l are processed by ConPTY internally. `screen.alternate_screen()` is ALWAYS false. psmux uses a heuristic (last row content) to detect fullscreen TUI apps.

2. **Ctrl+C propagation**: `GenerateConsoleCtrlEvent` sends to ALL processes sharing the console, not just the foreground process. This means sending `C-c` via `send-keys` can kill both a TUI app AND the parent shell. Use `q` or the app's quit key for clean exit testing.

3. **Alt screen restoration timing**: After a TUI sends RMCUP, ConPTY needs time to generate the restore sequences. Captures taken too soon will still show TUI content. Wait at least 6 seconds after a TUI exit.

4. **Force-killed TUI**: A TUI killed without RMCUP leaves its content visible permanently on ConPTY. This is expected behaviour (same as tmux on Linux when a TUI crashes without cleanup).

### Test Structure Pattern

Every test should follow this structure:
```powershell
$ErrorActionPreference = "Continue"
$results = @()

function Add-Result($name, $pass, $detail="") {
    $script:results += [PSCustomObject]@{
        Test=$name
        Result=if($pass){"PASS"}else{"FAIL"}
        Detail=$detail
    }
    $mark = if($pass) { "[PASS]" } else { "[FAIL]" }
    Write-Host "  $mark $name$(if($detail){' '+$detail}else{''})"
}

# ---- Setup ----
psmux kill-server 2>$null
Start-Sleep -Seconds 1

psmux new-session -d -s $SESSION -x 120 -y 30 2>$null
Start-Sleep -Seconds 3

# ---- Test body ----
psmux send-keys -t $SESSION "echo MARKER" Enter
Start-Sleep -Seconds 2
$cap = psmux capture-pane -t $SESSION -p 2>&1 | Out-String
Add-Result "Marker visible" ($cap -match "MARKER")

# ---- Cleanup ----
psmux kill-session -t $SESSION 2>$null
psmux kill-server 2>$null

# ---- Report ----
$pass = ($results | Where-Object { $_.Result -eq "PASS" }).Count
$fail = ($results | Where-Object { $_.Result -eq "FAIL" }).Count
Write-Host "Total: $($results.Count)  Pass: $pass  Fail: $fail"
if ($fail -gt 0) { exit 1 } else { exit 0 }
```

## Debugging Failures

### "no server running" Errors

The server exited before the test could query it. Common causes:
- Session created without `-x -y` (init_size = None)
- Shell died (crash, Ctrl+C propagation)
- Another test's `kill-server` killed this test's server (parallel interference)
- `exit_empty` kicked in because the last pane died

**Fix**: Add `-x 120 -y 30`, increase startup sleep, check for parallel test interference.

### Capture Returns Empty or Wrong Content

The PTY output has not been fully flushed. Common causes:
- Insufficient sleep after `send-keys`
- ConPTY buffering delay for large output or color sequences
- Split pane has smaller buffer, needs more time for ConPTY to render

**Fix**: Increase sleep time. For split panes, add 50% more wait time.

### Intermittent Failures (Pass on Re-run)

Timing-dependent failures that pass 4 out of 5 times. Root cause is almost always insufficient sleep between operations. If a test fails intermittently:
1. Identify which specific check fails
2. Find the sleep BEFORE that check
3. Increase it by 50-100%
4. Re-run 3 times to confirm stability

### TUI Content Remnants

After a TUI exits, `capture-pane` still shows TUI content. Causes:
- **Clean exit (RMCUP sent)**: Not enough wait time for ConPTY to restore. Increase post-exit sleep.
- **Crash exit (no RMCUP)**: Expected. ConPTY never restores without RMCUP. Test should verify shell responsiveness instead of screen cleanliness.

## Code Change Workflow

1. Make code changes
2. `cargo check` (compile check, fast)
3. `cargo test` (Rust unit tests, fast)
4. `.\scripts\build.ps1` (build release + install)
5. Run the specific PS test that covers your change
6. Run the full PS suite: `pwsh -NoProfile -ExecutionPolicy Bypass -File tests\run_all_tests.ps1`
7. For TUI/mouse changes, also run: `pwsh -NoProfile -File tests\test_tui_exit_cleanup.ps1`

## Critical Files

| Path | Purpose |
|------|---------|
| `tests/run_all_tests.ps1` | Full test suite runner with categories |
| `tests/test_*.ps1` | Individual test suites |
| `src/server/mod.rs` | Server event loop (session, pane, window management) |
| `src/input.rs` | Key handling, pane navigation, focus |
| `src/pane.rs` | Pane creation, warm pane, shell spawn |
| `crates/vt100-psmux/src/screen.rs` | VT100 terminal emulation |
| `src/copy_mode.rs` | Copy mode, capture-pane implementation |
| `src/layout.rs` | Layout tree, alternate screen heuristic |
