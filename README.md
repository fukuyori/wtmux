# wtmux

A tmux-like terminal multiplexer for Windows, macOS, and Linux, written in Rust.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Platform](https://img.shields.io/badge/platform-Windows%20%7C%20macOS%20%7C%20Linux-blue.svg)](https://github.com/fukuyori/wtmux)
[![Version](https://img.shields.io/badge/version-3.1.0-green.svg)](https://github.com/fukuyori/wtmux/releases)

[日本語版 README](README.ja.md)

## 3.0.0 Highlights

- **Configurable key bindings** — every key in the prefix table can now be reassigned or removed from `config.toml` the way tmux's `bind-key` does, and prefix-less bindings (tmux's `bind-key -n`) are supported through `[bind_root]`. See [Custom Key Bindings](#custom-key-bindings).
- **`wtmux list-keys`** (alias `lsk`) — print the effective binding table, after config overrides, in the same syntax `[bind]` accepts.
- **Prefix bindings now respect modifiers**, so `Prefix, C-x` no longer triggers the binding for `x` unless nothing more specific matches. Holding Ctrl through the whole sequence (`C-b C-n`) still works.
- From 2.3.4: **more visible pane borders and titles** — pane titles are drawn in the same color as their border, and the unfocused border color was brightened in all eight built-in themes.
- From 2.3.0-2.3.3: **agent workflow tools** — the `Prefix + m` message composer (multi-line floating editor for sending messages to a pane, IME-friendly) and `wtmux agents`, a monitor CLI showing every pane's WORKING / BLOCKED / DONE / IDLE state with a Nerd Font spinner, also visible in the `Prefix + g` dashboard.

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
- **Cross-platform PTY backends** - ConPTY on Windows, POSIX pty (openpty) on macOS / Linux
- **Multiple shells** - cmd.exe, PowerShell, PowerShell 7, WSL on Windows; `$SHELL` (bash, zsh, fish, ...) on macOS / Linux
- **Encoding support** - UTF-8 and Shift-JIS (CP932, Windows only)
- **Robust rendering** - Thread-safe output with synchronized updates (v0.4.0)
- **Mouse passthrough** - TUI apps receive mouse events (hold Shift for wtmux selection)
- **Kitty keyboard protocol** - panes support the *disambiguate escape codes* and *report event types* enhancements (`CSI u`), so apps like neovim, helix or fish (in WSL/ssh) can distinguish Ctrl+I from Tab, Shift+Enter from Enter, and receive key-release events
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

- **Windows**: Windows 10 version 1809 or later (ConPTY support required)
- **macOS / Linux**: any modern terminal (iTerm2, Ghostty, WezTerm, kitty, GNOME Terminal, ...) — wtmux runs inside it via the standard POSIX pty
- Rust 1.70 or later (for building from source)

## Installation

### Option 1: Download Release

Download from the [Releases](https://github.com/fukuyori/wtmux/releases) page:

**Windows**

- **Installer** (`wtmux-x.x.x-setup.exe`) - Recommended for most users
- **Portable** (`wtmux-x.x.x-portable-x64.zip`) - No installation required, just extract and run
- **MSI** (`wtmux-x.x.x-x64.msi`) - For enterprise deployment

**macOS**

- **Installer package** (`wtmux-x.x.x.pkg`) - Signed & notarized; installs `/usr/local/bin/wtmux`

**Linux**

- **.deb / .rpm packages** - Build with `scripts/build-linux-packages.sh` (see below)
- Build from source (see Option 3)

### Option 2: PowerShell Install Script (Windows)

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

# Windows: copy to your preferred location
copy target\release\wtmux.exe C:\your\bin\path\

# macOS / Linux: copy to a directory on your PATH
cp target/release/wtmux /usr/local/bin/
```

### Building Installers (Windows)

```powershell
# Portable package (ZIP)
.\scripts\build-portable.ps1

# Using Inno Setup (recommended for end users)
# Download from: https://jrsoftware.org/isinfo.php
.\scripts\build-inno-installer.ps1

# Using WiX Toolset (for enterprise deployment)
# Download from: https://wixtoolset.org/releases/
# On WiX Toolset v7, the script accepts the OSMF EULA automatically before building.
.\scripts\build-installer.ps1

# MSIX package (for Windows 10/11)
# Requires Windows 10 SDK
.\scripts\build-msix.ps1              # Unsigned (requires Developer Mode)
.\scripts\build-msix.ps1 -Sign        # Self-signed (for testing)

# Regenerate icon assets after editing assets/wtmux-icon.svg
# The generated .ico is embedded into wtmux.exe and reused by the installers.
.\scripts\generate-icons.ps1
```

### Building the macOS Installer

`scripts/sign-and-notarize-macos.sh` builds a signed & notarized `.pkg` from
`target/release/wtmux` (requires Developer ID certificates and a notarytool
keychain profile):

```bash
cargo build --release
./scripts/sign-and-notarize-macos.sh
```

### Building the Linux Packages

`scripts/build-linux-packages.sh` builds `.deb` and `.rpm` packages from
`target/release/wtmux` using [`cargo-deb`](https://crates.io/crates/cargo-deb)
and [`cargo-generate-rpm`](https://crates.io/crates/cargo-generate-rpm)
(package metadata lives in `Cargo.toml`):

```bash
cargo install cargo-deb cargo-generate-rpm  # one-time setup
./scripts/build-linux-packages.sh           # builds both
./scripts/build-linux-packages.sh --deb     # .deb only
./scripts/build-linux-packages.sh --rpm     # .rpm only
```

Output goes to `installer/output/`.

## Usage

```bash
# Default: Multi-pane mode
wtmux

# With PowerShell 7 and UTF-8
wtmux -7 -u

# With WSL
wtmux -w

# Custom shell (e.g. zsh on macOS / Linux)
wtmux -s zsh

# Simple single-pane mode
wtmux -1

# Show help
wtmux --help
```

### Command Line Options

| Option | Description |
|--------|-------------|
| `-1, --simple` | Simple single-pane mode |
| `-c, --cmd` | Use Command Prompt (cmd.exe) *(Windows only)* |
| `-p, --powershell` | Use Windows PowerShell *(Windows only)* |
| `-7, --pwsh` | Use PowerShell 7 (pwsh.exe) *(Windows only)* |
| `-w, --wsl` | Use WSL *(Windows only)* |
| `-s, --shell <CMD>` | Custom shell command |
| `--sjis` | Shift-JIS encoding (default: UTF-8) *(Windows only)* |
| `-P, --cwd-prompt-hook <on\|off>` | Set shell prompt hook cwd tracking |
| `--no-cwd-prompt-hook` | Disable shell prompt hook cwd tracking |
| `-v, --version` | Show version |
| `-h, --help` | Show help |
| `list-keys` (`lsk`) | List the effective key bindings |

The Windows-only options are hidden on macOS / Linux. There, the default
shell is `$SHELL` (falling back to `/bin/sh`); use `-s` or the `shell`
config key to override it.

## Keybindings

Prefix commands use `Ctrl+B` by default (same as tmux), and the prefix key can be changed with `prefix_key`.
The tables below show the default keybindings; every one of them can be
reassigned or removed — see [Custom Key Bindings](#custom-key-bindings).

### Windows (Tabs)

| Key | Action |
|-----|--------|
| `Ctrl+B, c` | Create new window |
| Click `[+]` in the tab bar | Create new window |
| `Ctrl+B, &` | Kill current window |
| `Ctrl+B, n` | Next window |
| `Ctrl+B, p` | Previous window |
| `Ctrl+B, l` | Toggle last window |
| `Ctrl+B, w` | Show the window selector |
| `Ctrl+B, 0-9` | Select window by number |
| `Ctrl+B, ,` | Rename window |

The window selector shows every window with its pane count and tmux-style
flags (`*` current, `-` last), plus a live preview of the selected window.
Use `Up`/`Down` or `j`/`k` to move, `1`-`9` to jump to a window by number,
`Enter` to switch, `x` to kill the selected item (confirm with `y`), and
`Esc` or `q` to close the list. Windows expand into a tree: `Right`/`l`
lists a window's panes as child rows, `Left`/`h` folds them; selecting a
pane row previews it, and `Enter` switches to the window with that pane
focused. The mouse works too: scroll to move the selection, click a row to
switch to it, or click outside the popup to close it.

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
| `Ctrl+B, :` | Command prompt (tmux-style commands, see below) |
| `Ctrl+B, t` | Theme selector |
| `Esc` in theme selector | Cancel theme selector |
| `Ctrl+B, r` | Reset cursor shape |
| `Ctrl+B, Shift+P` | Toggle output logging for the focused pane (`[LOG]`) |
| `Ctrl+B, b` | Send Ctrl+B to application |
| `Esc` | Cancel prefix mode |

### Command Prompt

`Ctrl+B, :` opens a tmux-style command prompt on the status bar. Supported
commands (tmux abbreviations in parentheses):

| Command | Action |
|---------|--------|
| `split-window [-h]` (`splitw`) | Split pane; `-h` = left/right |
| `new-window` (`neww`) | Create window |
| `kill-pane` (`killp`) / `kill-window` (`killw`) | Kill pane / window |
| `next-window` / `previous-window` / `last-window` (`next` / `prev` / `last`) | Switch window |
| `select-window -t <n>` (`selectw`) | Select window by number |
| `rename-window <name>` (`renamew`) | Rename window |
| `select-layout <even-horizontal\|even-vertical\|main-horizontal\|main-vertical\|tiled>` (`selectl`) | Apply layout preset |
| `resize-pane -Z` | Toggle pane zoom |
| `set synchronize-panes [on\|off]` | Input broadcast |
| `pipe-pane` | Toggle pane output logging |
| `display-popup [command]` (`popup`) | Open a floating popup pane |

Results and errors appear as a transient message on the status bar.

### Popup (display-popup)

`:display-popup [command]` — or `wtmux display-popup [command...]` from any
shell — opens a centered floating pane (60% of the terminal) running the
command, or your default shell. All input goes to the popup; it closes when
the command exits. `Ctrl+B, x` force-closes a stuck popup. Note: the command
is spawned directly, so shell built-ins need an explicit shell
(e.g. `display-popup cmd /c dir` on Windows, `display-popup sh -c "ls | head"`
on macOS / Linux).

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

### bash / zsh (macOS / Linux / WSL)

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

wtmux reads configuration from `config.toml` in its config directory:

| OS | Location |
|----|----------|
| Windows | `%LOCALAPPDATA%\wtmux\config.toml` (e.g. `C:\Users\you\AppData\Local\wtmux\config.toml`) |
| macOS / Linux | `$XDG_CONFIG_HOME/wtmux/config.toml`, or `~/.config/wtmux/config.toml` when `XDG_CONFIG_HOME` is unset |

If neither location can be determined, `~/.wtmux/config.toml` is used as a
fallback. Command history, pane output logs, and VT traces are stored in the
same directory.

```toml
# Default shell (optional)
# Windows: "cmd", "powershell", "pwsh", "wsl", or full path
# macOS / Linux: command name or full path (default: $SHELL, then /bin/sh)
# shell = "pwsh.exe"

# Codepage for encoding (optional, Windows only)
# codepage = 65001  # UTF-8
# codepage = 932    # Shift-JIS

# Prefix key (default: "C-b" for Ctrl+B)
# prefix_key = "C-a"  # Change to Ctrl+A

# Inject a prompt hook into cmd.exe / PowerShell to publish pane cwd changes.
# Disabled by default to avoid interfering with custom prompts.
# cwd_prompt_hook = false

# Remove built-in bindings (tmux: unbind-key).
# Being an array, this must appear before the [sections] below.
# unbind = ["d", "P"]

# Color scheme
# Available: default, solarized-dark, solarized-light, monokai, nord, dracula, gruvbox-dark, tokyo-night
color_scheme = "tokyo-night"

# Legacy global keybindings (prefer [bind_root] below)
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

# Bindings pressed after the prefix key (tmux: bind-key)
[bind]
# "M-4" = "select-layout main-vertical"
# "M-5" = "select-layout tiled"
# "C-o" = "swap-pane -D"
# "z"   = ""                    # empty string unbinds

# Bindings pressed without the prefix (tmux: bind-key -n) — the recommended
# place for scrollback / selection / copy keys
[bind_root]
# "S-PageUp" = "scroll-up"
# "C-S-c"    = "copy-selection"
# "C-M-t"    = "select-layout tiled"

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

# Agent state hooks (see "AI Agent Integration" below)
[hooks]
# on_agent_blocked = "powershell -NoProfile -Command \"...notify...\""
# on_agent_done = ""
```

The `[keybindings]` section is the legacy way to remap these non-prefix
shortcuts: the history selector (`Ctrl+R` by default), scrollback navigation,
keyboard selection, and copy-selection. It keeps working, but the same
functions are also available as bindable commands (`scroll-up`,
`extend-selection`, `copy-selection`, `history-selector`, ...) — prefer
binding them in `[bind_root]`, which allows any key, supports unbinding, and
takes precedence over `[keybindings]`.

### Custom Key Bindings

`[bind]`, `[bind_root]` and `unbind` assign commands to keys the way tmux's
`bind-key` does. Every built-in binding can be overridden or removed.

```toml
unbind = ["d"]                 # drop the default Ctrl+B, d (array goes before any [section])

[bind]                         # pressed after the prefix key
"M-1" = "select-layout even-horizontal"
"M-4" = "select-layout main-vertical"
"M-5" = "select-layout tiled"
"|"   = "split-window -h"
"C-o" = "swap-pane -D"
"z"   = ""                     # empty string unbinds, same as `unbind`

[bind_root]                    # pressed without the prefix
"C-M-Left"  = "select-pane -L"
"C-M-Right" = "select-pane -R"
```

**Key names**: a single character (`c`, `%`, `4`), or one of `Space`, `Enter`,
`Esc`, `Tab`, `Backspace`, `Delete`, `Insert`, `Up`, `Down`, `Left`, `Right`,
`Home`, `End`, `PageUp`, `PageDown`, `F1`-`F12`. Prefix with `C-` (Ctrl),
`M-` (Alt) or `S-` (Shift); the `Ctrl+` / `Alt+` / `Shift+` spellings also work.
Character case is significant, so `P` and `p` are different keys.

**Commands**:

| Group | Commands |
|-------|----------|
| Windows | `new-window` / `kill-window` / `next-window` / `previous-window` / `last-window` / `select-window -t <n>` / `rename-window` / `choose-window` |
| Panes | `split-window [-h]` / `kill-pane` / `next-pane` / `previous-pane` / `select-pane -L\|-R\|-U\|-D` / `swap-pane -U\|-D` / `display-panes` |
| Sizing | `resize-pane -Z` (zoom) / `resize-pane -L\|-R\|-U\|-D` / `resize-pane +` / `resize-pane -` |
| Layout | `next-layout` / `select-layout <even-horizontal\|even-vertical\|main-horizontal\|main-vertical\|tiled>` |
| Modes | `copy-mode` / `search` / `command-prompt` / `choose-theme` / `agent-dashboard` / `compose-message` |
| Terminal | `scroll-up [n]` / `scroll-down [n]` / `scroll-top` / `scroll-bottom` / `extend-selection -L\|-R\|-U\|-D` / `copy-selection` / `history-selector` |
| Other | `set synchronize-panes` / `pipe-pane` / `paste-buffer` / `send-prefix` / `detach-client` / `next-attention` / `reset-cursor` / `none` |

**Notes**:

- `[bind_root]` keys are intercepted before reaching the shell and take precedence over `[keybindings]`
- A malformed entry is skipped on its own, with the reason printed to stderr at startup
- `wtmux list-keys` (alias `lsk`) prints the effective table in the same syntax `[bind]` accepts

```
$ wtmux list-keys
bind      Space        next-layout
bind      Alt+4        select-layout main-vertical
bind      c            new-window
bind_root Ctrl+Alt+Left select-pane -L
```

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

## AI Agent Integration

wtmux monitors every pane and classifies it herdr-style as WORKING / BLOCKED /
DONE / IDLE (`Prefix + g` opens the agent dashboard; WORKING panes animate a
Nerd Font circle-slice spinner there). `wtmux agents` prints the same list
from any pane and refreshes four times a second (Ctrl+C quits; `--once` for a
single snapshot) — run it in a spare pane or a `display-popup` for an
always-on monitor. Three more features build on this for running AI coding
agents in panes:

### Agent state hooks

Run a command whenever a pane's state changes — e.g. raise a Windows toast the
moment a background agent blocks on a permission prompt:

```toml
# Windows: %LOCALAPPDATA%\wtmux\config.toml
# macOS / Linux: ~/.config/wtmux/config.toml
[hooks]
on_agent_blocked = 'powershell -NoProfile -Command "New-BurntToastNotification -Text \"wtmux\", \"$env:WTMUX_HOOK_TITLE is waiting for input\""'
# macOS notification instead:
# on_agent_blocked = 'osascript -e "display notification \"$WTMUX_HOOK_TITLE is waiting for input\" with title \"wtmux\""'
# Linux notification instead:
# on_agent_blocked = 'notify-send wtmux "$WTMUX_HOOK_TITLE is waiting for input"'
# on_agent_working / on_agent_done / on_agent_idle are also available
```

Hooks run detached (`cmd /C` on Windows, `sh -c` elsewhere) and receive the
context via environment variables: `WTMUX_HOOK_STATE`, `WTMUX_HOOK_PREV_STATE`,
`WTMUX_HOOK_PANE` (`<window>.<pane>`), `WTMUX_HOOK_WINDOW`, `WTMUX_HOOK_TITLE`.

### Ground-truth state reporting (`wtmux report-state`)

By default pane states are inferred from output heuristics. Tools running
inside a pane can instead report the state directly:

```bash
wtmux report-state blocked     # the calling pane, via WTMUX_PID / WTMUX_PANE
wtmux report-state -t 1.2 done # explicit <window>.<pane> target
```

This pairs naturally with agent CLIs that have their own hook systems, and is
the recommended setup for Claude Code: its waiting-for-input UI keeps
redrawing (spinner, status line), so the output-quiet heuristics tend to
leave the pane stuck on WORKING. With hooks the pane flips to BLOCKED the
moment Claude Code asks for permission or input. Add to
`~/.claude/settings.json`:

```json
{
  "hooks": {
    "UserPromptSubmit": [{ "hooks": [{ "type": "command", "command": "wtmux report-state working 2>/dev/null || true" }] }],
    "Notification":     [{ "hooks": [{ "type": "command", "command": "wtmux report-state blocked 2>/dev/null || true" }] }],
    "Stop":             [{ "hooks": [{ "type": "command", "command": "wtmux report-state done 2>/dev/null || true" }] }]
  }
}
```

`UserPromptSubmit` marks the pane WORKING when a prompt is sent,
`Notification` marks it BLOCKED (waiting for permission / input), and `Stop`
marks it DONE. The `2>/dev/null || true` keeps the hooks silent when Claude
Code runs outside wtmux.

Reported states override the heuristics (until new output arrives), update the
dashboard / status bar / attention flags, and fire `[hooks]` commands.

### Scripting a running instance (`send-keys` / `capture-pane`)

External tools — orchestrators, scripts, or another AI agent — can drive a
running wtmux:

```bash
# Type a command into pane 2 of window 1 and run it
wtmux send-keys -t 1.2 "cargo test" Enter

# Read back what that pane shows (visible screen; -S - adds full scrollback)
wtmux capture-pane -p -t 1.2
wtmux capture-pane -p -t 1.2 -S -

# Open a popup in the running instance
wtmux display-popup "cmd /c dir"
```

`send-keys` understands tmux key names (`Enter`, `Escape`, `Tab`, `Space`,
`BSpace`, `Up`/`Down`/`Left`/`Right`, `Home`, `End`, `PageUp`, `PageDown`,
`C-x`, `M-x`); anything else is sent literally. Without `-t`, the calling
pane (via `WTMUX_PANE`) or the focused pane is targeted. With a single wtmux
running, the instance is found automatically; otherwise pass `--pid <pid>`
(see `wtmux list-clients`). This pairs with `report-state` to build
claude-squad-style agent orchestration on top of wtmux.

### Pane output logging (tmux `pipe-pane` analog)

`Prefix + Shift+P` toggles logging of the focused pane's raw output stream to
`logs/wtmux-<pid>-<window>.<pane>-<epoch>.log` under the config directory
(`%LOCALAPPDATA%\wtmux` on Windows, `~/.config/wtmux` on macOS / Linux). The
status bar shows `[LOG]` while logging is active. Useful for auditing or
replaying an agent's session; the log contains the exact bytes including
escape sequences (strip them with e.g. `sed -r 's/\x1b\[[0-9;]*[a-zA-Z]//g'`).

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
# bash / zsh (macOS / Linux / WSL)
[ -n "$WTMUX" ] && echo "Running in wtmux"
```

| Variable | Meaning |
|----------|---------|
| `WTMUX` | `1` when running inside wtmux |
| `WTMUX_VERSION` | wtmux version |
| `WTMUX_PID` | Process id of the wtmux instance (target for `wtmux report-state`) |
| `WTMUX_PANE` | `<window>.<pane>` id of the pane the process runs in |

## Mouse Support

wtmux provides comprehensive mouse support:

### Text Selection and Copy

You can select text with the mouse and copy it to the clipboard:

1. **Click and drag** to select text
2. **Release the mouse button** - selected text is automatically copied to clipboard
3. **Paste** with `Ctrl+V` or **right-click → Paste** from the context menu

This works the same as standard terminal text selection.

### Split Resize

Drag a pane split border with the left mouse button to resize adjacent panes.
Boundary dragging is handled by wtmux even when the focused TUI application has
enabled mouse tracking.

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
| Left drag on split border | Resize panes | Resize panes |
| Left click on tab bar | Switch tab | Switch tab |
| Left click `[+]` in tab bar | Create new tab | Create new tab |
| Right click | Context menu (Paste, Zoom, Split, Rename Pane, etc.) | Context menu |
| Right click on tab bar | Rename that window | Rename that window |
| Right click on pane title (top border) | Rename that pane | Rename that pane |
| Scroll wheel | Scroll buffer | App receives event |

## Comparison with tmux

| Feature | tmux | wtmux |
|---------|------|-------|
| Platform | Unix/Linux/macOS | Windows / macOS / Linux |
| Backend | PTY | ConPTY (Windows) / POSIX pty (macOS, Linux) |
| Windows/Panes | ✓ | ✓ |
| Keybindings | ✓ | ✓ (compatible) |
| Configurable bindings | ✓ (`bind-key`) | ✓ (`[bind]` / `[bind_root]`) |
| Copy mode | ✓ | ✓ |
| Search | ✓ | ✓ |
| Layout presets | ✓ | ✓ |
| Config file | ✓ | ✓ |
| Color schemes | ✓ | ✓ |
| Mouse support | ✓ | ✓ |
| Detach/Attach | ✓ | Planned |
| Session sharing | ✓ | Planned |
| Scripting | ✓ | ✓ (`send-keys` / `capture-pane`) |

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
├── assets/
│   ├── wtmux-icon.svg       # Icon source artwork
│   └── generated/           # Generated .ico / preview PNG
├── installer/
│   ├── wtmux.iss          # Inno Setup script
│   ├── wtmux.wxs          # WiX script
│   ├── msix/Assets/       # MSIX icon assets
│   └── license.rtf
├── scripts/
│   ├── build-portable.ps1
│   ├── build-installer.ps1
│   ├── build-inno-installer.ps1
│   ├── build-msix.ps1
│   ├── generate-icons.ps1
│   ├── sign-and-notarize-macos.sh  # macOS signed/notarized .pkg build
│   └── build-linux-packages.sh     # Linux .deb / .rpm build
└── src/
    ├── main.rs            # Entry point
    ├── config.rs          # Configuration
    ├── copymode.rs        # Copy mode
    ├── history.rs         # Command history
    ├── core/
    │   ├── pty/
    │   │   ├── conpty.rs  # Windows ConPTY backend
    │   │   └── unix.rs    # POSIX pty backend (macOS / Linux)
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

**Step 2 — Set the font in your host terminal**

Windows Terminal example (macOS / Linux: set it in iTerm2, Ghostty, etc.):

```json
"fontFace": "CaskaydiaCove Nerd Font"
```

**Step 3 — If the problem persists, enable `suppress_bold`**

```toml
# Windows: %LOCALAPPDATA%\wtmux\config.toml
# macOS / Linux: ~/.config/wtmux/config.toml
[font]
suppress_bold = true
```

This tells wtmux never to send SGR 1 (Bold) to the host terminal, keeping all text in
the Regular face that contains the PUA glyphs.

### Collecting a VT trace for bug reports

If a rendering problem is hard to reproduce, run wtmux with `--vt-trace` to record
every raw byte from the PTY:

```bash
wtmux --vt-trace
```

The trace is written to `vt_trace.log` in the config directory
(`%LOCALAPPDATA%\wtmux` on Windows, `~/.config/wtmux` on macOS / Linux)
in hex + UTF-8 format. Attach this file when filing a bug report.

## Known Limitations

- Shell shortcuts (`-c`/`-p`/`-7`/`-w`) and Shift-JIS encoding are Windows-only
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
