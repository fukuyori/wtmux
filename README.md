# wtmux

A tmux-like terminal multiplexer for Windows, written in Rust.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Windows](https://img.shields.io/badge/platform-Windows-blue.svg)](https://www.microsoft.com/windows)
[![Version](https://img.shields.io/badge/version-1.3.9-green.svg)](https://github.com/fukuyori/wtmux/releases)

[日本語版 README](README.ja.md)

## 1.3.9 Highlights

- Windows input now preserves IME and emoji-panel non-BMP characters such as 🚶.
- PowerShell and PowerShell 7 sessions now set both input and output encoding to UTF-8.
- Key mapping and Windows surrogate-pair handling are covered by tests.

## Features

- **tmux-compatible keybindings** - Familiar `Ctrl+B` prefix commands by default, with configurable shortcuts
- **Multiple tabs (windows)** - Create, switch, rename, and manage tabs
- **Split panes** - Horizontal and vertical splits with resize support
- **Pane zoom** - Toggle full-screen for any pane (v0.4.0: seamless transitions)
- **Layout presets** - 5 layouts (even-horizontal, even-vertical, main-horizontal, main-vertical, tiled)
- **Copy mode** - vim-like scrollback navigation and text selection
- **Search** - Search through scrollback buffer with highlighting
- **Command history** - Record and reuse commands with `Ctrl+R` by default
- **Color schemes** - 8 built-in themes (default, solarized, monokai, nord, dracula, gruvbox, tokyo-night)
- **Configuration** - TOML config file support
- **ConPTY support** - Native Windows pseudo-terminal
- **Multiple shells** - cmd.exe, PowerShell, PowerShell 7, WSL
- **Encoding support** - UTF-8 and Shift-JIS (CP932)
- **Robust rendering** - Thread-safe output with synchronized updates (v0.4.0)
- **Mouse passthrough** - TUI apps receive mouse events (hold Shift for wtmux selection)
- **Nerd Font / Powerline support** - oh-my-posh, Starship, and Powerline prompts render correctly
- **Shell integration** - OSC 133/633 for accurate command history with modern prompts

## Screenshots

```
┌─[0: cmd]─────────────────┬─[1: pwsh]────────────────┐
│ C:\Users\user>           │ PS C:\Users\user>        │
│                          │                          │
│                          ├──────────────────────────┤
│                          │ user@wsl:~$              │
│                          │                          │
└──────────────────────────┴──────────────────────────┘
 [0] cmd [1] pwsh* [2] wsl                    tokyo-night
```

## Requirements

- Windows 10 version 1809 or later (ConPTY support required)
- Rust 1.70 or later (for building from source)

## Installation

### Option 1: Download Release

Download from the [Releases](https://github.com/fukuyori/wtmux/releases) page:

- **Installer** (`wtmux-x.x.x-setup.exe`) - Recommended for most users
- **Portable** (`wtmux-x.x.x-portable-x64.zip`) - No installation required, just extract and run
- **MSI** (`wtmux-x.x.x-x64.msi`) - For enterprise deployment

### Option 2: PowerShell Install Script

```powershell
# Build and install
cargo build --release
.\install.ps1

# To uninstall
.\install.ps1 -Uninstall
```

### Option 3: Build from Source

```bash
git clone https://github.com/fukuyori/wtmux.git
cd wtmux
cargo build --release

# Copy to your preferred location
copy target\release\wtmux.exe C:\your\bin\path\
```

### Building Installers

```powershell
# Portable package (ZIP)
.\build-portable.ps1

# Using Inno Setup (recommended for end users)
# Download from: https://jrsoftware.org/isinfo.php
.\build-inno-installer.ps1

# Using WiX Toolset (for enterprise deployment)
# Download from: https://wixtoolset.org/releases/
# On WiX Toolset v7, the script accepts the OSMF EULA automatically before building.
.\build-installer.ps1

# MSIX package (for Windows 10/11)
# Requires Windows 10 SDK
.\build-msix.ps1              # Unsigned (requires Developer Mode)
.\build-msix.ps1 -Sign        # Self-signed (for testing)

# Regenerate icon assets after editing assets/wtmux-icon.svg
# The generated .ico is embedded into wtmux.exe and reused by the installers.
.\generate-icons.ps1
```

## Usage

```bash
# Default: Multi-pane mode
wtmux

# With PowerShell 7 and UTF-8
wtmux -7 -u

# With WSL
wtmux -w

# Simple single-pane mode
wtmux -1

# Show help
wtmux --help
```

### Command Line Options

| Option | Description |
|--------|-------------|
| `-1, --simple` | Simple single-pane mode |
| `-c, --cmd` | Use Command Prompt (cmd.exe) |
| `-p, --powershell` | Use Windows PowerShell |
| `-7, --pwsh` | Use PowerShell 7 (pwsh.exe) |
| `-w, --wsl` | Use WSL |
| `-s, --shell <CMD>` | Custom shell command |
| `--sjis` | Shift-JIS encoding (default: UTF-8) |
| `-v, --version` | Show version |
| `-h, --help` | Show help |

## Keybindings

Prefix commands use `Ctrl+B` by default (same as tmux), and the prefix key can be changed with `prefix_key`.
The tables below show the default keybindings.

### Windows (Tabs)

| Key | Action |
|-----|--------|
| `Ctrl+B, c` | Create new window |
| `Ctrl+B, &` | Kill current window |
| `Ctrl+B, n` | Next window |
| `Ctrl+B, p` | Previous window |
| `Ctrl+B, l` | Toggle last window |
| `Ctrl+B, 0-9` | Select window by number |
| `Ctrl+B, ,` | Rename window |

### Panes

| Key | Action |
|-----|--------|
| `Ctrl+B, "` | Split horizontally (top/bottom) |
| `Ctrl+B, %` | Split vertically (left/right) |
| `Ctrl+B, x` | Kill current pane |
| `Ctrl+B, o` | Next pane |
| `Ctrl+B, ;` | Previous pane |
| `Ctrl+B, ←↑↓→` | Move focus to pane in direction |
| `Ctrl+B, Ctrl+←↑↓→` | Resize pane |
| `Ctrl+B, z` | Toggle pane zoom |
| `Ctrl+B, Space` | Cycle through layout presets |
| `Ctrl+B, q` | Show pane numbers (then 0-9 to select) |
| `Ctrl+B, {` | Swap with previous pane |
| `Ctrl+B, }` | Swap with next pane |

### Copy Mode

| Key | Action |
|-----|--------|
| `Ctrl+B, [` | Enter copy mode |
| `Ctrl+B, /` | Enter search mode |

In copy mode:

| Key | Action |
|-----|--------|
| `h/j/k/l` or arrows | Move cursor |
| `0` / `$` | Line start / end |
| `g` / `G` | Top / bottom of buffer |
| `Ctrl+U` / `Ctrl+D` | Half page up / down |
| `Ctrl+B` / `Ctrl+F` | Full page up / down |
| `Space` or `v` | Start/toggle selection |
| `Enter` or `y` | Copy selection and exit |
| `/` | Search forward |
| `?` | Search backward |
| `n` / `N` | Next / previous match |
| `q` or `Esc` | Exit copy mode |

### Other

| Key | Action |
|-----|--------|
| `Ctrl+B, t` | Theme selector |
| `Ctrl+B, r` | Reset cursor shape |
| `Ctrl+B, b` | Send Ctrl+B to application |
| `Esc` | Cancel prefix mode |

### Command History

wtmux includes its own command history feature, separate from your shell's built-in history. It records the commands you enter, eliminating the need to retype complex commands repeatedly.
The history selector shortcut defaults to `Ctrl+R` and can be changed with `keybindings.history_selector`.

| Key | Action |
|-----|--------|
| `Ctrl+R` | Show history search |
| `Enter` | Execute selected command (replace current input) |
| `Shift+Enter` | Append with `&&` (run if previous succeeds) |
| `Ctrl+Enter` | Append with `&` (background/parallel) |

For more details, see: https://qiita.com/spumoni/items/7d43ed7e579d99cfda3e

## Shell Integration (Recommended)

wtmux can detect commands precisely — regardless of prompt appearance — when
the shell emits **OSC 133 / OSC 633** markers.  This removes the need for
prompt-pattern heuristics and makes command history accurate even with fancy
prompts (oh-my-posh, Starship, multi-line prompts, etc.).

### PowerShell (automatic)

Add one line to your PowerShell profile (`$PROFILE`):

```powershell
$env:TERM_PROGRAM = "vscode"
```

PowerShell 7 and Windows PowerShell 5 automatically emit OSC 633 markers
when `TERM_PROGRAM` is set to `"vscode"`.

### bash / zsh (WSL)

Add to `~/.bashrc` or `~/.zshrc`:

```bash
# OSC 133 shell integration for wtmux
__wtmux_precmd() { printf '\e]133;A\e\'; }
__wtmux_preexec() { printf '\e]133;C\e\'; }
PS1='\[\e]133;B\e\\]'"$PS1"
# bash: use PROMPT_COMMAND
PROMPT_COMMAND="__wtmux_precmd;${PROMPT_COMMAND}"
# zsh: use precmd/preexec hooks instead
```

### oh-my-posh

Enable the built-in shell integration in your oh-my-posh config:

```json
{ "osc99": true, "osc7": true, "osc133": true }
```

### Fallback (cmd.exe)

`cmd.exe` does not support OSC sequences.  wtmux automatically falls back to
**keystroke tracking** — intercepting every character before it reaches the
shell — which gives accurate results without any shell configuration.

---

## Configuration

wtmux reads configuration from `%LOCALAPPDATA%\wtmux\config.toml`.

```toml
# Default shell (optional)
# Options: "cmd", "powershell", "pwsh", "wsl", or full path
# shell = "pwsh.exe"

# Codepage for encoding (optional)
# codepage = 65001  # UTF-8
# codepage = 932    # Shift-JIS

# Prefix key (default: "C-b" for Ctrl+B)
# prefix_key = "C-a"  # Change to Ctrl+A

# Color scheme
# Available: default, solarized-dark, solarized-light, monokai, nord, dracula, gruvbox-dark, tokyo-night
color_scheme = "tokyo-night"

# Global keybindings outside the prefix mode
[keybindings]
# history_selector = "C-r"      # Also accepts "Ctrl+R"
# scrollback_up = "S-PageUp"
# scrollback_down = "S-PageDown"
# scrollback_top = "S-Home"
# scrollback_bottom = "S-End"
# selection_left = "S-Left"
# selection_right = "S-Right"
# selection_up = "S-Up"
# selection_down = "S-Down"
# copy_selection = "C-S-c"      # Also accepts "Ctrl+Shift+C"

# Tab bar settings
[tab_bar]
visible = true

# Status bar settings
[status_bar]
visible = true
show_time = true

# Pane border settings
[pane]
border_style = "single"  # single, double, rounded, none

# Cursor settings
[cursor]
shape = "block"          # block, underline, bar
blink = true

# Scrollback buffer
[scrollback]
lines = 10000
```

The `[keybindings]` section currently controls these non-prefix shortcuts:
the history selector (`Ctrl+R` by default), scrollback navigation, keyboard selection, and copy-selection.

### Font Settings

```toml
[font]
# Font family (leave empty to inherit from host terminal)
# family = "CaskaydiaCove Nerd Font"

# Font size in points (0 = inherit)
# size = 12

# Suppress SGR 1 (Bold) — set to true if Powerline/Nerd Font glyphs
# look misaligned or replaced by boxes.
# See "Troubleshooting" section below.
# suppress_bold = false
```

### Available Color Schemes

- `default` - Default terminal colors
- `solarized` - Solarized Dark
- `monokai` - Monokai Pro
- `nord` - Nord
- `dracula` - Dracula
- `gruvbox` - Gruvbox Dark
- `tokyo-night` - Tokyo Night

## Detecting wtmux from Shell

wtmux sets environment variables that child processes can detect:

```batch
REM cmd.exe
if defined WTMUX echo Running in wtmux
```

```powershell
# PowerShell
if ($env:WTMUX) { "Running in wtmux" }
```

```bash
# bash/WSL
[ -n "$WTMUX" ] && echo "Running in wtmux"
```

## Mouse Support

wtmux provides comprehensive mouse support:

### Text Selection and Copy

You can select text with the mouse and copy it to the clipboard:

1. **Click and drag** to select text
2. **Release the mouse button** - selected text is automatically copied to clipboard
3. **Paste** with `Ctrl+V` or **right-click → Paste** from the context menu

This works the same as standard terminal text selection.

### Mouse Passthrough for TUI Applications

When running TUI applications that use mouse input (e.g., htop, mc, vim with mouse, or apps using crossterm's `EnableMouseCapture`), mouse events are automatically passed through to the application.

**How it works:**
- wtmux detects when a child application enables mouse tracking (DECSET 1000/1002/1003)
- Mouse events within the pane are forwarded to the application
- Supports SGR extended mouse mode (1006) for terminals larger than 223 columns/rows
- Tab bar and status bar clicks still work as expected

**Text selection in TUI apps:**
- Hold **Shift** while clicking/dragging to use wtmux's text selection instead of passing events to the child application
- This is useful when you need to copy text from a mouse-enabled TUI app

### Mouse Actions Summary

| Action | In normal shell | In TUI app (mouse-enabled) |
|--------|-----------------|---------------------------|
| Left drag | Select text | App receives event |
| Shift + Left drag | Select text | Select text |
| Left click on tab bar | Switch tab | Switch tab |
| Right click | Context menu (Paste, Zoom, Split, etc.) | Context menu |
| Scroll wheel | Scroll buffer | App receives event |

## Comparison with tmux

| Feature | tmux | wtmux |
|---------|------|-------|
| Platform | Unix/Linux/macOS | Windows |
| Backend | PTY | ConPTY |
| Windows/Panes | ✓ | ✓ |
| Keybindings | ✓ | ✓ (compatible) |
| Copy mode | ✓ | ✓ |
| Search | ✓ | ✓ |
| Layout presets | ✓ | ✓ |
| Config file | ✓ | ✓ |
| Color schemes | ✓ | ✓ |
| Mouse support | ✓ | ✓ |
| Detach/Attach | ✓ | Planned |
| Session sharing | ✓ | Planned |
| Scripting | ✓ | Planned |

## Project Structure

```
wtmux/
├── Cargo.toml
├── README.md
├── README.ja.md
├── LICENSE
├── CHANGELOG.md
├── config.example.toml
├── install.ps1
├── generate-icons.ps1
├── assets/
│   ├── wtmux-icon.svg       # Icon source artwork
│   └── generated/           # Generated .ico / preview PNG
├── build-portable.ps1
├── build-installer.ps1
├── build-inno-installer.ps1
├── installer/
│   ├── wtmux.iss          # Inno Setup script
│   ├── wtmux.wxs          # WiX script
│   ├── msix/Assets/       # MSIX icon assets
│   └── license.rtf
└── src/
    ├── main.rs            # Entry point
    ├── config.rs          # Configuration
    ├── copymode.rs        # Copy mode
    ├── history.rs         # Command history
    ├── core/
    │   ├── pty.rs         # ConPTY wrapper
    │   ├── session.rs     # Session management
    │   └── term/
    │       ├── state.rs   # Terminal state
    │       └── parser.rs  # VT parser
    ├── ui/
    │   ├── keymapper.rs   # Key mapping
    │   ├── renderer.rs    # Screen rendering
    │   └── wm_renderer.rs # Multi-pane rendering
    └── wm/
        ├── manager.rs     # Window manager
        ├── tab.rs         # Tab management
        ├── pane.rs        # Pane management
        └── layout.rs      # Layout calculation
```

## Troubleshooting

### Powerline / Nerd Font glyphs look wrong or misaligned

wtmux correctly renders prompts from oh-my-posh, Starship, and other Powerline-based
themes. If glyphs still appear broken, follow these steps:

**Step 1 — Install the full Nerd Font family**

Download Regular, Bold, Italic, and BoldItalic variants from [nerdfonts.com](https://www.nerdfonts.com/)
and install all four. Windows Terminal's font fallback will use a non-Nerd-Font bold face
if the Bold variant is missing, breaking PUA glyphs.

**Step 2 — Set the font in Windows Terminal**

```json
"fontFace": "CaskaydiaCove Nerd Font"
```

**Step 3 — If the problem persists, enable `suppress_bold`**

```toml
# %LOCALAPPDATA%\wtmux\config.toml
[font]
suppress_bold = true
```

This tells wtmux never to send SGR 1 (Bold) to the host terminal, keeping all text in
the Regular face that contains the PUA glyphs.

### Collecting a VT trace for bug reports

If a rendering problem is hard to reproduce, run wtmux with `--vt-trace` to record
every raw byte from the PTY:

```powershell
wtmux --vt-trace
```

The trace is written to `%LOCALAPPDATA%\wtmux\vt_trace.log` in hex + UTF-8 format.
Attach this file when filing a bug report.

## Known Limitations

- Windows only (ConPTY is Windows-specific)
- No detach/attach support yet (planned for future release)
- No session sharing yet

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Acknowledgments

- [tmux](https://github.com/tmux/tmux) - The inspiration for this project
- Windows ConPTY team for the pseudo-terminal API
- [crossterm](https://github.com/crossterm-rs/crossterm) - Cross-platform terminal manipulation
- [unicode-width](https://github.com/unicode-rs/unicode-width) - Unicode character width calculation
