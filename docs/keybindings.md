# Key Bindings

Default prefix: `Ctrl+b` (same as tmux). Change with `set -g prefix C-a`.

Supported prefix keys include `C-a` through `C-z`, `C-Space`, and any printable character.

## Case Sensitivity

Key bindings are **case-sensitive**, matching tmux behavior:

- `bind-key t` binds to lowercase `t` (just press `t`)
- `bind-key T` binds to uppercase `T` (`Shift+t`)

This is essential for plugins like PPM (`Prefix+I`/`Prefix+U`) and psmux-sensible (`Prefix+R`).

## Prefix Keys

### Window Management

| Key | Action |
|-----|--------|
| `Prefix + c` | Create new window |
| `Prefix + n` | Next window |
| `Prefix + p` | Previous window |
| `Prefix + l` | Last (previously active) window |
| `Prefix + w` | Interactive session/window/pane chooser (`choose-tree`) |
| `Prefix + &` | Kill current window (with confirmation) |
| `Prefix + ,` | Rename current window |
| `Prefix + '` | Prompt for window index (jump to any window) |
| `Prefix + 0-9` | Select window by number |

### Pane Splitting

| Key | Action |
|-----|--------|
| `Prefix + %` | Split pane left/right (horizontal) |
| `Prefix + "` | Split pane top/bottom (vertical) |

### Pane Navigation

| Key | Action |
|-----|--------|
| `Prefix + Arrow` | Navigate between panes (Up/Down/Left/Right), wraps at edges |
| `Prefix + o` | Select next pane (rotate) |
| `Prefix + ;` | Last (previously active) pane |
| `Prefix + q` | Display pane numbers (type number to switch, auto-dismisses) |

### Pane Management

| Key | Action |
|-----|--------|
| `Prefix + x` | Kill current pane (with confirmation) |
| `Prefix + z` | Toggle pane zoom (fullscreen) |
| `Prefix + {` | Swap pane up |
| `Prefix + }` | Swap pane down |
| `Prefix + !` | Break pane out to new window |

### Pane Resize

| Key | Action |
|-----|--------|
| `Prefix + Ctrl+Arrow` | Resize pane by 1 cell |
| `Prefix + Alt+Arrow` | Resize pane by 5 cells |

### Layout

| Key | Action |
|-----|--------|
| `Prefix + Space` | Cycle to next layout |
| `Prefix + Alt+1` | Even-horizontal layout |
| `Prefix + Alt+2` | Even-vertical layout |
| `Prefix + Alt+3` | Main-horizontal layout |
| `Prefix + Alt+4` | Main-vertical layout |
| `Prefix + Alt+5` | Tiled layout |

### Session

| Key | Action |
|-----|--------|
| `Prefix + d` | Detach from session |
| `Prefix + $` | Rename session |
| `Prefix + s` | Session chooser/switcher (`choose-tree -s`) |
| `Prefix + (` | Switch to previous session |
| `Prefix + )` | Switch to next session |

### Copy / Paste

| Key | Action |
|-----|--------|
| `Prefix + [` | Enter copy/scroll mode |
| `Prefix + ]` | Paste from buffer |
| `Prefix + =` | Interactive buffer chooser |

### Miscellaneous

| Key | Action |
|-----|--------|
| `Prefix + :` | Command prompt (with cursor, arrow key navigation, and history) |
| `Prefix + ?` | List keybindings (help overlay) |
| `Prefix + i` | Display window/pane info |
| `Prefix + t` | Clock mode |
| `Prefix + !` | Break pane out to new window |

### Repeat Bindings

Navigation and resize bindings support **repeat mode**: after pressing the prefix key once, successive keypresses within the `repeat-time` window (default 500ms) trigger the action without needing to re-enter the prefix. This applies to arrow-based pane navigation and resize bindings by default.

## Command Prompt

Press `Prefix + :` to open the command prompt at the bottom of the screen. You can type any psmux/tmux command here.

### Command Prompt Editing Keys

| Key | Action |
|-----|--------|
| `Left` / `Right` | Move cursor within the command |
| `Home` / `Ctrl+A` | Jump to start of line |
| `End` / `Ctrl+E` | Jump to end of line |
| `Backspace` | Delete character before cursor |
| `Delete` | Delete character at cursor |
| `Up` / `Down` | Browse command history (previous/next) |
| `Tab` | Command name completion |
| `Enter` | Execute the command |
| `Escape` | Cancel and close the prompt |

The command prompt remembers your history across the session. Use Up/Down arrows to recall previous commands.

You can run any command from the prompt that you would run from the CLI. For example:

- `:split-window -h` to split horizontally
- `:new-window -n logs` to create a named window
- `:source-file ~/.psmux.conf` to reload your config
- `:set -g status-style "bg=blue"` to change a setting live
- `:list-keys` to see all current key bindings

## Copy/Scroll Mode (Vi)

Enter copy mode with `Prefix + [` to scroll through terminal history with vim-style keybindings.

Mouse scroll wheel also enters copy mode by default. To disable this, set `scroll-enter-copy-mode off` in your config.

### Cursor Movement

| Key | Action |
|-----|--------|
| `h` / `Left` | Move cursor left |
| `j` / `Down` | Move cursor down |
| `k` / `Up` | Move cursor up |
| `l` / `Right` | Move cursor right |

### Word Motions

| Key | Action |
|-----|--------|
| `w` / `b` / `e` | Next word / prev word / end of word |
| `W` / `B` / `E` | WORD variants (whitespace-delimited) |

