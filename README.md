# wtmux

A tmux-like terminal multiplexer for Windows, written in Rust.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Windows](https://img.shields.io/badge/platform-Windows-blue.svg)](https://www.microsoft.com/windows)
[![Version](https://img.shields.io/badge/version-0.4.0-green.svg)](https://github.com/user/wtmux/releases)

[日本語版 README](README.ja.md)

## Features

- **tmux-compatible keybindings** - Familiar `Ctrl+B` prefix commands
- **Multiple tabs (windows)** - Create, switch, rename, and manage tabs
- **Split panes** - Horizontal and vertical splits with resize support
- **Pane zoom** - Toggle full-screen for any pane (v0.4.0: seamless transitions)
- **Layout presets** - 5 layouts (even-horizontal, even-vertical, main-horizontal, main-vertical, tiled)
- **Copy mode** - vim-like scrollback navigation and text selection
- **Search** - Search through scrollback buffer with highlighting
- **Command history** - Record and reuse commands with Ctrl+R
- **Color schemes** - 8 built-in themes (default, solarized, monokai, nord, dracula, gruvbox, tokyo-night)
- **Configuration** - TOML config file support
- **ConPTY support** - Native Windows pseudo-terminal
- **Multiple shells** - cmd.exe, PowerShell, PowerShell 7, WSL
- **Encoding support** - UTF-8 and Shift-JIS (CP932)
- **Robust rendering** - Thread-safe output with synchronized updates (v0.4.0)

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

Download from the [Releases](https://github.com/user/wtmux/releases) page:

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
git clone https://github.com/user/wtmux.git
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
.\build-installer.ps1
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

All commands use `Ctrl+B` as the prefix key (same as tmux default).

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

| Key | Action |
|-----|--------|
| `Ctrl+R` | Show history search |
| `Enter` | Execute selected command (replace current input) |
| `Shift+Enter` | Append with `&&` (run if previous succeeds) |
| `Ctrl+Enter` | Append with `&` (background/parallel) |

For more details, see: https://qiita.com/spumoni/items/7d43ed7e579d99cfda3e

## Configuration

wtmux reads configuration from `~/.wtmux/config.toml`.

```toml
# General settings
[general]
default_shell = "powershell"  # cmd, powershell, pwsh, wsl
encoding = "utf8"             # utf8, sjis

# Appearance
[appearance]
color_scheme = "tokyo-night"  # default, solarized, monokai, nord, dracula, gruvbox, tokyo-night

# Cursor settings
[cursor]
shape = "block"               # block, underline, bar
blink = true
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
├── build-portable.ps1
├── build-installer.ps1
├── build-inno-installer.ps1
├── installer/
│   ├── wtmux.iss          # Inno Setup script
│   ├── wtmux.wxs          # WiX script
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
