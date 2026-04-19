
## Complete Command Table

| Command | Alias | Args Template | Min | Max | Source File |
|---|---|---|---|---|---|
| `attach-session` | `attach` | `"ErdD:f:c:t:x:"` | 0 | 0 | cmd-attach-session.c *(not fetched, from Perplexity)* |
| `bind-key` | `bind` | `"nrN:T:"` | 1 | -1 | cmd-bind-key.c |
| `break-pane` | `breakp` | `"abdPF:n:s:t:"` | 0 | 0 | cmd-break-pane.c |
| `capture-pane` | `capturep` | `"ab:CeE:JMNpPqS:Tt:"` | 0 | 0 | cmd-capture-pane.c |
| `choose-buffer` | *(none)* | `"F:f:K:NO:rt:yZ"` | 0 | 1 | cmd-choose-tree.c |
| `choose-client` | *(none)* | `"F:f:K:NO:rt:yZ"` | 0 | 1 | cmd-choose-tree.c |
| `choose-tree` | *(none)* | `"F:f:GK:NO:rst:wyZ"` | 0 | 1 | cmd-choose-tree.c |
| `clear-history` | `clearhist` | `"Ht:"` | 0 | 0 | cmd-capture-pane.c |
| `clock-mode` | *(none)* | `"t:"` | 0 | 0 | cmd-copy-mode.c |
| `command-prompt` | *(none)* | `"1beFiklI:Np:t:T:"` | 0 | 1 | cmd-command-prompt.c |
| `confirm-before` | `confirm` | `"bc:p:t:y"` | 1 | 1 | cmd-confirm-before.c |
| `copy-mode` | *(none)* | `"deHMqSs:t:u"` | 0 | 0 | cmd-copy-mode.c |
| `customize-mode` | *(none)* | `"F:f:Nt:yZ"` | 0 | 0 | cmd-choose-tree.c |
| `delete-buffer` | `deleteb` | `"b:"` | 0 | 0 | cmd-paste-buffer.c *(delete in same file)* |
| `detach-client` | `detach` | `"aE:s:t:P"` | 0 | 0 | cmd-detach-client.c |
| `display-message` | `display` | `"aCc:d:lINpt:F:v"` | 0 | 1 | cmd-display-message.c |
| `display-panes` | `displayp` | `"bd:Nt:"` | 0 | 1 | cmd-display-panes.c |
| `has-session` | `has` | `"t:"` | 0 | 0 | cmd-select-window.c *(in same file)* |
| `if-shell` | `if` | `"bFt:"` | 2 | 3 | cmd-if-shell.c |
| `join-pane` | `joinp` | `"bdfhvp:l:s:t:"` | 0 | 0 | cmd-join-pane.c |
| `kill-pane` | `killp` | `"at:"` | 0 | 0 | cmd-kill-pane.c |
| `kill-server` | *(none)* | `""` | 0 | 0 | cmd-kill-server.c |
| `kill-session` | *(none)* | `"aCt:"` | 0 | 0 | cmd-kill-session.c |
| `kill-window` | `killw` | `"at:"` | 0 | 0 | cmd-kill-window.c |
| `last-pane` | `lastp` | `"det:Z"` | 0 | 0 | cmd-select-pane.c |
| `last-window` | `last` | `"t:"` | 0 | 0 | cmd-select-window.c |
| `link-window` | `linkw` | `"abdks:t:"` | 0 | 0 | cmd-move-window.c |
| `list-buffers` | `lsb` | `"F:f:O:r"` | 0 | 0 | cmd-list-buffers.c |
| `list-clients` | `lsc` | `"F:f:O:rt:"` | 0 | 0 | cmd-list-clients.c |
| `list-commands` | `lscm` | `"F:"` | 0 | 1 | cmd-list-keys.c |
| `list-keys` | `lsk` | `"1aNP:T:"` | 0 | 1 | cmd-list-keys.c |
| `list-panes` | `lsp` | `"aF:f:O:rst:"` | 0 | 0 | cmd-list-panes.c |
| `list-sessions` | `ls` | `"F:f:O:r"` | 0 | 0 | cmd-list-sessions.c |
| `list-windows` | `lsw` | `"aF:f:O:rt:"` | 0 | 0 | cmd-list-windows.c |
| `load-buffer` | `loadb` | `"b:t:w"` | 1 | 1 | cmd-load-buffer.c |
| `lock-client` | `lockc` | `"t:"` | 0 | 0 | cmd-lock-server.c |
| `lock-server` | `lock` | `""` | 0 | 0 | cmd-lock-server.c |
| `lock-session` | `locks` | `"t:"` | 0 | 0 | cmd-lock-server.c |
| `move-pane` | `movep` | `"bdfhvp:l:s:t:"` | 0 | 0 | cmd-join-pane.c |
| `move-window` | `movew` | `"abdkrs:t:"` | 0 | 0 | cmd-move-window.c |
| `new-session` | `new` | `"Ac:dDe:EF:f:n:Ps:t:x:Xy:"` | 0 | -1 | cmd-new-session.c |
| `new-window` | `neww` | `"abc:de:F:kn:PSt:"` | 0 | -1 | cmd-new-window.c |
| `next-window` | `next` | `"at:"` | 0 | 0 | cmd-select-window.c |
| `paste-buffer` | `pasteb` | `"db:prs:t:"` | 0 | 0 | cmd-paste-buffer.c |
| `pipe-pane` | `pipep` | `"IOot:"` | 0 | 1 | cmd-pipe-pane.c |
| `previous-window` | `prev` | `"at:"` | 0 | 0 | cmd-select-window.c |
| `refresh-client` | `refresh` | `"A:B:cC:Df:r:F:lLRSt:U"` | 0 | 1 | cmd-refresh-client.c |
| `rename-session` | `rename` | `"t:"` | 1 | 1 | cmd-rename-session.c |
| `rename-window` | `renamew` | `"t:"` | 1 | 1 | cmd-rename-window.c |
| `resize-pane` | `resizep` | `"DLMRTt:Ux:y:Z"` | 0 | 1 | cmd-resize-pane.c |
| `respawn-pane` | `respawnp` | `"c:e:kt:"` | 0 | -1 | cmd-respawn-pane.c |
| `respawn-window` | `respawnw` | `"c:e:kt:"` | 0 | -1 | cmd-respawn-window.c |
| `rotate-window` | `rotatew` | `"Dt:UZ"` | 0 | 0 | cmd-rotate-window.c |
| `run-shell` | `run` | `"bd:Ct:Es:c:"` | 0 | 1 | cmd-run-shell.c |
| `save-buffer` | `saveb` | `"ab:"` | 1 | 1 | cmd-save-buffer.c |
| `select-pane` | `selectp` | `"DdegLlMmP:RT:t:UZ"` | 0 | 0 | cmd-select-pane.c |
| `select-window` | `selectw` | `"lnpTt:"` | 0 | 0 | cmd-select-window.c |
| `send-keys` | `send` | `"c:FHKlMN:Rt:X"` | 0 | -1 | cmd-send-keys.c |
| `send-prefix` | *(none)* | `"2t:"` | 0 | 0 | cmd-send-keys.c |
| `set-buffer` | `setb` | `"ab:t:n:w"` | 0 | 1 | cmd-set-buffer.c |
| `set-environment` | `setenv` | `"Fhgrt:u"` | 1 | 2 | cmd-set-environment.c |
| `set-hook` | *(none)* | `"agpRt:uw"` | 1 | 2 | cmd-set-option.c |
| `set-option` | `set` | `"aFgopqst:uUw"` | 1 | 2 | cmd-set-option.c |
| `set-window-option` | `setw` | `"aFgoqt:u"` | 1 | 2 | cmd-set-option.c |
| `show-buffer` | `showb` | `"b:"` | 0 | 0 | cmd-save-buffer.c |
| `show-environment` | `showenv` | `"hgst:"` | 0 | 1 | cmd-show-environment.c |
| `show-hooks` | *(none)* | `"gpt:w"` | 0 | 1 | cmd-show-options.c |
| `show-options` | `show` | `"AgHpqst:vw"` | 0 | 1 | cmd-show-options.c |
| `show-window-options` | `showw` | `"gvt:"` | 0 | 1 | cmd-show-options.c |
| `source-file` | `source` | `"t:Fnqv"` | 1 | -1 | cmd-source-file.c |
| `split-window` | `splitw` | `"bc:de:fF:hIl:p:Pt:vZ"` | 0 | -1 | cmd-split-window.c |
| `start-server` | `start` | `""` | 0 | 0 | cmd-kill-server.c |
| `suspend-client` | `suspendc` | `"t:"` | 0 | 0 | cmd-detach-client.c |
| `swap-pane` | `swapp` | `"dDs:t:UZ"` | 0 | 0 | cmd-swap-pane.c |
| `swap-window` | `swapw` | `"ds:t:"` | 0 | 0 | cmd-swap-window.c |
| `switch-client` | `switchc` | `"c:EFlnO:pt:rT:Z"` | 0 | 0 | cmd-switch-client.c |
| `unbind-key` | `unbind` | `"anqT:"` | 0 | 1 | cmd-unbind-key.c |
| `unlink-window` | `unlinkw` | `"kt:"` | 0 | 0 | cmd-kill-window.c |
| `wait-for` | `wait` | `"LSU"` | 1 | 1 | cmd-wait-for.c |

