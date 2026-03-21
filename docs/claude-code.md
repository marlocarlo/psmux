# Claude Code Agent Teams

psmux has first-class support for [Claude Code](https://docs.anthropic.com/en/docs/claude-code) agent teams. When Claude Code runs inside a psmux session, it automatically spawns teammate agents in separate tmux panes instead of running them in-process — giving you full visibility into what each agent is doing.

## Quick Start

1. **Install psmux** (see [README](../README.md#installation))

2. **Start a psmux session:**

   ```powershell
   psmux new-session -s work
   ```

3. **Run Claude Code inside the psmux pane:**

   ```powershell
   claude
   ```

4. **Ask Claude to create a team.** Claude Code will automatically split panes for each teammate agent.

That's it. No extra configuration needed — psmux handles everything automatically.

## How It Works

When a pane spawns inside psmux, several environment variables are set automatically:

| Variable | Value | Purpose |
|----------|-------|---------|
| `TMUX` | `/tmp/psmux-{pid}/...` | Tells Claude Code it's inside tmux |
| `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS` | `1` | Enables the agent teams feature gate |
| `PSMUX_CLAUDE_TEAMMATE_MODE` | `tmux` | Triggers the `--teammate-mode tmux` CLI injection |

Claude Code detects the `TMUX` environment variable, recognizes it's inside a tmux-compatible multiplexer, and uses the **TmuxBackend** to spawn teammate agents via `split-window` and `send-keys` — the same mechanism it uses on Linux/macOS tmux.

### The Two Things psmux Fixes

Claude Code's standalone binary (the Bun SFE `claude.exe`) has two issues on Windows that psmux works around:

1. **Agent teams feature gate**: The entire teammate tool-set (spawnTeam, spawnTeammate) is gated behind `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`. Without this env var, Claude only has the in-process "Agent" tool and never creates separate panes. psmux sets this automatically.

2. **`teammateMode` config ignored**: The standalone binary ignores `teammateMode: "tmux"` from `~/.claude/settings.json`. psmux injects `--teammate-mode tmux` via a PowerShell wrapper function that's loaded in every pane.

## Configuration Options

These options can be set in `~/.psmux.conf` or at runtime:

```tmux
# Auto-inject --teammate-mode tmux for Claude Code (default: on)
set -g claude-code-fix-tty on

# Disable the Claude Code teammate-mode workaround
set -g claude-code-fix-tty off
```

### What each option controls

| Option | Default | Description |
|--------|---------|-------------|
| `claude-code-fix-tty` | `on` | Sets `PSMUX_CLAUDE_TEAMMATE_MODE=tmux` and defines a `claude` wrapper function that injects `--teammate-mode tmux` into every `claude` invocation |

The `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` env var is always set (not gated by any option) since it's required for the feature to work at all.

## Two Agent Systems in Claude Code

Claude Code has **two completely separate agent systems**. Understanding both is critical because psmux can only control one of them.

### 1. Teammate Agents (tmux panes) ✅

The **teammate system** spawns agents in visible tmux panes. This is the system psmux fully supports.

- Triggered when the model passes `team_name` + `name` to the subagent tool
- Gated by `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1` (psmux sets this)
- Controlled by `--teammate-mode tmux` (psmux injects this)
- Each agent gets its own pane with full terminal visibility
- Lower-tier models (Haiku, Sonnet) tend to prefer this path

### 2. Worktree Agents (in-process, invisible) ⚠️

The **worktree system** creates isolated git worktrees and runs agents in-process — **invisible to the user**.

- Triggered when the model passes `isolation: "worktree"` to the subagent tool
- Creates git worktrees at `.claude/worktrees/agent-<id>/` via `git worktree add`
- Each agent works on a separate branch in an isolated repo copy
- Runs entirely in-process (no pane, no terminal output visible)
- Higher-tier models (Opus) tend to prefer this path for git-level isolation
- **On Windows, worktree tmux integration is hardcoded disabled** (`"--tmux may not have effect on Windows when model chooses worktrees. Opus tends to always choose that."`)
- There is **no env var or setting** to force worktree agents into tmux panes

### Why Opus says "Let me launch agents in worktrees"

Both systems are exposed through the **same subagent tool**. The model chooses which to use:

| Parameter | System | Visibility | Model preference |
|-----------|--------|------------|-----------------|
| `team_name` + `name` | Teammate | Visible tmux pane | Haiku, Sonnet |
| `isolation: "worktree"` | Worktree | Invisible in-process | Opus |

Opus prefers worktree agents because they provide **git-level isolation** — each agent works on its own branch and can't cause merge conflicts with other agents. The tradeoff is zero visibility.

### Workaround: Project Instructions

Since the model decides which system to use, you can influence its choice via `CLAUDE.md` project instructions:

```markdown
# Agent Configuration
When spawning subagents, always use the teammate system (team_name + name parameters)
instead of worktree isolation. This ensures agents are visible in tmux panes.
Do NOT use isolation: "worktree" — use teammates instead.
```

Place this in your project's `CLAUDE.md` or `~/.claude/CLAUDE.md` for global effect. This is a **best-effort** approach — the model may still choose worktree isolation for complex parallel tasks.

## Important: Interactive Mode Required

Agent teams spawn in separate tmux panes only when Claude Code is running **interactively** (the default when you type `claude` in a pane). When using `-p` (pipe/print mode), Claude intentionally runs agents in-process since there's no interactive terminal to split.

```powershell
# ✅ Interactive — agents spawn in tmux panes
claude

# ❌ Pipe mode — agents run in-process (by design)
claude -p "do something"
```

## Verifying the Setup

To confirm everything is configured correctly inside a psmux pane:

```powershell
# Check environment variables
Write-Host "TMUX: $env:TMUX"
Write-Host "AGENT_TEAMS: $env:CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS"
Write-Host "TEAMMATE_MODE: $env:PSMUX_CLAUDE_TEAMMATE_MODE"
```

Expected output:
```
TMUX: /tmp/psmux-{pid}/default,{port},0
AGENT_TEAMS: 1
TEAMMATE_MODE: tmux
```

You can also verify the `claude` wrapper is active:

```powershell
Get-Command claude | Format-List
```

If the wrapper is active, this shows a `Function` (not an `Application`). The wrapper auto-injects `--teammate-mode tmux` when calling `claude.exe`.

## Troubleshooting

### Agents still running in-process

1. **Check you're in interactive mode** — not using `-p` or `--print`
2. **Verify env vars** — run the verification commands above
3. **Check debug log** — start Claude with `--debug-file $env:TEMP\claude_debug.log` and look for:
   - `[TeammateModeSnapshot] Captured from CLI override: tmux` — teammate mode is set
   - `[BackendRegistry] isInProcessEnabled: false` — tmux panes will be used
   - `[BackendRegistry] isInProcessEnabled: true (non-interactive session)` — you're in pipe mode

### Opus using "worktree agents" instead of tmux panes

This is expected behavior. Opus prefers `isolation: "worktree"` over the teammate system. These are two completely different agent systems — see [Two Agent Systems](#two-agent-systems-in-claude-code) above.

**What you'll see:** Claude says "Let me launch 3 implementation agents in worktrees" — agents run invisibly, no panes appear.

**Workaround:** Add a `CLAUDE.md` instruction telling the model to prefer teammates over worktree isolation. This is best-effort — the model ultimately decides.

### Claude command not found

Make sure `claude.exe` is on your PATH. Install via:
```powershell
npm install -g @anthropic-ai/claude-code
```

### Wrapper not injecting `--teammate-mode`

The wrapper is only defined when `claude-code-fix-tty` is `on` (default). Check:
```powershell
tmux show-options -g claude-code-fix-tty
```

## Technical Details

For the curious — here's what happens under the hood when Claude Code spawns a teammate:

1. Claude calls `spawnTeammate` tool (available because `T8()` gate passes due to `CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1`)
2. `BackendRegistry.detectAndGetBackend()` checks `isInProcessEnabled`:
   - If non-interactive → true → in-process (by design)
   - If interactive → checks `teammateMode` → `"tmux"` → false → uses TmuxBackend
3. `TmuxBackend` runs `tmux split-window` via psmux's tmux compatibility
4. Sends `cd <workdir> && claude.exe --agent-id <id> --agent-name <name> ...` via `tmux send-keys`
5. The teammate agent starts in its own pane with full terminal access
