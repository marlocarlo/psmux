# Plugins & Themes

psmux has a full plugin ecosystem — ports of the most popular tmux plugins, reimplemented in PowerShell for Windows.

## Plugin Repository

**Browse available plugins and themes:** [**psmux-plugins**](https://github.com/psmux/psmux-plugins)

**Install & manage plugins with a TUI:** [**Tmux Plugin Panel**](https://github.com/psmux/Tmux-Plugin-Panel) — a terminal UI for browsing, installing, updating, and removing plugins and themes.

## Available Plugins

| Plugin | Description |
|--------|-------------|
| [psmux-sensible](https://github.com/psmux/psmux-plugins/tree/main/psmux-sensible) | Sensible defaults for psmux |
| [psmux-yank](https://github.com/psmux/psmux-plugins/tree/main/psmux-yank) | Windows clipboard integration |
| [psmux-resurrect](https://github.com/psmux/psmux-plugins/tree/main/psmux-resurrect) | Save/restore sessions |
| [psmux-pain-control](https://github.com/psmux/psmux-plugins/tree/main/psmux-pain-control) | Better pane navigation |
| [psmux-prefix-highlight](https://github.com/psmux/psmux-plugins/tree/main/psmux-prefix-highlight) | Prefix key indicator |
| [ppm](https://github.com/psmux/psmux-plugins/tree/main/ppm) | Plugin manager (like tpm) |

## Themes

Catppuccin · Dracula · Nord · Tokyo Night · Gruvbox

## Quick Start

```powershell
# Install the plugin manager
git clone https://github.com/psmux/psmux-plugins.git "$env:TEMP\psmux-plugins"
Copy-Item "$env:TEMP\psmux-plugins\ppm" "$env:USERPROFILE\.psmux\plugins\ppm" -Recurse
Remove-Item "$env:TEMP\psmux-plugins" -Recurse -Force
```

Then add to your `~/.psmux.conf`:

```tmux
set -g @plugin 'psmux-plugins/ppm'
set -g @plugin 'psmux-plugins/psmux-sensible'
run '~/.psmux/plugins/ppm/ppm.ps1'
```

Press `Prefix + I` inside psmux to install the declared plugins.
