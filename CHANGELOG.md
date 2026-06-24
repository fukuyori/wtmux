## [1.6.0] - 2026-06-24

### Added

- **Optional prompt-based cwd tracking for arbitrary directory jumps**:
  cmd.exe and PowerShell sessions can opt in to a prompt hook that publishes
  cwd changes after aliases, functions, and directory-jump tools that do not
  use built-in `cd` commands.

- **cwd prompt hook controls**:
  the prompt hook is disabled by default and can be enabled with
  `cwd_prompt_hook = true` in `config.toml`, `--cwd-prompt-hook on`, or
  `-P on` at startup. `--cwd-prompt-hook off`, `-P off`, and
  `--no-cwd-prompt-hook` can force it off when the config enables it.

- **Mouse split resizing**:
  pane split boundaries can now be resized by dragging the border with the
  left mouse button.

### Fixed

- **Focused pane border redraws after focus changes**:
  pane focus changes now force border redraws so the previously focused pane no
  longer remains highlighted.

## [1.5.0] - 2026-06-12

### Added

- **tmux-compatible cwd query commands**:
  added a narrow tmux-compatible CLI surface for external cwd sync tools,
  including `wtmux display-message -p '#{pane_current_path}'`,
  `wtmux list-clients -F '#{client_pid}\t#{session_id}'`, and
  `wtmux display-message -p -t '<session_id>' '#{pane_current_path}'`.

- **Pane cwd tracking for shell integrations**:
  panes now track the best-known current directory from OSC 7 and Windows
  Terminal `OSC 9;9` cwd notifications, falling back to the wtmux startup
  directory when no shell notification is available.

## [1.4.0] - 2026-06-05

### Changed

- **New tabs can be created from the tab bar**:
  the tab bar now shows a `[+]` button next to the tabs, and clicking it creates
  a new tab.

- **Single-pane input no longer overlaps the status bar after resize**:
  Windows console resize events now use the reported cell count directly instead
  of adding an extra row and column.

- **Theme selector closes cleanly with Esc**:
  closing or applying the theme selector now forces the underlying panes to
  redraw, clearing the overlay immediately.

- **Tabs close promptly when their shell exits**:
  pane and tab cleanup now requests a render even when the exiting shell has no
  remaining output, so closed tabs disappear immediately in multi-tab sessions.

## [1.3.10] - 2026-06-05

### Changed

- **MSI upgrades now replace the installed binary**:
  bumped the installer/package version to `1.3.10` so Windows Installer treats
  the package as a newer upgrade and replaces older `1.3.9` installations.

## [1.3.9] - 2026-06-05

### Changed

- **Emoji input now works with Windows IME input**:
  Windows input is now read through a small wtmux wrapper that preserves
  key-down UTF-16 surrogate pairs from IME and emoji-panel input, fixing
  non-BMP characters such as `🚶`. PowerShell and PowerShell 7 sessions also
  set both `InputEncoding` and `OutputEncoding` to UTF-8 so ConPTY input is
  decoded correctly.

## [1.3.8] - 2026-05-30

### Changed

- **Mouse click no longer copies accidental single-cell selections**:
  wtmux now copies mouse selections on button release only after the selected
  range actually moves away from the initial mouse-down cell. Normal
  drag-to-copy behavior is preserved, and click-only selections are cleared
  without copying.

## [1.3.7] - 2026-05-08

### Changed

- **Split-pane cursor blink is steadier**:
  normal multi-pane rendering no longer hides and re-shows the host cursor on
  every frame, so output from another pane does not reset the focused cursor's
  blink cycle.

- **Renderer cursor handling was split out**:
  frame guards and cached cursor presentation now live in smaller UI modules,
  keeping `WmRenderer` focused on multi-pane composition.

## [1.3.6] - 2026-04-24

### Changed

- **Shortcut labels now follow configured keybindings**:
  the status bar, history selector title, and `wtmux --help` output now show
  the configured history selector shortcut instead of always showing `Ctrl+R`.

