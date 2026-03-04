# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.2.4] - 2026-03-04

### Fixed

- **履歴ウィンドウを Esc で閉じても残像が残る問題を修正**:
  v1.2.0 のダーティライン最適化により、PTY 出力がない状態でセレクターを
  閉じると、ペイン内容が再描画されずオーバーレイの残像が残っていた。
  セレクター / コンテキストメニューを閉じる際に `force_full_redraw()` を
  呼ぶことで、次の render で全ペインを強制再描画するよう修正。
  Del で履歴削除後の Esc、マウスクリックによる閉じる操作も同様に修正。

## [1.2.3] - 2026-03-04

### Added

- **履歴ウィンドウで Del キーによる削除**: コマンド履歴セレクター（Ctrl+R）で
  選択中のエントリを Del キーで削除できるようになった。削除後はリストが即座に
  更新され、カーソル位置は維持される（最終行を削除した場合は一つ上に移動）。
  削除はファイルにも即座に反映される。
- ヒントバー表示を更新: `Enter:Run Del:Delete S-Enter:&& Esc:Close`

## [1.2.2] - 2026-03-04

### Added

- **Shell integration (OSC 133 / OSC 633)** for accurate command history:
  - VT parser now recognises OSC 133 and OSC 633 (VS Code extension) markers
    emitted by PowerShell, bash, zsh, fish, oh-my-posh, and Starship.
  - Marker B records the exact cursor column where user input begins, making
    history capture independent of prompt appearance (no more `strip_prompt`
    regex for modern shells).
  - Marker C captures the confirmed command text at the moment Enter is pressed.
  - Marker D records the exit code of the last command.
  - `ShellIntegration` struct added to `TerminalState`.

- **Keystroke tracker** (`KeystrokeTracker`) as a fallback for `cmd.exe`:
  - Intercepts every printable key before forwarding to the PTY.
  - Handles Backspace, Ctrl+W (delete word), Ctrl+U / Ctrl+C (clear line).
  - Active only when no OSC markers have been seen for the current pane.

- **Tiered `get_current_line()`** in `WindowManager`:
  - Priority 1: OSC confirmed command (marker C).
  - Priority 2: OSC prompt-end position (marker B) + screen buffer slice.
  - Priority 3: Keystroke tracker buffer.
  - Priority 4: Legacy `strip_prompt` heuristic (last resort).

- **README**: new *Shell Integration* section with setup instructions for
  PowerShell, bash/zsh (WSL), oh-my-posh, and cmd.exe fallback.

### Fixed

- Command history no longer breaks when prompts contain `─`, `❯`, multi-line
  decorations, git branch names, conda/venv prefixes, or other characters that
  confused the previous `rfind('>')` heuristic.

## [1.2.1] - 2026-03-04

### Added

- **Font configuration** (`[font]` section in `config.toml`):
  - `family` — font family name (e.g. `"Cascadia Code"`, `"JetBrains Mono"`).
    Leave empty to inherit the host terminal's current font.
  - `size` — font size in points (`0` = inherit from host terminal).
  - `bold` — force bold rendering for all text (default: `false`).
  - `ligatures` — enable ligatures for supported fonts (default: `true`).
  - Settings are applied at startup via OSC 50 escape sequences where the
    host terminal supports them (silently ignored otherwise).
  - `config.example.toml` updated with the new `[font]` section and examples.

## [1.2.0] - 2026-03-04

### Performance

- **Dirty-line rendering**: `WmRenderer` now skips rows that have not changed
  since the last frame. Only rows marked dirty by the VT parser are redrawn,
  cutting render work to near-zero for idle panes (e.g. a vim split next to a
  running build log).
- **Per-pane output tracking**: panes that produced no new output since the
  last render pass are skipped entirely, not just their unchanged rows.
- **Batched SGR escape sequences**: `apply_attrs` and `apply_attrs_with_selection`
  now emit a single `\x1b[...m` sequence per attribute group instead of one
  `execute!()` call per attribute. Reduces write call overhead by 5–10x per
  styled cell group.
- **Resize debounce (30 ms)**: rapid terminal-resize events (fired every pixel
  during drag on Windows) are coalesced into a single resize + redraw after
  the window settles, eliminating flicker and redundant PTY resizes.
- **`clear_all_dirty()` after render**: dirty-line sets are now cleared after
  every render pass so subsequent frames start with a clean slate.

## [1.1.1] - 2025-01-21

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

- **Configurable prefix key**
  - Prefix key can be changed via `prefix_key` in config.toml
  - Uses tmux-style notation: `"C-b"` (Ctrl+B), `"C-a"` (Ctrl+A), etc.
  - Default: `"C-b"` (Ctrl+B, same as tmux)
  - Status bar and theme selector display adapt to configured key

- **MSIX package support**
  - Added `build-msix.ps1` script for creating MSIX packages
  - Supports unsigned packages (Developer Mode) and self-signed packages
  - Includes app execution alias for command-line access
  - Windows 10/11 compatible (requires Windows 10 SDK to build)

### Fixed

- **README configuration example** now matches actual config format
  - Fixed incorrect section names (`[general]`, `[appearance]` → top-level keys)
  - Fixed key names (`default_shell` → `shell`, `encoding` → `codepage`)
  - Added missing sections (`[tab_bar]`, `[status_bar]`, `[pane]`, `[scrollback]`)

- **Clipboard paste with LF-only line endings**
  - Text with Unix-style line endings (LF) now works correctly
  - Automatically converts LF to CR+LF for Windows shells

### Changed

- **Configuration directory moved to AppData**
  - Config and history files now stored in `%LOCALAPPDATA%\wtmux\`
  - Previously: `~/.wtmux/` (home directory)
  - Better alignment with Windows standards

- **Logging disabled by default**
  - No log files are created during normal operation
  - Use `--debug` (`-d`) flag to enable logging for troubleshooting
  - Log file: `%LOCALAPPDATA%\wtmux\wtmux.log`

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