## Flag Details by Command

### Session Commands

**new-session** (`new`) — `"Ac:dDe:EF:f:n:Ps:t:x:Xy:"`
- Boolean: `-A` (attach if exists), `-d` (detach), `-D` (detach other), `-E` (no environ update), `-P` (print info), `-X` (no default-command exec), `-x` → **wait, x: takes value**
- Value: `-c` (start-dir), `-e` (environment), `-F` (format), `-f` (flags), `-n` (window-name), `-s` (session-name), `-t` (group target), `-x` (width), `-y` (height)

**has-session** (`has`) — `"t:"`
- Value: `-t` (target session)

**kill-session** — `"aCt:"`
- Boolean: `-a` (kill all other), `-C` (clear alerts)
- Value: `-t` (target session)

**rename-session** (`rename`) — `"t:"`  [1 positional arg: new-name]
- Value: `-t` (target session)

**list-sessions** (`ls`) — `"F:f:O:r"`
- Boolean: `-r` (reverse sort)
- Value: `-F` (format), `-f` (filter), `-O` (sort order)

**switch-client** (`switchc`) — `"c:EFlnO:pt:rT:Z"`
- Boolean: `-E` (no environ), `-F` → **wait, F: no-colon = boolean here**, `-l` (last), `-n` (next), `-p` (previous), `-r` (toggle readonly), `-Z` (zoom)
- Value: `-c` (client), `-O` (sort order), `-t` (target), `-T` (key-table)

