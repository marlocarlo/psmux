# Mouse Over SSH

psmux has **first-class mouse support over SSH** when the server runs **Windows 11 build 22523+ (22H2+)**. Click panes, drag-resize borders, scroll, click tabs — everything works, from any SSH client on any OS.

## Compatibility

### Remote access (over SSH)

| Client → Server | Keyboard | Mouse | Notes |
|---|:---:|:---:|---|
| Linux → Windows 11 (22523+) | ✅ | ✅ | Full support |
| macOS → Windows 11 (22523+) | ✅ | ✅ | Full support |
| Windows 10 → Windows 11 (22523+) | ✅ | ✅ | Full support |
| Windows 11 → Windows 11 (22523+) | ✅ | ✅ | Full support |
| WSL → Windows 11 (22523+) | ✅ | ✅ | Full support |
| Any OS → Windows 10 | ✅ | ❌ | ConPTY limitation (see below) |
| Any OS → Windows 11 (pre-22523) | ✅ | ❌ | ConPTY limitation (see below) |

### Local use (no SSH)

| Platform | Keyboard | Mouse |
|---|:---:|:---:|
| Windows 11 (local) | ✅ | ✅ |
| Windows 10 (local) | ✅ | ✅ |

Mouse works perfectly when running psmux locally on both Windows 10 and 11.

## Why No Mouse Over SSH on Windows 10?

Windows 10's ConPTY consumes mouse-enable escape sequences internally and does not forward them to sshd. The SSH client never receives the signal to start sending mouse data. This is a Windows 10 ConPTY limitation that was fixed in Windows 11 (build 22523+). Keyboard input works fully on both versions — only mouse over SSH is affected.

> **Recommendation:** Use Windows 11 build 22523+ (22H2 or later) as your psmux server for full SSH mouse support.
