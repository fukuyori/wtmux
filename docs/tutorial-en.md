# wtmux: A tmux-like Terminal Multiplexer for Windows

## Introduction

For developers who work in Linux or macOS terminals, **tmux** is an indispensable tool. It allows you to manage multiple terminal sessions in a single window, split panes, and preserve sessions. Unfortunately, tmux doesn't run natively on Windows.

**wtmux** is a terminal multiplexer developed to bring the tmux experience to Windows. Written in Rust, it leverages Windows 10's ConPTY (Console Pseudo Terminal) to provide a native Windows experience.

### Key Features

- **tmux-compatible keybindings** - Instantly familiar for tmux users
- **Multiple tabs & pane splitting** - Organize your work efficiently
- **Copy mode** - vim-like scrollback navigation
- **Search** - Quickly find text in output history
- **Command history** - Record and reuse entered commands
- **Color schemes** - 8 built-in themes
- **Multiple shell support** - cmd, PowerShell, PowerShell 7, WSL

---

## Installation

### Option 1: Portable Version (Recommended)

1. Download `wtmux-x.x.x-portable-x64.zip` from the [Releases](https://github.com/user/wtmux/releases) page
2. Extract to any folder
3. Run `wtmux.exe`

### Option 2: Installer

1. Download `wtmux-x.x.x-setup.exe` from the [Releases](https://github.com/user/wtmux/releases) page
2. Run the installer
3. Run `wtmux` from Command Prompt or PowerShell

### Option 3: Build from Source

```powershell
git clone https://github.com/user/wtmux.git
cd wtmux
cargo build --release
.\target\release\wtmux.exe
```

---

## Quick Start

### Launch

```powershell
# Default (cmd.exe)
wtmux

# With PowerShell 7
wtmux -7

# With WSL
wtmux -w

# With UTF-8 encoding
wtmux -u
```

### Basic Concepts

wtmux has three hierarchical levels:

```
wtmux
├── Tab (Window) 1
│   ├── Pane 1
│   └── Pane 2
├── Tab (Window) 2
│   └── Pane 1
└── Tab (Window) 3
    ├── Pane 1
    ├── Pane 2
    └── Pane 3
```

- **Tab (Window)**: Independent workspace shown in the tab bar at the bottom
- **Pane**: Individual terminal area within a tab

---

## Basic Operations

### Prefix Key

All wtmux commands start with **`Ctrl+B`** (same as tmux).

For example, to create a new tab:
1. Press `Ctrl+B` (you'll see `[PREFIX]` at the bottom)
2. Press `c`

### Tab Operations

| Key | Action |
|-----|--------|
| `Ctrl+B, c` | Create new tab |
| `Ctrl+B, n` | Next tab |
| `Ctrl+B, p` | Previous tab |
| `Ctrl+B, 0-9` | Select tab by number |
| `Ctrl+B, ,` | Rename tab |
| `Ctrl+B, &` | Close current tab |
| `Ctrl+B, l` | Toggle to last used tab |

### Pane Operations

| Key | Action |
|-----|--------|
| `Ctrl+B, "` | Split horizontally (top/bottom) |
| `Ctrl+B, %` | Split vertically (left/right) |
| `Ctrl+B, ←↑↓→` | Move to pane in arrow direction |
| `Ctrl+B, o` | Next pane |
| `Ctrl+B, x` | Close current pane |
| `Ctrl+B, z` | Zoom pane (fullscreen toggle) |

### Pane Resizing

| Key | Action |
|-----|--------|
| `Ctrl+B, Ctrl+←` | Expand left |
| `Ctrl+B, Ctrl+→` | Expand right |
| `Ctrl+B, Ctrl+↑` | Expand up |
| `Ctrl+B, Ctrl+↓` | Expand down |

---

## Tutorial: Building a Development Environment

Let's build a practical web development workspace.

### Step 1: Launch wtmux

```powershell
wtmux -7 -u  # PowerShell 7 + UTF-8
```

### Step 2: Split Panes

1. Press `Ctrl+B, %` to split vertically (left/right)
2. In the right pane, press `Ctrl+B, "` to split horizontally (top/bottom)

You now have a 3-pane layout:

```
┌─────────────────────┬─────────────────────┐
│                     │                     │
│   Editor            │   Server logs       │
│                     │                     │
│                     ├─────────────────────┤
│                     │                     │
│                     │   Git/Tests         │
│                     │                     │
└─────────────────────┴─────────────────────┘
```

### Step 3: Work in Each Pane

**Left pane (Editor):**
```powershell
cd your-project
code .  # Launch VS Code
```

**Top-right pane (Server):**
```powershell
cd your-project
npm run dev  # Start dev server
```

**Bottom-right pane (Git/Tests):**
```powershell
cd your-project
git status
```

### Step 4: Navigate Between Panes

- `Ctrl+B, ←` to go left
- `Ctrl+B, →` to go right
- `Ctrl+B, ↑` to go up
- `Ctrl+B, ↓` to go down

### Step 5: Use Zoom

When you need to see server logs in detail:
1. Focus on the server pane
2. Press `Ctrl+B, z` to zoom (fullscreen)
3. Press `Ctrl+B, z` again to return to normal view

---

## Layout Presets

Press `Ctrl+B, Space` to cycle through 5 layout presets:

### 1. even-horizontal
```
┌────────┬────────┬────────┐
│        │        │        │
│   1    │   2    │   3    │
│        │        │        │
└────────┴────────┴────────┘
```

### 2. even-vertical
```
┌──────────────────────────┐
│            1             │
├──────────────────────────┤
│            2             │
├──────────────────────────┤
│            3             │
└──────────────────────────┘
```

### 3. main-horizontal
```
┌──────────────────────────┐
│            1             │
├────────┬────────┬────────┤
│   2    │   3    │   4    │
└────────┴────────┴────────┘
```

### 4. main-vertical
```
┌────────────────┬─────────┐
│                │    2    │
│       1        ├─────────┤
│                │    3    │
│                ├─────────┤
│                │    4    │
└────────────────┴─────────┘
```

### 5. tiled
```
┌────────────┬─────────────┐
│     1      │      2      │
├────────────┼─────────────┤
│     3      │      4      │
└────────────┴─────────────┘
```

---

## Copy Mode

Navigate through terminal output history and copy text.

### Enter Copy Mode

| Key | Action |
|-----|--------|
| `Ctrl+B, [` | Enter copy mode |
| `Ctrl+B, /` | Enter with search |

### Copy Mode Navigation

**Cursor movement (vim-like):**

| Key | Action |
|-----|--------|
| `h` / `←` | Left |
| `j` / `↓` | Down |
| `k` / `↑` | Up |
| `l` / `→` | Right |
| `0` | Line start |
| `$` | Line end |
| `g` | Buffer top |
| `G` | Buffer bottom |

**Page scrolling:**

| Key | Action |
|-----|--------|
| `Ctrl+U` | Half page up |
| `Ctrl+D` | Half page down |
| `Ctrl+B` | Full page up |
| `Ctrl+F` | Full page down |
| `PageUp` | Full page up |
| `PageDown` | Full page down |

**Selection and copy:**

| Key | Action |
|-----|--------|
| `Space` / `v` | Start/toggle selection |
| `Enter` / `y` | Copy selection and exit |

**Search:**

| Key | Action |
|-----|--------|
| `/` | Search forward |
| `?` | Search backward |
| `n` | Next match |
| `N` | Previous match |

**Exit:**

| Key | Action |
|-----|--------|
| `q` / `Esc` | Exit copy mode |

### Practical Example

Finding a specific error in long logs:

1. Press `Ctrl+B, /` to enter search mode
2. Type `error` and press `Enter`
3. Press `n` to jump to next match
4. Press `Space` to start selection
5. Use `j` and `l` to extend selection
6. Press `Enter` to copy to clipboard

---

## Color Schemes

wtmux includes 8 built-in color schemes.

### Change at Runtime

1. Press `Ctrl+B, t` to open theme selector
2. Use `↑`/`↓` to browse themes
3. Press `Enter` to apply

### Available Themes

| Theme | Description |
|-------|-------------|
| `default` | Default colors |
| `solarized` | Solarized Dark |
| `monokai` | Monokai Pro |
| `nord` | Nord |
| `dracula` | Dracula |
| `gruvbox` | Gruvbox Dark |
| `tokyo-night` | Tokyo Night |

### Set in Config File

Create `%LOCALAPPDATA%\wtmux\config.toml`:

```toml
color_scheme = "tokyo-night"
```

---

## Command History

wtmux includes its own command history feature, separate from your shell's built-in history. It records the commands you enter, eliminating the need to retype complex commands repeatedly.

| Key | Action |
|-----|--------|
| `Ctrl+R` | Show history search |

For more details, see: https://qiita.com/spumoni/items/7d43ed7e579d99cfda3e

---

## Configuration File

wtmux reads settings from `%LOCALAPPDATA%\wtmux\config.toml`.

### Example Configuration

```toml
# Shell
shell = "pwsh.exe"

# Encoding (65001 = UTF-8, 932 = Shift-JIS)
codepage = 65001

# Color scheme
color_scheme = "tokyo-night"

# Tab bar
[tab_bar]
visible = true

# Status bar
[status_bar]
visible = true
show_time = true

# Pane
[pane]
border_style = "single"  # single, double, rounded, none

# Cursor
[cursor]
shape = "block"  # block, underline, bar
blink = true
```

---

## Tips & Tricks

### 1. Detect wtmux from Shell

Check if running inside wtmux:

**PowerShell:**
```powershell
if ($env:WTMUX) {
    Write-Host "Running in wtmux"
}
```

**cmd.exe:**
```batch
if defined WTMUX echo Running in wtmux
```

**bash (WSL):**
```bash
[ -n "$WTMUX" ] && echo "Running in wtmux"
```

### 2. Quick Pane Selection

When you have many panes:

1. Press `Ctrl+B, q` to show pane numbers
2. Press a number key (0-9) within 2 seconds
3. Focus moves to that pane

### 3. Swap Panes

| Key | Action |
|-----|--------|
| `Ctrl+B, {` | Swap with previous pane |
| `Ctrl+B, }` | Swap with next pane |

### 4. Send Prefix Key to Application

For applications that use `Ctrl+B` (e.g., Emacs):

Press `Ctrl+B, b` to send `Ctrl+B` to the application

### 5. Organize with Named Tabs

Create tabs for different tasks and name them:

1. Press `Ctrl+B, c` to create new tab
2. Press `Ctrl+B, ,` to rename (e.g., "frontend", "backend", "logs")
3. Use `Ctrl+B, 0`-`9` to switch quickly

---

## tmux User Reference

| tmux | wtmux | Note |
|------|-------|------|
| `Ctrl+B, c` | `Ctrl+B, c` | New window |
| `Ctrl+B, "` | `Ctrl+B, "` | Horizontal split |
| `Ctrl+B, %` | `Ctrl+B, %` | Vertical split |
| `Ctrl+B, x` | `Ctrl+B, x` | Close pane |
| `Ctrl+B, z` | `Ctrl+B, z` | Zoom |
| `Ctrl+B, [` | `Ctrl+B, [` | Copy mode |
| `Ctrl+B, Space` | `Ctrl+B, Space` | Next layout |
| `Ctrl+B, d` | - | Detach (not yet) |
| `tmux attach` | - | Attach (not yet) |

---

## Troubleshooting

### Garbled Characters

Launch with UTF-8 mode:

```powershell
wtmux -u
```

Or set in config:

```toml
codepage = 65001
```

### Keys Not Responding

Check if you're in prefix mode (`Ctrl+B`). You'll see `[PREFIX]` at the bottom.

Press `Esc` to cancel prefix mode.

### Display Issues

Resize the terminal window to force a redraw. Or run `clear` in the affected pane.

---

## Summary

With wtmux, you can enjoy tmux-like terminal workflows on Windows.

**Essential commands to get started:**

| Action | Key |
|--------|-----|
| New tab | `Ctrl+B, c` |
| Vertical split | `Ctrl+B, %` |
| Horizontal split | `Ctrl+B, "` |
| Navigate panes | `Ctrl+B, arrow` |
| Zoom pane | `Ctrl+B, z` |
| Copy mode | `Ctrl+B, [` |
| Search | `Ctrl+B, /` |
| Change theme | `Ctrl+B, t` |

Start using wtmux in your daily development workflow today!

---

## Links

- **GitHub**: https://github.com/user/wtmux
- **Releases**: https://github.com/user/wtmux/releases
- **Issues**: https://github.com/user/wtmux/issues