**detach-client** (`detach`) — `"aE:s:t:P"`
- Boolean: `-a` (all other), `-P` (kill after detach)
- Value: `-E` (shell-command), `-s` (target-session), `-t` (target-client)

**suspend-client** (`suspendc`) — `"t:"`
- Value: `-t` (target-client)

**lock-server** (`lock`) — `""`
- No flags

**lock-session** (`locks`) — `"t:"`
- Value: `-t` (target-session)

**lock-client** (`lockc`) — `"t:"`
- Value: `-t` (target-client)

**kill-server** — `""`
- No flags

**start-server** (`start`) — `""`
- No flags

**list-clients** (`lsc`) — `"F:f:O:rt:"`
- Boolean: `-r` (reverse sort)
- Value: `-F` (format), `-f` (filter), `-O` (sort order), `-t` (target-session)

**refresh-client** (`refresh`) — `"A:B:cC:Df:r:F:lLRSt:U"`
- Boolean: `-c` (clear pan), `-D` (pan down), `-l` (clipboard query), `-L` (pan left), `-R` (pan right), `-S` (status only), `-U` (pan up)
- Value: `-A` (pane:state), `-B` (subscription), `-C` (size), `-f` (flags), `-r` (pane:report), `-F` (flags alias), `-t` (target-client)

### Window Commands

