# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.2] - 2025-01-21

### Added

- **Mouse event passthrough to child applications**
  - TUI applications that enable mouse capture now receive mouse events
  - Automatically detects when child apps request mouse tracking (DECSET 1000/1002/1003)
  - Supports SGR extended mouse mode (1006) for large terminals
  - Supports URXVT mouse mode (1015)
  - Hold Shift to bypass passthrough and use wtmux's text selection
  - Works with applications like htop, mc, vim with mouse support, and custom TUI apps

- **Paste from context menu**
  - Right-click context menu now includes "Paste" option
  - Paste clipboard content directly to the focused pane
  - Supports bracketed paste mode for compatible applications

## [1.0.0] - 2025-01-18

### Added

- **Tab bar mouse click support**
  - Click on tabs in the tab bar to switch between windows
  - Intuitive window navigation with mouse

- **Context menu (right-click menu)**
  - Right-click on any pane to show context menu
  - Menu items: Zoom/Unzoom, Split ─, Split │, Kill Pane, Cancel
  - Keyboard navigation: ↑/↓ or j/k to navigate, Enter to select, Esc to close
  - Mouse hover to highlight menu items
  - Useful when a pane becomes unresponsive

- **Comprehensive documentation**
  - Added documentation comments to all public APIs
  - Module-level documentation for all components

### Fixed

- Fixed Split ─ and Split │ direction mapping
- Fixed context menu flickering on mouse hover

## [0.4.0] - 2025-01-11

### Changed

- **Major rendering architecture refactoring**
  - Unified frame management with `with_frame()` wrapper
  - All render functions now use `stdout.lock()` for thread safety
  - Consistent begin/end frame handling across all rendering paths
  - Terminal state (cursor, autowrap, synchronized update) always restored on error

- **Layout management overhaul**
  - `reflow()` is now the single entry point for all geometry changes
  - `apply_geometry()` ensures consistent order: border → position → resize
  - Generation-based full redraw detection (replaces boolean flag)
  - Removed double-reflow bugs in `cleanup_dead_panes()`

### Fixed

- Fixed zoom causing black screen
  - Zoom now preserves terminal content instead of clearing it
  - Zoom/unzoom transitions are seamless

- Fixed potential cursor disappearing after render errors
  - `with_cursor_hidden()` ensures Show on all exit paths

- Fixed synchronized update boundary issues with BufWriter
  - Begin/end sequences now written to same buffer

- Fixed autowrap state leaking between render frames

### Removed

- Removed unused `resize_and_clear()` methods
- Removed unused `send_clear_screen()` methods
- Removed redundant synchronized update ON from `init()`

### Internal

- Added `with_frame()` for RAII-like frame management
- Added `with_cursor_hidden()` for lightweight cursor-only updates
- Improved error logging with PaneId and size information
- Cleaner separation between full renders and partial updates

## [0.3.4] - 2025-01-11

### Fixed

- Fixed wide character (CJK) rendering issues
  - Japanese text no longer truncated or displayed incorrectly
  - Fixed mismatch between unicode-width calculation and Windows Terminal rendering
  - Renderer now properly handles character width differences

- Fixed progress bar artifacts (backslash characters appearing on screen)
  - This was a bug since v0.1.0
  - Properly parse OSC sequence terminator (ESC \)
  - Cargo build progress and other progress indicators now display correctly

- Fixed carriage return not marking line as dirty for redraw

## [0.3.2] - 2025-01-09

### Added

- Added `-c, --cmd` option to explicitly use Command Prompt
  - Useful when config.toml specifies a different default shell

### Fixed

- Fixed config.toml shell setting not being applied
  - Shell setting from config file now properly merged with command line args
  - Priority: command line > config.toml > default (cmd.exe)

## [0.3.1] - 2025-01-09

### Fixed

- Fixed double shell startup for PowerShell/pwsh/WSL when using UTF-8 encoding
  - PowerShell and pwsh now launch directly with UTF-8 encoding
  - WSL now launches directly without cmd.exe wrapper

## [0.3.0] - 2025-01-09

### Changed

- **Default encoding changed to UTF-8** - UTF-8 is now the default encoding instead of Shift-JIS
- Added `--sjis` option for Shift-JIS encoding when needed

### Added

- **Command History** - Record and reuse entered commands with `Ctrl+R`
  - Persistent storage in `~/.wtmux/history`
  - Shared across all panes
  - Automatic sensitive data exclusion
  - Maximum 1000 entries
  - `Shift+Enter` to append with `&&` (conditional execution)
  - `Ctrl+Enter` to append with `&` (background/parallel)

- **Cursor Shape Reset** - Fix cursor shape issues with vim and other applications
  - Manual reset with `Ctrl+B, r`
  - Auto reset on pane switch (keyboard and mouse)

### Fixed

- Fixed double cmd.exe startup when using default UTF-8 encoding

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
