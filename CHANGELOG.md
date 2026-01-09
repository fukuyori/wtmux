# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.0] - 2025-01-09

### Changed

- **Default encoding changed to UTF-8** - UTF-8 is now the default encoding instead of Shift-JIS
- Added `--sjis` option for Shift-JIS encoding when needed

### Added

- **Command History** - Record and reuse entered commands with `Ctrl+R`
  - Persistent storage in `~/.wtmux/history`
  - Shared across all panes
  - Automatic sensitive data exclusion
  - Maximum 1000 entries

## [0.1.0] - 2025-01-08

### Added

- Initial release
- **Window Manager**
  - Multiple tabs (windows) with creation, switching, and management
  - Pane splitting (horizontal and vertical)
  - Pane resizing with Ctrl+Arrow keys
  - Pane zoom toggle
  - Pane swapping
  - Pane number display and selection
  - Focus navigation between panes
  - 5 layout presets (even-horizontal, even-vertical, main-horizontal, main-vertical, tiled)
  - Window renaming

- **tmux-compatible Keybindings**
  - `Ctrl+B` prefix key
  - Window commands (c, &, n, p, l, 0-9, ,)
  - Pane commands (", %, x, o, ;, arrows, z, Space, q, {, })
  - Copy mode ([, /)

- **Copy Mode**
  - vim-like cursor navigation (h, j, k, l)
  - Page navigation (Ctrl+U, Ctrl+D, Ctrl+B, Ctrl+F)
  - Text selection and clipboard copy
  - Search with highlighting (/, ?, n, N)

- **Configuration**
  - TOML config file support (~/.wtmux/config.toml)
  - Shell selection (cmd, powershell, pwsh, wsl)
  - Encoding selection (UTF-8, Shift-JIS)

- **Color Schemes**
  - 8 built-in themes: default, solarized, monokai, nord, dracula, gruvbox, tokyo-night
  - Runtime theme switching with Ctrl+B, t

- **Terminal Emulation**
  - ConPTY backend for native Windows support
  - VT100/VT220 escape sequence parsing
  - Mouse support (selection, scrolling)
  - Scrollback buffer (10,000 lines)
  - Cursor shape control (block, underline, bar)

- **Shell Support**
  - cmd.exe
  - Windows PowerShell
  - PowerShell 7 (pwsh)
  - WSL

- **Installers**
  - PowerShell install script
  - Inno Setup installer
  - WiX MSI installer (v3.x and v6.0 compatible)

### Known Issues

- Detach/attach functionality not yet implemented
- Session sharing not yet implemented
- Some complex VT sequences may not be fully supported

## [Unreleased]

### Planned

- Detach/attach support
- Session sharing
- Scripting support
- Custom keybinding configuration
- Status bar customization
- Plugin system