**new-window** (`neww`) — `"abc:de:F:kn:PSt:"`
- Boolean: `-a` (after current), `-b` (before current), `-d` (don't switch), `-k` (kill if exists), `-P` (print info), `-S` (select if exists)
- Value: `-c` (start-dir), `-e` (environment), `-F` (format), `-n` (window-name), `-t` (target-window)

**kill-window** (`killw`) — `"at:"`
- Boolean: `-a` (kill all other)
- Value: `-t` (target-window)

**unlink-window** (`unlinkw`) — `"kt:"`
- Boolean: `-k` (kill if last)
- Value: `-t` (target-window)

**rename-window** (`renamew`) — `"t:"`  [1 positional arg: new-name]
- Value: `-t` (target-window)

**select-window** (`selectw`) — `"lnpTt:"`
- Boolean: `-l` (last), `-n` (next), `-p` (previous), `-T` (toggle)
- Value: `-t` (target-window)

**next-window** (`next`) — `"at:"`
- Boolean: `-a` (with alert)
- Value: `-t` (target-session)

**previous-window** (`prev`) — `"at:"`
- Boolean: `-a` (with alert)
- Value: `-t` (target-session)

**last-window** (`last`) — `"t:"`
- Value: `-t` (target-session)

**move-window** (`movew`) — `"abdkrs:t:"`
- Boolean: `-a` (after), `-b` (before), `-d` (detach), `-k` (kill if exists), `-r` (renumber)
- Value: `-s` (src-window), `-t` (dst-window)

**link-window** (`linkw`) — `"abdks:t:"`
- Boolean: `-a` (after), `-b` (before), `-d` (detach), `-k` (kill if exists)
- Value: `-s` (src-window), `-t` (dst-window)

**swap-window** (`swapw`) — `"ds:t:"`
- Boolean: `-d` (don't switch)
- Value: `-s` (src-window), `-t` (dst-window)

**rotate-window** (`rotatew`) — `"Dt:UZ"`
- Boolean: `-D` (down/clockwise), `-U` (up/counter-clockwise), `-Z` (keep zoomed)
- Value: `-t` (target-window)

**list-windows** (`lsw`) — `"aF:f:O:rt:"`
- Boolean: `-a` (all sessions), `-r` (reverse sort)
- Value: `-F` (format), `-f` (filter), `-O` (sort order), `-t` (target-session)

**respawn-window** (`respawnw`) — `"c:e:kt:"`
- Boolean: `-k` (kill existing)
- Value: `-c` (start-dir), `-e` (environment), `-t` (target-window)

### Pane Commands

**split-window** (`splitw`) — `"bc:de:fF:hIl:p:Pt:vZ"`
- Boolean: `-b` (before), `-d` (don't switch), `-f` (full width/height), `-h` (horizontal), `-I` (stdin forward), `-P` (print info), `-v` (vertical), `-Z` (zoom)
- Value: `-c` (start-dir), `-e` (environment), `-F` (format), `-l` (size), `-p` (percentage), `-t` (target-pane)

**select-pane** (`selectp`) — `"DdegLlMmP:RT:t:UZ"`
- Boolean: `-D` (down), `-d` (disable input), `-e` (enable input), `-g` (show style), `-L` (left), `-l` (last), `-M` (clear marked), `-m` (mark), `-R` (right), `-U` (up), `-Z` (keep zoomed)
- Value: `-P` (style), `-T` (title), `-t` (target-pane)

**last-pane** (`lastp`) — `"det:Z"`
- Boolean: `-d` (disable input), `-e` (enable input), `-Z` (keep zoomed)
- Value: `-t` (target-window)

**kill-pane** (`killp`) — `"at:"`
- Boolean: `-a` (kill all other)
- Value: `-t` (target-pane)

**resize-pane** (`resizep`) — `"DLMRTt:Ux:y:Z"`
- Boolean: `-D` (down), `-L` (left), `-M` (mouse), `-R` (right), `-T` (trim), `-U` (up), `-Z` (zoom toggle)
- Value: `-t` (target-pane), `-x` (width), `-y` (height)

**swap-pane** (`swapp`) — `"dDs:t:UZ"`
- Boolean: `-d` (don't switch focus), `-D` (swap down), `-U` (swap up), `-Z` (keep zoomed)
- Value: `-s` (src-pane), `-t` (dst-pane)

**join-pane** (`joinp`) — `"bdfhvp:l:s:t:"`
- Boolean: `-b` (before), `-d` (don't switch), `-f` (full size), `-h` (horizontal), `-v` (vertical)
- Value: `-p` (percentage), `-l` (size), `-s` (src-pane), `-t` (dst-pane)

**move-pane** (`movep`) — `"bdfhvp:l:s:t:"`
- *(Same flags as join-pane)*

**break-pane** (`breakp`) — `"abdPF:n:s:t:"`
- Boolean: `-a` (after), `-b` (before), `-d` (don't switch), `-P` (print info)
- Value: `-F` (format), `-n` (window-name), `-s` (src-pane), `-t` (dst-window)

**respawn-pane** (`respawnp`) — `"c:e:kt:"`
- Boolean: `-k` (kill existing)
- Value: `-c` (start-dir), `-e` (environment), `-t` (target-pane)

**capture-pane** (`capturep`) — `"ab:CeE:JMNpPqS:Tt:"`
- Boolean: `-a` (alt screen), `-C` (escape non-printable as C0), `-e` (escape sequences), `-J` (join wrapped lines), `-M` (mouse target), `-N` (with trailing spaces), `-p` (to stdout), `-P` (only if pane active), `-q` (quiet), `-T` (ignore trailing positions)
- Value: `-b` (buffer-name), `-E` (end-line), `-S` (start-line), `-t` (target-pane)

**clear-history** (`clearhist`) — `"Ht:"`
- Boolean: `-H` (also clear hidden history)
- Value: `-t` (target-pane)

**list-panes** (`lsp`) — `"aF:f:O:rst:"`
- Boolean: `-a` (all), `-r` (reverse sort), `-s` (session)
- Value: `-F` (format), `-f` (filter), `-O` (sort order), `-t` (target)

**display-panes** (`displayp`) — `"bd:Nt:"`
- Boolean: `-b` (non-blocking), `-N` (no key handling)
- Value: `-d` (duration), `-t` (target-client)

**pipe-pane** (`pipep`) — `"IOot:"`
- Boolean: `-I` (stdin), `-O` (stdout), `-o` (toggle/open-only)
- Value: `-t` (target-pane)

### Copy & Paste Commands

**copy-mode** — `"deHMqSs:t:u"`
- Boolean: `-d` (page down), `-e` (exit at bottom), `-H` (hide position), `-M` (mouse), `-q` (cancel), `-S` (scroll bar drag), `-u` (page up)
- Value: `-s` (src-pane), `-t` (target-pane)

**paste-buffer** (`pasteb`) — `"db:prs:t:"`
- Boolean: `-d` (delete after), `-p` (use bracketed paste), `-r` (no newline replacement)
- Value: `-b` (buffer-name), `-s` (separator), `-t` (target-pane)

**set-buffer** (`setb`) — `"ab:t:n:w"`
- Boolean: `-a` (append), `-w` (send to clipboard)
- Value: `-b` (buffer-name), `-t` (target-client), `-n` (new-name)

**delete-buffer** (`deleteb`) — `"b:"`
- Value: `-b` (buffer-name)

**show-buffer** (`showb`) — `"b:"`
- Value: `-b` (buffer-name)

**save-buffer** (`saveb`) — `"ab:"`  [1 positional arg: path]
- Boolean: `-a` (append)
- Value: `-b` (buffer-name)

**load-buffer** (`loadb`) — `"b:t:w"`  [1 positional arg: path]
- Boolean: `-w` (send to clipboard)
- Value: `-b` (buffer-name), `-t` (target-client)

**list-buffers** (`lsb`) — `"F:f:O:r"`
- Boolean: `-r` (reverse sort)
- Value: `-F` (format), `-f` (filter), `-O` (sort order)

**choose-buffer** — `"F:f:K:NO:rt:yZ"`
- Boolean: `-N` (hide preview), `-r` (reverse sort), `-y` (immediate exit), `-Z` (zoom)
- Value: `-F` (format), `-f` (filter), `-K` (key-format), `-O` (sort order), `-t` (target-pane)

### Key Binding Commands

**bind-key** (`bind`) — `"nrN:T:"`
- Boolean: `-n` (root table / no prefix), `-r` (repeat)
- Value: `-N` (note), `-T` (key-table)

**unbind-key** (`unbind`) — `"anqT:"`
- Boolean: `-a` (all), `-n` (root table), `-q` (quiet)
- Value: `-T` (key-table)

**list-keys** (`lsk`) — `"1aNP:T:"`
- Boolean: `-1` (one key per line), `-a` (with notes), `-N` (with notes only)
- Value: `-P` (prefix), `-T` (key-table)

**send-keys** (`send`) — `"c:FHKlMN:Rt:X"`
- Boolean: `-F` (expand formats), `-H` (hex), `-K` (key name), `-l` (literal), `-M` (mouse), `-R` (reset terminal), `-X` (copy-mode command)
- Value: `-c` (target-client), `-N` (repeat count), `-t` (target-pane)

**send-prefix** — `"2t:"`
- Boolean: `-2` (send prefix2)
- Value: `-t` (target-pane)

### Configuration Commands

**set-option** (`set`) — `"aFgopqst:uUw"`
- Boolean: `-a` (append), `-F` (expand formats), `-g` (global), `-o` (no overwrite), `-p` (pane), `-q` (quiet), `-s` (server), `-u` (unset), `-U` (unset and delete), `-w` (window)
- Value: `-t` (target)

**set-window-option** (`setw`) — `"aFgoqt:u"`
- Boolean: `-a` (append), `-F` (expand formats), `-g` (global), `-o` (no overwrite), `-q` (quiet), `-u` (unset)
- Value: `-t` (target-window)

**show-options** (`show`) — `"AgHpqst:vw"`
- Boolean: `-A` (inherited), `-g` (global), `-H` (include hidden), `-p` (pane), `-q` (quiet), `-s` (server), `-v` (value only), `-w` (window)
- Value: `-t` (target)

**show-window-options** (`showw`) — `"gvt:"`
- Boolean: `-g` (global), `-v` (value only)
- Value: `-t` (target-window)

**set-hook** — `"agpRt:uw"`
- Boolean: `-a` (append), `-g` (global), `-p` (pane), `-R` (run immediately), `-u` (unset), `-w` (window)
- Value: `-t` (target)

**show-hooks** — `"gpt:w"`
- Boolean: `-g` (global), `-p` (pane), `-w` (window)
- Value: `-t` (target)

**set-environment** (`setenv`) — `"Fhgrt:u"`
- Boolean: `-F` (expand format), `-h` (hidden), `-g` (global), `-r` (remove from env), `-u` (unset)
- Value: `-t` (target-session)

**show-environment** (`showenv`) — `"hgst:"`
- Boolean: `-h` (hidden only), `-g` (global), `-s` (as shell commands)
- Value: `-t` (target-session)

**source-file** (`source`) — `"t:Fnqv"`
- Boolean: `-F` (expand format), `-n` (syntax check only), `-q` (quiet), `-v` (verbose)
- Value: `-t` (target-pane)

**list-commands** (`lscm`) — `"F:"`
- Value: `-F` (format)

### Display & Misc Commands

**display-message** (`display`) — `"aCc:d:lINpt:F:v"`
- Boolean: `-a` (list all variables), `-C` (escape output), `-l` (log to server), `-I` (stdin), `-N` (no output), `-p` (to stdout), `-v` (verbose)
- Value: `-c` (target-client), `-d` (delay), `-t` (target-pane), `-F` (format)

**command-prompt** — `"1beFiklI:Np:t:T:"`
- Boolean: `-1` (single key), `-b` (background), `-e` (backspace exit), `-F` (expand), `-i` (incremental), `-k` (key only), `-l` (literal), `-N` (numeric)
- Value: `-I` (inputs), `-p` (prompts), `-t` (target-client), `-T` (prompt-type)

**confirm-before** (`confirm`) — `"bc:p:t:y"`
- Boolean: `-b` (background), `-y` (default yes)
- Value: `-c` (confirm-key), `-p` (prompt), `-t` (target-client)

**choose-tree** — `"F:f:GK:NO:rst:wyZ"`
- Boolean: `-G` (grouped sessions), `-N` (no preview), `-r` (reverse), `-s` (sessions only), `-w` (windows only), `-y` (immediate exit), `-Z` (zoom)
- Value: `-F` (format), `-f` (filter), `-K` (key-format), `-O` (sort order), `-t` (target-pane)

**choose-client** — `"F:f:K:NO:rt:yZ"`
- Boolean: `-N` (no preview), `-r` (reverse), `-y` (immediate exit), `-Z` (zoom)
- Value: `-F` (format), `-f` (filter), `-K` (key-format), `-O` (sort order), `-t` (target-pane)

**run-shell** (`run`) — `"bd:Ct:Es:c:"`
- Boolean: `-b` (background), `-C` (command), `-E` → **wait, no-colon = boolean**
- Value: `-d` (delay), `-t` (target-pane), `-s` (shell), `-c` (start-dir)

**if-shell** (`if`) — `"bFt:"`  [2-3 positional args: shell-cmd, if-true-cmd, [if-false-cmd]]
- Boolean: `-b` (background), `-F` (test as format not shell)
- Value: `-t` (target-pane)

**wait-for** (`wait`) — `"LSU"`  [1 positional arg: channel]
- Boolean: `-L` (lock), `-S` (signal/unlock), `-U` (unlock)

**clock-mode** — `"t:"`
- Value: `-t` (target-pane)