- **Help output now reflects the configured prefix key**:
  `wtmux --help` reads `config.toml` for display purposes and shows the active
  prefix key in the multi-pane keybinding list.

## [1.3.5] - 2026-04-22

### Changed

- **Phase 4 resize policy work started**:
  the runtime `Session::resize()` path now uses an explicit resize policy, and
  Windows defaults to `HostDriven` ordering so ConPTY / the host terminal can
  recalculate wrapping before wtmux updates its local screen state.

- **Host-driven resize preserves scrollback navigation**:
  added host-driven resize planning that keeps total line count and the
  scrolled-view anchor stable across resize, so scrollback remains reachable
  after a window size change.

- **Phase 2 logical-line refactor started**:
  introduced `LogicalLineView` and logical-line text collection helpers so
  wrapped physical rows can be read as a single logical line without changing
  the stored row layout yet.

- **Selection and command extraction now read logical lines**:
  moved key read paths away from raw physical-row traversal for selection text,
  shell integration command extraction, and current-line lookup.

## [1.3.0] - 2026-04-21

### Changed

- **Phase 1 resize / reflow refactor started**:
  resize policy and local reflow logic are now separated so follow-up `1.3.x`
  work can decouple rendering, logical lines, and host-driven resize handling
  without re-entangling `TerminalState::resize()`.

- **Planning document added for the `1.3.x` refactor series**:
  documented the staged resize / rendering refactor plan, with Phase 1
  implemented in `1.3.0` and later phases intended to proceed based on
  progress through the `1.3.x` series.

## [1.2.8] - 2026-04-20

### Changed

- **Faster startup path**:
  The command history selector is now initialized lazily on first use
  (`Ctrl+R`) instead of being loaded during startup.

- **Startup no longer creates config/history directories eagerly**:
  `%LOCALAPPDATA%\wtmux\` is now created only when config or history data is
  actually written, reducing unnecessary filesystem work during startup.

- **Reduced steady-state overhead**:
  - scrollback storage now trims from the front without shifting the whole buffer
  - `Session::process_output()` no longer collects an intermediate `Vec<Vec<u8>>`
  - dirty-line tracking uses a lighter row-indexed structure

- **Expanded prompt detection for command history (strip_prompt fallback)**:
  Added prompt-ending patterns for all major modern shells and themes.
  Patterns are ordered longest-first to avoid sub-sequence false positives,
  and ASCII patterns (>, $, #) are placed last to minimise false positives
  when those characters appear inside commands.
  - New: `╰─❯` (╰─❯, Powerlevel10k/oh-my-posh rounded multiline)
  - New: `❯` (❯), `➜` (➜), `` (Nerd Font ), `` (Nerd Font )
  - New: `⚡` (⚡ lightning), `🚀` (🚀 rocket), `λ` (λ lambda)
  - New: `→` (→), `›` (›)
  - New: `>>` (cmd.exe continuation), `% ` (zsh/fish/tcsh)
  - Existing: `>`, `$ `, `# `, `>>> ` (Python), etc.
  Note: strip_prompt is Priority-4 last-resort fallback; shells that emit
  OSC 133/633 markers (PowerShell 7, oh-my-posh, Starship, zsh) are handled
  by Priority 1/2 and are unaffected by this change.

## [1.2.6] - 2026-03-06

### Fixed

- **Multi-line paste now works correctly**:
  All line endings in pasted text are normalised to `
` (CR only) before
  being sent to the PTY.  Terminals interpret CR as a single Enter keypress;
  sending `
` (CRLF) caused PowerShell and other shells to see two
  newlines, which double-submitted lines or produced parse errors.
  The fix applies to both bracketed-paste mode and plain paste.

## [1.2.5] - 2026-03-05

### Added

- **`[font] suppress_bold`** config option:
  When the installed Nerd Font lacks a Bold face, the OS font-fallback
  substitutes a non-Nerd-Font Bold face for PUA code points, causing
  Powerline separators and icons to render incorrectly.  Setting
  `suppress_bold = true` tells wtmux never to emit SGR 1 (Bold), keeping
  all glyphs in the Regular face that contains the PUA glyphs.

