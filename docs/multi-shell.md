# Multi-Shell Workflows

psmux lets you run **any combination of shells** side by side in the same session.
PowerShell, Git Bash, cmd.exe, WSL, Nushell, or any other shell or program,
each in its own pane, window, or session. Switch between them instantly.

```
 +-----------------------+-----------------------+
 |  PowerShell 7         |  Git Bash             |
 |  PS C:\project> ...   |  user@pc ~/project $  |
 |                       |                       |
 +-----------------------+-----------------------+
 |  cmd.exe              |  WSL (Ubuntu)         |
 |  C:\project>          |  user@pc:~/project$   |
 |                       |                       |
 +-----------------------+-----------------------+
  [0] pwsh*  [1] bash  [2] node  [3] python
```

## Setting Your Default Shell

Add one line to `~/.psmux.conf`:

```tmux
# Git Bash
set -g default-shell "C:/Program Files/Git/bin/bash.exe"

# Git Bash (backslashes work too)
set -g default-shell "C:\Program Files\Git\bin\bash.exe"

# Git Bash with login profile
set -g default-shell "C:/Program Files/Git/bin/bash.exe" --login

# cmd.exe
set -g default-shell cmd.exe

# PowerShell 7 (the default if nothing is set)
set -g default-shell pwsh

# Windows PowerShell 5
set -g default-shell powershell

# Nushell
set -g default-shell nu

# WSL default distro
set -g default-shell wsl
```

Bare names like `bash`, `pwsh`, `cmd`, `nu`, `wsl` are resolved via PATH.
Full paths with spaces must be wrapped in quotes. Both forward slashes and
backslashes are supported.

## Changing the Shell at Runtime

You don't need to restart psmux to switch shells. Press `Prefix + :` (default
`Ctrl+B` then `:`) to open the command prompt, then type:

```tmux
set -g default-shell "C:/Program Files/Git/bin/bash.exe"
```

Every new window and pane created after this will use the new shell.
Existing panes keep their current shell.

## Mix and Match: Different Shells in Different Panes

This is where psmux really shines. You can override the default shell for
any individual window or pane by passing the shell as a command:

### From the command prompt (`Prefix + :`)

```tmux
# Open a new Git Bash window while your default is pwsh
new-window "C:/Program Files/Git/bin/bash.exe"

# Split the current pane and run cmd.exe in the new split
split-window cmd.exe

# Split horizontally and run WSL
split-window -h wsl

# Open a new window running Python
new-window python

# Open a new window running Node.js REPL
new-window node
```

### From the CLI (PowerShell, cmd, or any terminal)

```powershell
# Create a bash window in an existing session
psmux new-window -- "C:/Program Files/Git/bin/bash.exe"

# Split with cmd.exe
psmux split-window -- cmd.exe

# Create a whole new session running WSL
psmux new-session -s linux -- wsl

# Launch a Python REPL in a split pane
psmux split-window -- python
```

### From your config file (`~/.psmux.conf`)

```tmux
# Default shell is PowerShell
set -g default-shell pwsh

# Bind keys to quickly open specific shells
bind-key B new-window "C:/Program Files/Git/bin/bash.exe"
bind-key C new-window cmd.exe
bind-key W new-window wsl
bind-key N new-window nu

# Bind keys for splitting with a specific shell
bind-key b split-window -v "C:/Program Files/Git/bin/bash.exe"
bind-key c split-window -v cmd.exe
```

Now `Prefix + B` opens a bash window, `Prefix + C` opens cmd, etc.

## Real-World Use Cases

### Web Development

Your default shell is PowerShell for project management, but you need bash
for your build tools and Node scripts:

```tmux
# ~/.psmux.conf
set -g default-shell pwsh

# Quick access to bash for npm/node
bind-key B new-window "C:/Program Files/Git/bin/bash.exe" --login
bind-key b split-window -v "C:/Program Files/Git/bin/bash.exe" --login
```

Workflow:
1. Window 0 (pwsh): `git status`, `dotnet build`, project management
2. `Prefix + B` to open Window 1 (bash): `npm run dev`
3. `Prefix + b` to split (bash): `npm test` running alongside
4. `Prefix + :` then `split-window node` for a quick Node REPL

### DevOps / Infrastructure

Mix WSL Linux tools with native Windows admin shells:

```tmux
set -g default-shell pwsh

bind-key L new-window wsl
bind-key l split-window -v wsl
```

Workflow:
1. Window 0 (pwsh): Azure/AWS CLI, Windows admin tasks
2. `Prefix + L` for Window 1 (WSL): `kubectl`, `docker`, `terraform`
3. Split both windows as needed for logs, monitoring, editors

### Cross-Platform Testing

Test your scripts in every shell without leaving your session:

```tmux
bind-key F1 new-window pwsh
bind-key F2 new-window "C:/Program Files/Git/bin/bash.exe"
bind-key F3 new-window cmd.exe
bind-key F4 new-window wsl
```

### Dedicated Tool Windows

Run long-running tools in their own shells:

```tmux
# Quick launch for common tools
bind-key P new-window python
bind-key J new-window node
bind-key S new-window "C:/Program Files/Git/bin/bash.exe" -c "ssh myserver"
```

## Multiple Sessions with Different Defaults

You can also create entirely separate sessions, each with its own default shell:

```powershell
# Session for PowerShell work
psmux new-session -d -s work

# Session for Linux/bash work
psmux new-session -d -s linux -- wsl

# Session for a specific project using bash
psmux new-session -d -s webapp -- "C:/Program Files/Git/bin/bash.exe" --login
```

Switch between sessions with `Prefix + s` (session picker) or `Prefix + (` / `)`.

## Supported Shells

psmux works with any program that reads from stdin and writes to stdout.
Here are common shells and how to configure them:

| Shell | Config Value | Notes |
|-------|-------------|-------|
| PowerShell 7 | `pwsh` | Default. Fastest startup with psmux optimizations |
| Windows PowerShell 5 | `powershell` | Built into Windows |
| Git Bash | `"C:/Program Files/Git/bin/bash.exe"` | Quotes required (path has spaces) |
| Git Bash (login) | `"C:/Program Files/Git/bin/bash.exe" --login` | Loads `.bash_profile` |
| cmd.exe | `cmd` or `cmd.exe` | Classic Windows command prompt |
| WSL | `wsl` | Launches your default WSL distro |
| WSL (specific distro) | `wsl -d Ubuntu` | Specify a distro by name |
| Nushell | `nu` | Modern structured-data shell |
| Fish | `"C:/path/to/fish.exe"` | If installed via MSYS2/Cygwin |
| Python REPL | `python` | Not a shell, but works great in a pane |
| Node.js REPL | `node` | Same, useful for quick JS testing |

## Tips

- **Paths with spaces** must be wrapped in double quotes: `"C:/Program Files/..."`
- **Forward slashes and backslashes** both work: `C:/Program Files` and `C:\Program Files` are equivalent
- **Bare names** like `bash`, `pwsh`, `cmd`, `nu` are resolved via your system PATH
- **Extra arguments** go after the path: `"C:/Program Files/Git/bin/bash.exe" --login`
- **Changing default-shell at runtime** only affects new panes/windows. Existing ones keep their shell
- **Each pane is independent**. Closing a bash pane does not affect your pwsh panes
- **Environment variables** (`TMUX`, `PSMUX_SESSION`, `TERM`) are set correctly in all shell types