### Line Motions

| Key | Action |
|-----|--------|
| `0` / `Home` | Start of line |
| `$` / `End` | End of line |
| `^` | First non-blank character |

### Scrolling

| Key | Action |
|-----|--------|
| `Ctrl+u` / `Ctrl+d` | Half page up / down |
| `Ctrl+b` / `PageUp` | Full page up |
| `Ctrl+f` / `PageDown` | Full page down |
| `g` | Top of scrollback |
| `G` | Bottom (live output) |

### Screen Position

| Key | Action |
|-----|--------|
| `H` | Jump to top of visible area |
| `M` | Jump to middle of visible area |
| `L` | Jump to bottom of visible area |

### Character Find

| Key | Action |
|-----|--------|
| `f{char}` / `F{char}` | Find char forward / backward |
| `t{char}` / `T{char}` | Till char forward / backward |

### Bracket / Paragraph

| Key | Action |
|-----|--------|
| `%` | Jump to matching bracket (`()`, `[]`, `{}`, `<>`) |
| `{` | Jump to previous paragraph (blank line) |
| `}` | Jump to next paragraph (blank line) |

### Selection

| Key | Action |
|-----|--------|
| `Space` | Begin character selection |
| `v` | Toggle rectangle selection |
| `V` | Line selection |
| `Ctrl+v` | Toggle rectangle selection |
| `o` | Swap cursor/anchor ends |

### Yank (Copy)

| Key | Action |
|-----|--------|
| `y` / `Enter` | Copy selection and exit |
| `D` | Copy to end of line and exit |
| `A` | Append selection to buffer |

### Search

| Key | Action |
|-----|--------|
| `/` | Search forward |
| `?` | Search backward |
| `n` / `N` | Next / previous match |

### Text Objects & Registers

| Key | Action |
|-----|--------|
| `"a`–`"z` | Named registers (set register for next yank) |
| `aw` / `iw` | Select a word / inner word |
| `aW` / `iW` | Select a WORD / inner WORD |
| `1`–`9` | Numeric prefix for motions (up to 9999) |

### Exit

| Key | Action |
|-----|--------|
| `Esc` / `q` | Exit copy mode |
| `Ctrl+C` / `Ctrl+G` | Exit copy mode |

### Copy Mode Search Input

| Key | Action |
|-----|--------|
| `Esc` | Cancel search |
| `Enter` | Accept search / jump to match |
| `Backspace` | Delete character |
| Any char | Append to search pattern |

### Emacs Copy Mode

When `set mode-keys emacs`, additional bindings are available:

| Key | Action |
|-----|--------|
| `Ctrl+N` / `Ctrl+P` | Scroll down / up 1 line |
| `Ctrl+A` / `Ctrl+E` | Line start / end |
| `Ctrl+V` | Page down |
| `Alt+V` | Page up |
| `Alt+F` / `Alt+B` | Word forward / backward |
| `Alt+W` | Yank and exit |
| `Ctrl+S` / `Ctrl+R` | Search forward / backward |

When in copy mode:
- The pane border turns **yellow**
- `[copy mode]` appears in the title
- A scroll position indicator shows in the top-right corner
- Mouse drag-select copies to Windows clipboard on release

## Command Prompt

Open with `Prefix + :`:

| Key | Action |
|-----|--------|
| `Esc` | Cancel |
| `Enter` | Execute command (saved to history) |
| `Backspace` / `Delete` | Delete character |
| `Left` / `Right` | Move cursor |
| `Home` / `Ctrl+A` | Start of line |
| `End` / `Ctrl+E` | End of line |
| `Up` / `Down` | Cycle command history |
| `Ctrl+U` | Kill line (clear to start) |
| `Ctrl+K` | Kill to end of line |
| `Ctrl+W` | Delete word backward |

## Mouse Bindings

When `mouse on` (default):

| Action | Behavior |
|--------|----------|
| Left-click status tab | Switch to clicked window |
| Left-click pane | Focus that pane |
| Left-click/drag border | Resize split interactively |
| Scroll up/down | Scroll pane (or enter copy mode at prompt) |
| Mouse drag in copy mode | Select text → auto-copy on release |
| Right-click | Paste clipboard |

## Supported Key Names

Key names for `bind-key` and `send-keys`:

| Key | Name |
|-----|------|
| Arrow keys | `Up`, `Down`, `Left`, `Right` |
| Function keys | `F1` through `F12` |
| Special keys | `Enter`, `Tab`, `Escape`, `Space`, `Backspace` |
| Navigation | `Home`, `End`, `PageUp`, `PageDown`, `Insert`, `Delete` |
| Ctrl modifier | `C-a` through `C-z`, `C-Space` |
| Alt modifier | `M-a` through `M-z`, `M-Left`, `M-Right`, etc. |
| Shift+key | Use uppercase letter: `T` for `Shift+t` |
| Shift+Enter | `S-Enter` (sends proper escape sequence) |
| Shift+Tab | `BTab` (sends `ESC [ Z`) |

## Custom Key Bindings

```tmux
# Bind in prefix table (default)
bind-key h split-window -h
bind-key v split-window -v

# Bind in root table (no prefix needed)
bind-key -n C-h select-pane -L

# Repeatable binding (stay in prefix mode)
bind-key -r H resize-pane -L 5

# Unbind a key
unbind-key C-b

# Unbind all
unbind-key -a
```