- **`--vt-trace` flag**: records every raw byte received from the PTY
  to `%LOCALAPPDATA%\wtmux\vt_trace.log` in hex-dump + UTF-8 annotation
  format.  Useful for diagnosing prompt and colour rendering issues.

- **`VtParser::feed_char()`**: new method that routes decoded Unicode
  characters through the parser state machine.  Prevents multi-byte UTF-8
  characters from bypassing string-body states (DCS, APC, …) and being
  written directly to the screen buffer.

### Fixed

- **SGR 7 (Reverse Video) ignored by renderer** — root cause of Powerline
  colour rendering failure:
  oh-my-posh draws every Powerline separator arrow with `ESC[7m` (Reverse
  Video).  `AttrFlags::INVERSE` was set correctly by the parser but
  `apply_attrs_with_selection()` never emitted it.  Fixed by passing
  `ESC[7m` plus the original FG/BG colours through to the host terminal,
  letting the host perform the colour swap using its own background colour
  (theme-dependent; cannot be computed inside wtmux).

- **Font config breaking Powerline rendering when `family` is set**:
  Windows Terminal implements OSC 50 and switches to the named font when
  received.  If that font lacks Nerd Font / Powerline glyphs the arrows and
  icons break.  `apply_font_config()` no longer emits OSC 50.  Font must be
  configured in Windows Terminal's `settings.json` using a Nerd Font variant.

- **DCS / APC / SOS / PM string sequences not consumed by VT parser**:
  `ESC P` (DCS), `ESC _` (APC), `ESC X` (SOS), and `ESC ^` (PM) had no
  handling; the parser fell through to Ground and wrote the body bytes as
  visible characters.  Added `DcsString`, `ApcString`, `SosString`, and
  `PmString` states that consume content until ST (`ESC 0x5C`) or BEL.

- **Multi-byte UTF-8 characters bypassing parser state machine**:
  `session::feed_bytes()` decoded multi-byte sequences and called
  `put_char()` directly, skipping the parser.  Characters inside DCS/APC
  bodies were therefore rendered on screen instead of being discarded.
  All characters now go through `VtParser::feed_char()`.

- **PUA glyph cell-width tracking** (`unicode_width` returns 0 for PUA):
  Characters in U+E000–U+F8FF were treated as combining characters (width 0)
  and merged into the previous cell.  PUA range is now always treated as
  width 1, matching Windows Terminal behaviour.

- **Per-drift-cell `MoveTo` anchor for PUA glyphs**:
  When the host terminal renders a PUA glyph wider than the internal
  tracking width, subsequent cells drift right.  Cells containing a PUA
  codepoint now flush their run, emit `MoveTo(col_idx)`, write the glyph,
  and start a fresh run — resetting drift at every PUA glyph boundary.

## [1.2.4] - 2026-03-04

### Fixed

- **History overlay leaving artifacts after close**:
  The v1.2.0 dirty-line optimisation skipped pane redraws when no PTY output
  occurred.  Closing the history selector or context menu without any PTY
  activity left the overlay box visible on screen.  `force_full_redraw()` is
  now called on Esc, outside mouse click, and after menu action execution so
  the next render unconditionally redraws all panes.

## [1.2.3] - 2026-03-04

### Added

- **Delete history entries with the Del key**:
  In the command history selector (Ctrl+R), pressing Del removes the
  selected entry from both the in-memory list and the history file.  The
  list refreshes immediately; the cursor clamps to the last entry when the
  bottom entry is removed.
- Hint bar updated to: `Enter:Run  Del:Delete  S-Enter:&&  Esc:Close`

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
  - `family` — font family name; leave empty to inherit host terminal font.
  - `size` — font size in points (`0` = inherit).
  - `bold` — force bold rendering (default: `false`).
  - `ligatures` — enable ligatures for supported fonts (default: `true`).
  - `config.example.toml` updated with new `[font]` section and examples.

### Fixed

