# FAQ

**Q: Is psmux cross-platform?**
A: No. psmux is built exclusively for Windows using the Windows ConPTY API. For Linux/macOS, use tmux. psmux is the Windows counterpart.

**Q: Does psmux work with Windows Terminal?**
A: Yes! psmux works great with Windows Terminal, PowerShell, cmd.exe, ConEmu, and other Windows terminal emulators.

**Q: Why use psmux instead of Windows Terminal tabs?**
A: psmux offers session persistence (detach/reattach), synchronized input to multiple panes, full tmux command scripting, hooks, format engine, and tmux-compatible keybindings. Windows Terminal tabs can't do any of that.

**Q: Can I use my existing `.tmux.conf`?**
A: Yes! psmux reads `~/.tmux.conf` automatically. Most tmux config options, key bindings, and style settings work as-is.

**Q: Can I use tmux themes?**
A: Yes. psmux supports 14 style options with 24-bit true color, 256 indexed colors, and text attributes (bold, italic, dim, etc.). Most tmux theme configs are compatible.

**Q: Can I use tmux commands with psmux?**
A: Yes! psmux includes a `tmux` alias. Commands like `tmux new-session`, `tmux attach`, `tmux ls`, `tmux split-window` all work. 76 commands in total.

**Q: How fast is psmux?**
A: Session creation takes < 100ms. New windows/panes add < 80ms overhead. The bottleneck is your shell's startup time, not psmux. Compiled with opt-level 3 and full LTO.

**Q: Does psmux support mouse?**
A: Full mouse support: click to focus panes, drag to resize borders, scroll wheel, click status-bar tabs, drag-select text, right-click copy. Plus VT mouse forwarding for TUI apps like vim, htop, and midnight commander.

**Q: What shells does psmux support?**
A: PowerShell 7 (default), PowerShell 5, cmd.exe, Git Bash, WSL, nushell, and any Windows executable. Change with `set -g default-shell <shell>`.

**Q: Is it stable for daily use?**
A: Yes. psmux is stress-tested with 15+ rapid windows, 18+ concurrent panes, 5 concurrent sessions, kill+recreate cycles, and sustained load, all with zero hangs or resource leaks.
