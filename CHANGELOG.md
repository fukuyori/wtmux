## [3.4.1] - 2026-08-30

### Fixed

- Copy mode: copying text containing wide (CJK, etc.) characters no
  longer inserts a space after each one (`日本語` came out as
  `日 本 語`). Multi-codepoint graphemes (combining marks, emoji) are
  also copied intact, and a selection starting on the right half of a
  wide character now includes that character. (#7)

## [3.4.0] - 2026-08-30

### Added

- **Key cheat sheet** (`Prefix + ?`, tmux's `list-keys`): a scrollable
  popup listing every effective binding — after `config.toml` overrides
  and unbinds — grouped into Windows / Panes / Layouts / Scrollback &
  selection / Tools / Other, each with the command name and a one-line
  description. The legacy `[keybindings]` keys (history selector,
  scrollback, selection) are listed in their own section. `j`/`k`,
  `PgUp`/`PgDn`, `g`/`G` scroll; `q`, `Esc` or `?` close. Also
  `list-keys` (`lsk`) at the command prompt and as a bindable command.
- **tmux's layout keys**: `Prefix + M-1` … `M-5` now apply
  `even-horizontal`, `even-vertical`, `main-horizontal`, `main-vertical`
  and `tiled` directly (previously only reachable by cycling with
  `Space` or via `select-layout`).
- **`previous-layout`** (`prevl`) cycles the presets backwards; also
  available as `select-layout -p` (and `-n` for next), as a prompt
  command and as a bindable command. No default key.
- **`move-window` / `swap-window`** (tmux window reordering). `Prefix + .`
  then a digit `1-9` moves the current window to that position; the
  status bar shows the prompt and any other key cancels. Both are also
  prompt commands (`move-window -t <n>` / `movew`, `swap-window -t <n>`
  / `swapw`) and bindable commands (`move-window [-t <n>]`,
  `swap-window [-t <n>]`; without `-t` the key waits for a digit).

### Changed

- Window names no longer embed a number: new windows are named `main` /
  `shell` and the tab bar prefixes the current display position
  (`1:main`, `2:shell`), so the number stays correct after windows are
  moved, swapped, or closed. `rename-window` names show the same way
  (`3:build`).
- **Automatic pane titles**: panes are now titled after their working
  directory's last component (e.g. `wtmux` for
  `D:\home\source\rust\wtmux`) and follow `cd` as it happens. Panes in
  the same window that end up with the same name are numbered
  `wtmux`, `wtmux:2`, `wtmux:3`, … in pane order.
- Manual pane renaming was removed: the `rename-pane` prompt command
  and key binding (`Prefix + .`), the context menu's "Rename Pane"
  item, and right-click-on-title renaming are gone. Window renaming is
  unchanged.
- `cwd_prompt_hook` now defaults to **on** so pane titles can track the
  working directory out of the box. Set `cwd_prompt_hook = false` in
  `config.toml` (or pass `--no-cwd-prompt-hook`) to opt out; titles
  then stay at each pane's starting directory.

## [3.3.1] - 2026-08-21

### Fixed

- Message composer: `Up`/`Down` (and `Shift+Up`/`Shift+Down`, wheel
  scrolling) now move by display row in soft-wrapped text, keeping the
  x position in cells, instead of jumping by logical line.

### Changed

- Release package files are now uniformly named
  `wtmux-<version>-<os>-<arch>` (e.g. `wtmux-3.3.1-windows-x64.exe`
  / `.zip` / `.msi`, `wtmux-3.3.1-linux-amd64.deb`).
- Packaging scripts gained a `-Sign` option that code-signs the bundled
  executable, the Inno Setup installer and its uninstaller, and the MSI;
  the new `build-release-packages.ps1` builds all Windows packages in
  one go. The manual `install.ps1` was removed.

## [3.3.0] - 2026-08-16

### Added

- **Message composer editing upgrades** (`Prefix + m`):
  - Clipboard: `Ctrl+C` copies the selection, `Ctrl+X` cuts it, `Ctrl+A`
    selects all (system clipboard, like `Ctrl+V` paste).
  - Undo/redo with `Ctrl+Z` / `Ctrl+Y`; a run of plain typing undoes as
    one step, and clears, pastes, and history recalls are undoable too.
  - `Shift+Home` / `Shift+End` select to the start/end of the line;
    `Ctrl+Home` / `Ctrl+End` jump to the start/end of the message.
  - `Tab` inserts 4 spaces.
  - Mouse support: click places the cursor, drag selects, the wheel moves
    through the text, and dragging the right/bottom border (or corner)
    resizes the popup; the chosen size is kept for the session. Clicks
    outside the popup are ignored so a stray click cannot discard the
    draft.
  - The footer shows a line/character counter (e.g. `3L 45C`).

## [3.2.4] - 2026-08-06

### Added

- **Shift+Arrow selection in the message composer**: selections can span
  multiple lines, are highlighted in the popup, and are replaced by typing,
  Enter, Backspace, Delete, or pasted text.

### Fixed

- **Cancelling a message no longer restores it on the next open**: Esc now
  stores non-empty unfinished text in the composer's Ctrl+P/N history and the
  next message starts empty. A message whose send failed is still restored so
  it can be retried.

## [3.2.3] - 2026-08-06

### Added

- **`Prefix + .` renames the focused pane** (next to `Prefix + ,` =
  rename window). Also available as `rename-pane [name]` (`renamep`)
  in the command prompt and `[bind]` config; an empty name restores
  the default "Pane N" title. Previously pane rename was only
  reachable from the right-click context menu / title bar.

### Fixed

- **Right-clicking a pane title sometimes did not open the rename
  dialog**: when the app in the focused pane had mouse tracking on
  (vim, htop, Claude Code, …), clicks on the pane border — including
  the title row — were forwarded to the app instead of being handled
  as wtmux chrome. Border clicks now always stay with wtmux, and
  forwarded mouse coordinates are computed from the content area
  inside the border (they were off by one cell on bordered panes).
  Borderless panes (single pane, zoomed) no longer treat their top
  content row as a clickable title.
- **Leftover dialog fragments after closing overlays**: closing the
  rename dialog (Esc/Enter), the pane-numbers overlay, or the context
  menu via mouse did not force a full repaint, so panes with no new
  output kept showing pieces of the dismissed overlay. All overlay
  close paths now trigger a full redraw, matching the keyboard
  context-menu and window-selector behavior.

## [3.2.2] - 2026-08-01

### Added

- **`[keybindings]` entries can be disabled with `"none"`** (also
  `"off"` / `"disabled"`), letting the key pass through to the shell
  instead of falling back to the default shortcut.
- **OSC 8 hyperlinks**: links emitted by applications in panes (`ls
  --hyperlink`, gcc/rustc diagnostics, delta, starship, …) are stored
  per cell — including the `id=` grouping parameter — and re-emitted
  to the host terminal, so they stay Ctrl+clickable in Windows
  Terminal / WezTerm. URIs are sanitized (control characters stripped,
  length-capped) so a malicious child cannot smuggle escape sequences
  into the render stream, erased cells never stay clickable, and
  SGR 0 correctly leaves links open (they are orthogonal to SGR).

## [3.2.1] - 2026-08-01

### Added

- **Extended underlines (SGR `4:x`, 58, 59)**: curly / double / dotted /
  dashed underline styles and independent underline colors
  (`58:5:n` / `58:2::r:g:b`, colon and legacy semicolon forms) are now
  parsed per cell and re-emitted to the host terminal. nvim's LSP
  diagnostics (curly underlines) and modern prompts render correctly in
  Windows Terminal / WezTerm.

### Fixed

- **SGR colon subparameters are now parsed correctly.** Previously `:`
  was treated like `;`, so `4:3` (curly underline) was misread as
  "underline + italic" and `58:5:196` (underline color) accidentally
  enabled blink. The CSI parser now tracks subparameters separately,
  which also hardens it against other `:`-form sequences.

## [3.2.0] - 2026-08-01

### Added

- **win32-input-mode passthrough (DECSET 9001)**: wtmux now honors the
  win32-input-mode request every pane's conhost issues at startup and
  forwards the original Win32 key records (`CSI Vk;Sc;Uc;Kd;Cs;Rc _`)
  instead of lossy legacy VT bytes. Applications that read the console
  with `ReadConsoleInputW` now receive full modifier state — notably
  Shift+Enter / Ctrl+Enter / Alt+Enter, which legacy VT collapses to
  plain Enter — as well as key-release events. A pane's kitty keyboard
  flags take priority over win32-input-mode when both are active.
  Verified end-to-end through ConPTY by the
  `conpty_win32_input_roundtrip_repro` harness. (#2)
- **OSC 52 clipboard (write-only)**: applications in panes — nvim, or
  tmux/CLI tools inside ssh and WSL — can set the host clipboard with
  `OSC 52 ; c ; <base64>`. Read requests (`?`) are deliberately not
  answered so child programs cannot silently read the clipboard. (#2)
- **Focus reporting (DECSET 1004)**: panes that enable mode 1004
  receive `CSI I` / `CSI O` when the host terminal gains or loses focus
  and when wtmux's own pane focus moves between panes, like tmux's
  `focus-events`. (#2)

## [3.1.0] - 2026-08-01

### Added

- **Kitty keyboard protocol (panes)**: applications running inside a
  pane can now enable the kitty keyboard protocol's *disambiguate
  escape codes* (flag 1) and *report event types* (flag 2) progressive
  enhancements via `CSI = u` / `CSI > u` / `CSI < u`, and query support
  with `CSI ? u`. With flag 1 active, Esc, Ctrl/Alt-modified keys and
  modified Enter/Tab/Backspace are reported unambiguously as
  `CSI code;mods u` (e.g. Ctrl+I vs Tab, Shift+Enter vs Enter); with
  flag 2, key releases of escape-coded keys are reported as
  `CSI code;mods:3 u`. Flag stacks are tracked per screen (main /
  alternate), unsupported flag bits are masked so the query reply never
  over-advertises. Verified to survive the ConPTY hop in both
  directions (diagnostic harness: `conpty_kitty_roundtrip_repro`).
  This chiefly benefits VT-input applications in panes — neovim, and
  Linux TUIs (helix, fish 4, kakoune) inside WSL or ssh sessions.

- **Nested-session guard**: launching `wtmux` inside a wtmux pane now
  exits with an error instead of starting a nested instance (which
  would fight over the prefix key and stack ConPTY inside ConPTY),
  mirroring tmux's `$TMUX` check. CLI subcommands (`send-keys`,
  `list-keys`, ...) still work inside panes. Unset the `WTMUX`
  environment variable to force nesting.

### Fixed

- Multi-pane mode now honors the focused pane's terminal modes when
  encoding key input (e.g. DECCKM application cursor keys); previously
  it always used default modes.

## [3.0.1] - 2026-07-30

### Added

- **Terminal-interaction commands for `[bind]` / `[bind_root]`**:
  `scroll-up [n]`, `scroll-down [n]`, `scroll-top`, `scroll-bottom`,
  `extend-selection -L|-R|-U|-D`, `copy-selection` and
  `history-selector` expose the `[keybindings]` feature set as bindable
  commands, so scrollback / selection / copy keys can be freely chosen
  (and unbound) in `[bind_root]`. `[keybindings]` keeps working as a
  legacy layer; `[bind_root]` entries take precedence over it.

### Fixed

- The `[keybindings]` shortcuts (scrollback navigation, keyboard
  selection, copy-selection) now work in the default multi-pane mode;
  previously they were only wired up in `--simple` mode.

## [3.0.0] - 2026-07-30

### Added

- **Configurable key bindings (`[bind]` / `[bind_root]` / `unbind`)**:
  every key in the prefix table can now be reassigned or removed from
  `config.toml`, the way tmux's `bind-key` works, and prefix-less
  bindings (tmux's `bind-key -n`) are supported through `[bind_root]`:

  ```toml
  unbind = ["d"]                 # array must precede the [sections]

  [bind]
  "M-4" = "select-layout main-vertical"
  "|"   = "split-window -h"
  "z"   = ""                     # empty value unbinds

  [bind_root]
  "C-M-Left" = "select-pane -L"
  ```

  Commands use tmux names where an equivalent exists (`new-window`,
  `split-window -h`, `resize-pane -Z`, `select-layout <preset>`,
  `swap-pane -D`, ...) plus wtmux-only ones (`agent-dashboard`,
  `compose-message`, `next-attention`, `reset-cursor`). A malformed
  entry is skipped on its own with the reason printed to stderr, rather
  than discarding the whole table.
- **`wtmux list-keys` (alias `lsk`)**: prints the effective binding
  table, after config overrides, in the same syntax `[bind]` accepts.

### Changed

- Prefix bindings now match modifiers instead of ignoring them, so
  `Prefix, C-x` no longer triggers the binding for `x` unless nothing
  more specific matches. Holding Ctrl through the whole sequence
  (`C-b C-n`) still reaches the bare binding, and character bindings
  remain case-sensitive (`P` is distinct from `p`).

## [2.3.4] - 2026-07-29

### Changed

- **Pane border and title visibility**: the pane title is now always
  drawn in the same color as its surrounding border, so the focused
  pane's title picks up the active border color instead of a
  tab-bar-oriented color that could vanish on dark backgrounds. The
  unfocused border color was also brightened in all eight built-in
  themes (each theme now uses its palette's "comment" tone), making
  inactive pane frames and titles easier to distinguish without
  overpowering the focused pane.

## [2.3.3] - 2026-07-25

### Fixed

- **Ctrl+V not pasting in the message composer (`Prefix + m`)**: the
  composer only accepted terminal bracketed-paste events, so on
  terminals that forward Ctrl+V as a key press nothing happened. Ctrl+V
  now reads the system clipboard directly and inserts the text at the
  cursor; the composer help line lists the shortcut.

## [2.3.2] - 2026-07-23

### Fixed

- **Clipboard copy losing content on Linux (X11/XWayland)**: mouse-drag
  copy and copy-mode (`y`/`Enter`) created a fresh clipboard handle per
  copy and dropped it immediately, which released X11 selection
  ownership right away — the copy silently disappeared even though it
  reported success. A single clipboard handle is now kept alive for the
  life of the process, so copied text stays pasteable.

## [2.3.1] - 2026-07-21

### Added

- **`wtmux agents [--once]`**: a herdr-style agent monitor CLI. Run it
  in any pane (or a popup) to see the `Prefix + g` dashboard list —
  WORKING / BLOCKED / DONE / IDLE per pane, with focus and attention
  markers — refreshed four times a second until Ctrl+C. Backed by a new
  `list-agents` IPC request.
- **WORKING spinner**: panes in the WORKING state animate a Nerd Font
  circle-slice spinner (`󰪞…󰪥`, 250 ms per frame) in the `Prefix + g`
  agent dashboard and in `wtmux agents`.

## [2.3.0] - 2026-07-21

### Added

- **Message composer (`Prefix + m`)**: a floating multi-line editor for
  sending a message to a pane (typically an AI agent such as Claude
  Code). Enter inserts a newline; Ctrl+Enter (on terminals supporting
  the kitty keyboard protocol, and on Windows) or Ctrl+S sends; Esc
  cancels. The 8-row box soft-wraps long lines, works with IMEs
  (Japanese input shows its preedit inline), and delivers multi-line
  text via bracketed paste so agents receive it as one message. Sent
  messages are recallable with Ctrl+P / Ctrl+N while wtmux runs, and an
  unsent draft is restored the next time the composer opens. From the
  agent dashboard (`Prefix + g`), `m` composes to the selected pane.
- **Held popups**: `display-popup <command>` without `-E` now keeps the
  popup open after the command exits (title gains ` [exited]`), so
  short-lived commands like `ls` no longer flash and vanish. Any key
  closes it; the wheel and Up / Down / PageUp / PageDown / Home / End
  scroll the output. `-E` restores the auto-close behaviour, matching
  tmux. The wheel also scrolls a still-running popup's scrollback.
- **vim-style `:!<command>`**: the command prompt runs `!ls`,
  `!git log --oneline | head -5` etc. through `/bin/sh -c` in a held
  popup.

## [2.2.1] - 2026-07-20

### Added

- **Right-click rename**: right-clicking a tab in the tab bar opens the
  rename popup for that window (switching to it first), and
  right-clicking a pane's title row (top border) opens a rename popup
  for that pane. An empty pane name restores the default `Pane N`
  title.
- **Rename Pane in the context menu**: the right-click context menu
  gained a "Rename Pane" item.

## [2.2.0] - 2026-07-20

### Fixed

- **Prompt corruption when dragging a split border**: dragging a pane
  divider left the pane littered with stale prompt copies (especially
  with Powerline prompts such as oh-my-posh on macOS / Linux). Three
  fixes work together:
  - PTY resizes are deferred while the drag is in progress and flushed
    once on mouse-up, so the shell receives a single SIGWINCH (one
    prompt redraw) instead of a redraw storm racing the local reflow.
  - Panes whose size did not actually change no longer trigger a PTY
    ioctl or a local reflow on layout recomputes.
  - With OSC 133 / 633 shell integration active, the prompt + input
    rows are carried through a resize physically (pinned, not
    rewrapped) — the shell repaints them on SIGWINCH with
    cursor-relative sequences, which now line up. The prompt start row
    (OSC 133 A) is tracked so multi-line prompts stay stable too.

## [2.1.0] - 2026-07-20

### Added

- **Command prompt (`Prefix + :`)**: tmux-style command line on the status
  bar. Supports `split-window [-h]`, `new-window`, `kill-pane`,
  `kill-window`, `next-`/`previous-`/`last-window`, `select-window -t <n>`,
  `rename-window <name>`, `select-layout <preset>`, `resize-pane -Z`,
  `set synchronize-panes [on|off]`, `pipe-pane`, and
  `display-popup [command]`, including the usual tmux abbreviations
  (`splitw`, `neww`, `killp`, ...). Results and errors appear as a
  transient status-bar message.
- **`wtmux send-keys` / `wtmux capture-pane` CLI**: drive a running
  instance from outside — `wtmux send-keys -t 1.2 "cargo test" Enter`
  injects keys (with tmux key names like `Enter`, `Escape`, `C-c`, `Up`),
  `wtmux capture-pane -p [-S -]` prints a pane's screen (or full
  scrollback) to stdout. Targets default to the calling pane inside wtmux
  (`WTMUX_PANE`), or the focused pane; the instance is auto-selected when
  only one is running (`--pid` otherwise). Designed for orchestrating AI
  agents running in panes.
- **`display-popup`**: a centered floating pane running a command (default:
  your shell), tmux 3.2 style. Closes when the command exits;
  `Prefix, x` force-closes a stuck popup. Available from the command
  prompt (`:display-popup [command]`) and the CLI
  (`wtmux display-popup [command...]`).

- **Agent state hooks (`[hooks]`)**: config commands run (detached) when a
  pane's agent state changes — `on_agent_working`, `on_agent_blocked`,
  `on_agent_done`, `on_agent_idle`. The transition context is passed via
  `WTMUX_HOOK_*` environment variables, enabling e.g. a Windows toast the
  moment a background agent blocks on a question.
- **`wtmux report-state` CLI**: reports a pane's ground-truth agent state
  (`idle` / `working` / `blocked` / `done`) to the running instance,
  overriding the output heuristics. Designed to be called from an agent
  CLI's own hooks (e.g. Claude Code Stop / Notification hooks); targets the
  calling pane automatically via the new `WTMUX_PID` / `WTMUX_PANE`
  environment variables (override with `--pid` / `-t <window>.<pane>`).
  Reported transitions also fire `[hooks]` commands.
- **Pane output logging (tmux `pipe-pane` analog)**: `Prefix + Shift+P`
  toggles logging of the focused pane's raw output to
  `<data-dir>/logs/wtmux-<pid>-<window>.<pane>-<epoch>.log`; the status bar
  shows `[LOG]` while active.

## [2.0.2] - 2026-07-20

### Added

- **macOS release scripts**: `scripts/build-macos.sh` builds the release
  binary (arm64 / x86_64 / universal) and an unsigned `.pkg`;
  `scripts/sign-and-notarize-macos.sh` signs it with Developer ID
  (hardened runtime), builds a signed `.pkg`, submits it for notarization
  and staples the ticket. wtmux is now distributed for macOS as a signed,
  notarized `.pkg` installing to `/usr/local/bin/wtmux`.

### Fixed

- **Overlays no longer close on mouse movement**: the agent dashboard
  (`Prefix + g`) and the snippet selector were dismissed by any mouse
  event, so merely moving the pointer closed them. They now stay open on
  mouse move / drag / scroll and are dismissed only by an actual click
  (or their usual keys).

## [2.0.1] - 2026-07-20

### Added

- **Agent dashboard**: `Prefix + g` opens a live-updating overlay listing
  every pane across all windows with its agent state; `Enter` focuses the
  selected pane, `a` jumps to the next flagged pane, `q`/`Esc` closes.
- **Agent state summary in the status bar**: the number of working /
  blocked / done panes is shown as e.g. `2W 1B 1D`.

### Changed

- **Pane activity monitor upgraded to herdr-style states**: every pane is
  now classified as WORKING / BLOCKED / DONE / IDLE (tracked for focused
  panes too). BLOCKED is detected from bells, OSC 9 notifications, and
  output that stops on a question or permission prompt (`[y/n]`,
  "Do you want …", a trailing `?`); output that stops on an ordinary shell
  prompt now counts as IDLE and no longer raises the attention flag, so
  plain shells stop producing false alerts. A pane whose process exits is
  marked DONE.

## [2.0.0] - 2026-07-19

### Added

- **macOS / Linux support**: wtmux now runs on macOS and Linux in addition
  to Windows. A new POSIX pty backend (openpty + controlling terminal)
  mirrors the ConPTY wrapper's API, so sessions, rendering, and input are
  fully shared across platforms. On Unix the default shell is `$SHELL`
  (falling back to `/bin/sh`), the config/data directory follows XDG
  (`~/.config/wtmux`), the clipboard uses the native system clipboard, and
  the host terminal (iTerm2, Ghostty, WezTerm, kitty, etc.) is detected for
  the window title. Windows-only CLI flags (`-c`, `-p`, `-7`, `-w`,
  `--sjis`, `-n`) are hidden on Unix.
- **Pane activity monitor (agent multiplexing)**: every pane is watched for
  output and bells so background agents (e.g. AI coding agents) that finish
  or wait for input get flagged. A pane that produces output while
  unfocused and then goes quiet — or rings BEL / sends an OSC 9
  notification — is marked with `!` in the tab bar and a highlighted
  border (`*` marks panes actively producing output). `Prefix + a` jumps
  to the next flagged pane across windows; focusing a pane clears its flag.
  Configurable via the new `[activity]` section (`enabled`,
  `quiet_threshold_ms`).
- **Input broadcast (synchronize-panes)**: `Prefix + e` toggles sending
  keystrokes and pastes to every pane in the active window, shown as
  `[SYNC]` in the status bar. Bracketed-paste mode is honored per pane.

### Fixed

- **Startup crash on 0-sized terminals**: ptys that report a 0x0 size
  (CI harnesses, expect) no longer panic; wtmux falls back to 80x24.
- **Status bar overflow on narrow terminals**: the status line is now
  clipped to the terminal width by display cells instead of spilling past
  the last column.

## [1.8.1] - 2026-07-18

### Fixed

- **Old buffer content scrolled past when a pane was resized**:
  ConPTY replays the pane's entire text buffer after every resize
  (split, close, zoom, window resize), and each arriving chunk used to be
  rendered immediately, so the history visibly streamed from the top.
  The replay is now parsed off-screen and rendering resumes with a single
  frame of the final state once the stream has been quiet for 60 ms
  (hard cap 400 ms so continuously-printing shells are never frozen).
- **Cursor flickered while other panes were producing output**:
  render frames are now buffered and flushed in a single write, and the
  cursor is hidden during repaint and re-shown at its final position within
  the same frame. Previously each drawing command was flushed separately,
  letting the host terminal show the cursor hopping through whichever pane
  was being redrawn.
- **CJK window names could be split or exceed the rename popup limit**:
  window-name input and truncation now use terminal-cell width, so wide
  characters are kept intact and the 30-cell limit is applied consistently.

### Changed

- **Unified overlay state and rendering**:
  mutually exclusive UI modes now share one application-state model, and
  context menus, rename prompts, pane numbers, command history, and the
  window selector are composed through one renderer entry point. This keeps
  modal transitions and one-frame redraws consistent.

## [1.8.0] - 2026-07-17

### Added

- **tmux-style window selector**:
  `Ctrl+B, w` now opens a full-screen window chooser like tmux's
  `choose-tree`. The list shows each window's number, name, pane count, and
  tmux-style flags (`*` current, `-` last), with a live preview of the
  selected window's panes below the list. Windows expand into a tree:
  `Right`/`l` shows a window's panes as child rows (`-`/`+` marks
  expanded/collapsed), `Left`/`h` folds them again. Selecting a pane row
  previews just that pane, `Enter` on it switches to the window and focuses
  that pane, and `x` kills it. On window rows, move with the arrow keys or
  `j`/`k`, jump with `1`-`9`, switch with `Enter`, and kill with `x` (both
  kills ask `y/N` for confirmation); close the chooser with `Esc` or `q`.
  The mouse works too: scroll to move the selection, click a row to switch
  to it, or click outside the popup to close it; moving the mouse does not
  dismiss the chooser.

## [1.7.2] - 2026-07-07

### Fixed

- **Pane borders could inherit TUI highlight attributes**:
  pane border rendering now resets color and text attributes before drawing,
  so Vim mode changes and other full-screen TUI repaints no longer leave split
  borders with the application's background or reverse/highlight state.

## [1.7.1] - 2026-07-06

### Fixed

- **Stray spaces / blank spans appearing inside CJK text**:
  repainted rows are now erased (`ECH`) before being painted. wtmux's render
  output passes through the host terminal's ConPTY/conhost, which pads any
  bisected double-width character with a space — so overpainting wide glyphs
  whose columns had shifted since the previous frame (e.g. CJK text being
  re-wrapped while a TUI app like Claude Code streams output) corrupted the
  host's copy of the screen even though wtmux's own grid was correct.

- **Wide-character overwrite handling in the terminal state**:
  writing a wide char whose second cell lands on the lead cell of another
  wide char no longer leaves that char's continuation cell orphaned, and
  overwriting an orphaned continuation cell no longer blanks an unrelated
  neighboring cell (which could erase the wide char written just before it).

- **Multi-byte characters split across PTY reads were dropped**:
  PTY reads arrive in arbitrary-sized chunks, so a UTF-8 sequence can be cut
  at any byte; the leading bytes are now held until the continuation bytes
  arrive instead of being discarded.

- **Wide char landing on the last column was silently dropped**:
  it now wraps to the next line, matching how real terminals avoid splitting
  a double-width glyph across the right margin.

- **Partial erases could split a double-width pair**:
  EL/ECH/ICH/DCH and partial line erases now repair orphaned wide-char
  halves so the row's column accounting always matches the visible glyphs.

- **Kitty keyboard pop sequence printed a stray `u`**:
  the `<` private-prefix byte in `CSI < u` is now consumed as part of the
  sequence instead of aborting the parse and leaking the final byte.

- **Shift+Tab was not forwarded to applications**:
  BackTab is now sent as `ESC[Z`.

### Changed

- **Per-glyph cursor re-anchoring for non-ASCII runs**:
  rows are painted with the cursor re-anchored at each wide or multi-byte
  glyph, so any width disagreement with the host terminal is bounded to a
  single cell and can no longer accumulate across a row or bleed into a
  neighboring pane. Plain ASCII runs are still batched into single writes.

## [1.7.0] - 2026-07-03

### Changed

- **Faster rendering**:
  SGR escape sequences are now formatted in place instead of allocating
  strings per cell, and scrolling / line insert / line delete only redraw the
  rows that actually changed instead of the whole screen.

- **Lighter command history I/O**:
  adding a command now appends a single line to the history file instead of
  rewriting all entries; the file is compacted automatically once it grows
  past twice the entry limit.

- **Lower idle CPU usage**:
  the event loop relaxes its polling interval from 10ms to 50ms after about
  half a second of inactivity. Key input still wakes the loop immediately.

### Fixed

- **Escape-sequence injection via window titles**:
  OSC 0/1/2 titles are sanitized (control characters stripped, length capped)
  before being stored, so a program running in a pane can no longer inject
  escape sequences into pane border rendering.

- **Unbounded memory growth from unterminated OSC strings**:
  OSC string bodies are now capped at 4KB while waiting for a terminator.

- **Hostile escape sequences could stall or crash the parser**:
  oversized CSI parameters (e.g. `ESC[65535S`) are clamped to the screen
  height, and row access in insert/delete/erase character handlers no longer
  panics if the cursor invariant is ever violated.

- **Win32 handle leaks on PTY spawn failure**:
  pipe handles, the pseudo console, and the process attribute list are now
  released via RAII guards when `CreatePseudoConsole` or `CreateProcessW`
  fails.

- **Tab bar rendering could panic**:
  tab ids missing from the tab map are skipped instead of panicking.

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