- Fixed MSI installer error 2503 (Windows Installer permissions issue on
  some systems).

## [1.2.0] - 2026-03-04

### Performance

- **Dirty-line rendering**: only rows marked dirty by the VT parser are
  redrawn, cutting render work to near-zero for idle panes.
- **Per-pane output tracking**: panes with no new output since the last
  render pass are skipped entirely.
- **Batched SGR escape sequences**: `apply_attrs_with_selection()` emits a
  single `\x1b[...m` sequence per attribute group, reducing write-call
  overhead by 5–10x per styled cell group.
- **Resize debounce (30 ms)**: rapid resize events during window drag are
  coalesced into one resize + redraw after the window settles.
- **`clear_all_dirty()` after render**: dirty-line sets are cleared after
  every render pass so subsequent frames start clean.

## [1.1.1] - 2025-01-21

### Added

- **Mouse event passthrough to child applications**:
  TUI apps that enable mouse capture (htop, mc, vim, …) now receive mouse
  events.  Hold Shift to bypass passthrough and use wtmux text selection.
- **Paste from context menu**: right-click → Paste; supports bracketed paste.
- **Configurable prefix key**: set `prefix_key` in `config.toml`
  (tmux-style notation: `"C-b"`, `"C-a"`, …).  Default: `"C-b"`.
- **MSIX package support**: `build-msix.ps1` for signed/unsigned packages.

### Fixed

- README configuration examples corrected to match actual config format.
- Clipboard paste with LF-only line endings now works (auto-converted to CRLF).

### Changed

- Configuration directory moved to `%LOCALAPPDATA%\wtmux\`.
- Debug logging disabled by default; use `--debug` to enable.

## [1.0.0] - 2025-01-18

### Added

- Context menu (right-click): Zoom/Unzoom, Split, Kill Pane.
- Tab bar mouse click to switch windows.
- Comprehensive API documentation.

### Fixed

- Split direction mapping for `─` and `│`.
- Context menu flicker on mouse hover.

## [0.4.0] - 2025-01-11

### Changed

- Unified frame management with `with_frame()` wrapper.
- `reflow()` as single entry point for geometry changes.
- Generation-based full-redraw detection.

### Fixed

- Zoom no longer causes a black screen.
- Cursor no longer disappears after render errors.
- Synchronized update boundary issues with `BufWriter` resolved.
- Autowrap state no longer leaks between render frames.

## [0.3.4] - 2025-01-11

### Fixed

- Japanese / CJK text no longer truncated or displayed incorrectly.
- Progress bar backslash artifacts fixed (OSC terminator `ESC \` now parsed
  correctly; affected Cargo build output since v0.1.0).
- Carriage return now marks the line dirty for redraw.

## [0.3.2] - 2025-01-09

### Added

- `-c, --cmd` option to explicitly launch Command Prompt.

### Fixed

- `shell` setting in `config.toml` now applied correctly.

## [0.3.1] - 2025-01-09

### Fixed

- Eliminated double shell startup for PowerShell / pwsh / WSL when using
  UTF-8 encoding.

## [0.3.0] - 2025-01-09

### Changed

- Default encoding changed to UTF-8 (was Shift-JIS).  Use `--sjis` for
  Shift-JIS when needed.

### Added

- **Command History** (Ctrl+R): persistent, shared across panes, max 1000
  entries.  Shift+Enter appends `&&`; Ctrl+Enter appends `&`.
- **Cursor shape reset**: Ctrl+B, r; auto-reset on pane switch.

### Fixed

- Double cmd.exe startup when using default UTF-8 encoding.

## [0.1.0] - 2025-01-08

### Added

- Initial release: window manager, pane splitting, copy mode, colour schemes,
  ConPTY backend, VT100/VT220 parser, mouse support, scrollback buffer,
  cmd / PowerShell / pwsh / WSL shell support, Inno Setup + WiX installers.

## [Unreleased]

### Planned

- Detach/attach support
- Session sharing
- Scripting support
- Custom keybinding configuration
- Status bar customization
- Plugin system
