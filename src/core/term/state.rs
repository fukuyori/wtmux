//! Terminal state management
//!
//! This module defines the terminal's screen buffer, cursor state, and attributes.

use super::resize::{
    console_buffer_resize_screen, host_resize_screen, reflow_screen, ReflowAnchor, ResizeOutcome,
    ResizePolicy, ScreenResizePlan,
};
use super::width::{char_width, is_text_default_emoji, vs16_emoji_wide};
use bitflags::bitflags;
use std::collections::VecDeque;

/// Terminal state holding all screen data
pub struct TerminalState {
    pub cols: u16,
    pub rows: u16,
    pub primary_screen: ScreenBuffer,
    pub alternate_screen: ScreenBuffer,
    pub using_alternate: bool,
    pub primary_cursor: CursorState,
    pub alternate_cursor: CursorState,
    pub current_attrs: CellAttrs,
    pub modes: TerminalModes,
    pub title: String,
    /// Best-known current working directory for this pane.
    ///
    /// Initialized from the wtmux process cwd and updated by OSC 7 or
    /// Windows Terminal's OSC 9;9 cwd notifications when the child shell
    /// emits them.
    pub current_path: String,
    /// Scroll region (top, bottom) - 0-indexed, inclusive
    pub scroll_region: (u16, u16),
    /// Text selection state
    pub selection: Option<Selection>,
    /// Shell integration state (OSC 133 / OSC 633)
    pub shell_integration: ShellIntegration,
    /// Keystroke tracker (fallback when shell integration is inactive)
    pub keystroke_tracker: KeystrokeTracker,
    /// A BEL (or OSC 9 notification) arrived since this flag was last
    /// cleared. Consumed by the pane activity monitor to flag panes whose
    /// program (e.g. an AI agent) is asking for attention.
    pub bell: bool,
    /// Kitty keyboard protocol flag stacks (CSI = / > / < / ? u).
    pub kitty_keyboard: KittyKeyboardState,
    /// Per-state override of the host's VS16 emoji layout (`None` = use the
    /// value measured at startup, see `width::vs16_emoji_wide`). Tests set
    /// it explicitly; the application leaves it `None`.
    pub vs16_emoji_wide: Option<bool>,
    /// Decoded OSC 52 clipboard payload from the child, waiting to be
    /// written to the host clipboard. Consumed (`take()`) by the event
    /// loop, which owns clipboard access.
    pub osc52: Option<String>,
}

/// Kitty keyboard protocol progressive-enhancement flag stacks.
///
/// The spec requires the main and alternate screens to track their flags
/// independently, so there is one stack per screen. An empty stack means
/// "no enhancements" (flags 0, i.e. legacy encoding).
#[derive(Clone, Debug, Default)]
pub struct KittyKeyboardState {
    primary: Vec<u8>,
    alternate: Vec<u8>,
}

/// Progressive-enhancement bits wtmux honors: 1 = disambiguate escape
/// codes, 2 = report event types. Unsupported bits are masked out on
/// set/push so the `CSI ? u` reply never advertises behavior the key
/// encoder does not implement.
pub const KITTY_SUPPORTED_FLAGS: u8 = 0b0000_0011;

/// Push depth limit; pushing beyond it evicts the oldest entry, per spec.
const KITTY_STACK_LIMIT: usize = 8;

/// Text selection
#[derive(Clone, Debug)]
pub struct Selection {
    /// Start position (col, absolute_row) - in buffer coordinates (including scrollback)
    pub start: (u16, usize),
    /// End position (col, absolute_row) - in buffer coordinates (including scrollback)
    pub end: (u16, usize),
    /// Whether selection is active (mouse button held)
    pub active: bool,
}

impl TerminalState {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            cols,
            rows,
            primary_screen: ScreenBuffer::new(cols, rows),
            alternate_screen: ScreenBuffer::new(cols, rows),
            using_alternate: false,
            primary_cursor: CursorState::default(),
            alternate_cursor: CursorState::default(),
            current_attrs: CellAttrs::default(),
            modes: TerminalModes::default(),
            title: String::from("RustTerm"),
            current_path: std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            scroll_region: (0, rows.saturating_sub(1)),
            selection: None,
            shell_integration: ShellIntegration::default(),
            keystroke_tracker: KeystrokeTracker::default(),
            bell: false,
            kitty_keyboard: KittyKeyboardState::default(),
            vs16_emoji_wide: None,
            osc52: None,
        }
    }

    pub fn active_screen(&self) -> &ScreenBuffer {
        if self.using_alternate {
            &self.alternate_screen
        } else {
            &self.primary_screen
        }
    }

    pub fn active_screen_mut(&mut self) -> &mut ScreenBuffer {
        if self.using_alternate {
            &mut self.alternate_screen
        } else {
            &mut self.primary_screen
        }
    }

    pub fn active_cursor(&self) -> &CursorState {
        if self.using_alternate {
            &self.alternate_cursor
        } else {
            &self.primary_cursor
        }
    }

    pub fn active_cursor_mut(&mut self) -> &mut CursorState {
        if self.using_alternate {
            &mut self.alternate_cursor
        } else {
            &mut self.primary_cursor
        }
    }

    fn kitty_stack_mut(&mut self) -> &mut Vec<u8> {
        if self.using_alternate {
            &mut self.kitty_keyboard.alternate
        } else {
            &mut self.kitty_keyboard.primary
        }
    }

    /// Kitty keyboard flags currently in effect for the active screen
    /// (0 = legacy encoding).
    pub fn kitty_flags(&self) -> u8 {
        let stack = if self.using_alternate {
            &self.kitty_keyboard.alternate
        } else {
            &self.kitty_keyboard.primary
        };
        stack.last().copied().unwrap_or(0)
    }

    /// `CSI > flags u` — push a new flag entry onto the active screen's stack.
    pub fn kitty_push(&mut self, flags: u8) {
        let stack = self.kitty_stack_mut();
        if stack.len() >= KITTY_STACK_LIMIT {
            stack.remove(0);
        }
        stack.push(flags & KITTY_SUPPORTED_FLAGS);
    }

    /// `CSI < n u` — pop `n` entries; popping past the bottom leaves the
    /// screen with flags 0 (empty stack).
    pub fn kitty_pop(&mut self, n: u16) {
        let stack = self.kitty_stack_mut();
        for _ in 0..n.max(1) {
            if stack.pop().is_none() {
                break;
            }
        }
    }

    /// `CSI = flags ; mode u` — modify the current entry in place.
    /// Mode 1 replaces the flags, 2 sets the given bits, 3 clears them.
    pub fn kitty_set(&mut self, flags: u8, mode: u16) {
        let flags = flags & KITTY_SUPPORTED_FLAGS;
        let stack = self.kitty_stack_mut();
        if stack.is_empty() {
            stack.push(0);
        }
        let top = stack.last_mut().expect("stack non-empty");
        match mode {
            1 => *top = flags,
            2 => *top |= flags,
            3 => *top &= !flags,
            _ => {}
        }
    }

    /// Resize the terminal
    #[allow(dead_code)]
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.resize_with_policy(cols, rows, ResizePolicy::LocalReflow);
    }

    pub fn resize_with_policy(&mut self, cols: u16, rows: u16, policy: ResizePolicy) -> ResizeOutcome {
        self.cols = cols;
        self.rows = rows;
        let mut outcome = ResizeOutcome::default();

        match policy {
            ResizePolicy::LocalReflow => {
                let sb_len = self.primary_screen.scrollback.len();

                // Pin the active region: on the post-resize SIGWINCH the
                // shell erases and repaints its prompt + input line with
                // cursor-relative sequences, so the rows it is about to
                // touch must keep their physical layout instead of being
                // rewrapped. Rows above still reflow; they shift the pinned
                // region and the cursor by the same amount, which keeps the
                // shell's relative movements aligned.
                let rows_len = self.primary_screen.rows.len();
                let line_start = |row: usize| {
                    let mut r = row.min(rows_len.saturating_sub(1));
                    while r > 0 && self.primary_screen.rows[r - 1].wrapped {
                        r -= 1;
                    }
                    r
                };
                let cursor_line_start = line_start(self.primary_cursor.row as usize);
                // Pin only when OSC 133 markers confirm the cursor sits on
                // the shell's input line — then the shell is guaranteed to
                // repaint the pinned rows. Without that confidence, rewrap
                // everything as before to preserve content (nothing may
                // repaint a truncated row). The pin extends to the prompt's
                // first row so multi-line prompts stay stable; the distance
                // guard protects against stale markers.
                let pin_from_abs_row = match self.shell_integration.prompt_end_row {
                    Some(prompt_end)
                        if line_start(prompt_end as usize) == cursor_line_start =>
                    {
                        let mut pin_start = cursor_line_start;
                        if let Some(prompt_start) = self.shell_integration.prompt_start_row {
                            let prompt_start = prompt_start as usize;
                            if prompt_start <= pin_start && pin_start - prompt_start <= 4 {
                                pin_start = line_start(prompt_start);
                            }
                        }
                        Some(sb_len + pin_start)
                    }
                    _ => None,
                };

                let mut primary_anchors = vec![ReflowAnchor {
                    abs_row: sb_len + self.primary_cursor.row as usize,
                    col: self.primary_cursor.col,
                }];
                let prompt_anchor_idx = if let (Some(prompt_row), Some(prompt_col)) = (
                    self.shell_integration.prompt_end_row,
                    self.shell_integration.prompt_end_col,
                ) {
                    primary_anchors.push(ReflowAnchor {
                        abs_row: sb_len + prompt_row as usize,
                        col: prompt_col,
                    });
                    Some(primary_anchors.len() - 1)
                } else {
                    None
                };
                let prompt_start_anchor_idx =
                    self.shell_integration.prompt_start_row.map(|row| {
                        primary_anchors.push(ReflowAnchor {
                            abs_row: sb_len + row as usize,
                            col: 0,
                        });
                        primary_anchors.len() - 1
                    });

                let primary_plan = reflow_screen(
                    &self.primary_screen,
                    cols,
                    rows,
                    &primary_anchors,
                    pin_from_abs_row,
                );
                let primary_positions = primary_plan.anchor_positions.clone();
                self.primary_screen.apply_resize_plan(primary_plan, cols, rows);

                if let Some(Some((row, col))) = primary_positions.first() {
                    self.primary_cursor.row = *row;
                    self.primary_cursor.col = *col;
                    outcome.primary_cursor = Some((*row, *col));
                }

                if let Some(idx) = prompt_anchor_idx {
                    match primary_positions.get(idx).copied().flatten() {
                        Some((row, col)) => {
                            self.shell_integration.prompt_end_row = Some(row);
                            self.shell_integration.prompt_end_col = Some(col);
                            outcome.prompt_anchor = Some((row, col));
                        }
                        None => {
                            self.shell_integration.prompt_end_row = None;
                            self.shell_integration.prompt_end_col = None;
                        }
                    }
                }

                if let Some(idx) = prompt_start_anchor_idx {
                    self.shell_integration.prompt_start_row = primary_positions
                        .get(idx)
                        .copied()
                        .flatten()
                        .map(|(row, _)| row);
                }
            }
            ResizePolicy::ConsoleBuffer => {
                // Anchor rows are indices into the visible screen (the
                // console buffer has no scrollback). anchors[0] = cursor.
                let mut anchors = vec![ReflowAnchor {
                    abs_row: self.primary_cursor.row as usize,
                    col: self.primary_cursor.col,
                }];
                let prompt_end_idx = match (
                    self.shell_integration.prompt_end_row,
                    self.shell_integration.prompt_end_col,
                ) {
                    (Some(row), Some(col)) => {
                        anchors.push(ReflowAnchor { abs_row: row as usize, col });
                        Some(anchors.len() - 1)
                    }
                    _ => None,
                };
                let prompt_start_idx = self.shell_integration.prompt_start_row.map(|row| {
                    anchors.push(ReflowAnchor { abs_row: row as usize, col: 0 });
                    anchors.len() - 1
                });

                let plan = console_buffer_resize_screen(&self.primary_screen, cols, rows, &anchors);
                let positions = plan.anchor_positions.clone();
                self.primary_screen.apply_resize_plan(plan, cols, rows);

                if let Some(Some((row, col))) = positions.first() {
                    self.primary_cursor.row = *row;
                    self.primary_cursor.col = *col;
                    outcome.primary_cursor = Some((*row, *col));
                }
                if let Some(idx) = prompt_end_idx {
                    match positions.get(idx).copied().flatten() {
                        Some((row, col)) => {
                            self.shell_integration.prompt_end_row = Some(row);
                            self.shell_integration.prompt_end_col = Some(col);
                            outcome.prompt_anchor = Some((row, col));
                        }
                        None => {
                            self.shell_integration.prompt_end_row = None;
                            self.shell_integration.prompt_end_col = None;
                        }
                    }
                }
                if let Some(idx) = prompt_start_idx {
                    self.shell_integration.prompt_start_row =
                        positions.get(idx).copied().flatten().map(|(row, _)| row);
                }
            }
            ResizePolicy::HostDriven | ResizePolicy::NoReflow => {
                let primary_plan = host_resize_screen(&self.primary_screen, cols, rows);
                self.primary_screen.apply_resize_plan(primary_plan, cols, rows);
            }
        }

        self.alternate_screen.resize(cols, rows);
        self.scroll_region = (0, rows.saturating_sub(1));

        // Clamp cursor positions
        let max_col = cols.saturating_sub(1);
        let max_row = rows.saturating_sub(1);
        
        self.primary_cursor.col = self.primary_cursor.col.min(max_col);
        self.primary_cursor.row = self.primary_cursor.row.min(max_row);
        self.alternate_cursor.col = self.alternate_cursor.col.min(max_col);
        self.alternate_cursor.row = self.alternate_cursor.row.min(max_row);

        outcome
    }

    /// Put a character at the current cursor position
    pub fn put_char(&mut self, ch: char) {
        let width = char_width(ch) as u16;

        if width == 0 {
            // VARIATION SELECTOR-16 asks for emoji presentation. On hosts
            // that honour it (measured at startup) a text-default emoji such
            // as ❤ grows from one cell to two, so the lead cell is widened.
            let host_widens = self.vs16_emoji_wide.unwrap_or_else(vs16_emoji_wide);
            if ch == '\u{FE0F}'
                && host_widens
                && self.extend_cluster_before_cursor(ch, is_text_default_emoji)
            {
                return;
            }
            // Combining character - append to previous cell
            self.append_to_previous_cell(ch);
            return;
        }

        // Two regional indicators form one flag; halfwidth katakana plus a
        // halfwidth (han)dakuten form one glyph pair. Both are drawn as a
        // single two-cell cluster by terminals, so keep them in one cell.
        let is_regional_indicator = |c: char| ('\u{1F1E6}'..='\u{1F1FF}').contains(&c);
        if is_regional_indicator(ch) && self.extend_cluster_before_cursor(ch, is_regional_indicator) {
            return;
        }
        if matches!(ch, '\u{FF9E}' | '\u{FF9F}')
            && self.extend_cluster_before_cursor(ch, |c| ('\u{FF66}'..='\u{FF9D}').contains(&c))
        {
            return;
        }

        // Get cursor position first
        let (cursor_row, cursor_col) = {
            let cursor = self.active_cursor();
            (cursor.row, cursor.col)
        };

        // A character right after a ZERO WIDTH JOINER continues the previous
        // grapheme cluster (👨‍👩‍👧 is one cluster, not three emoji), which is
        // how every measured terminal — and the applications' own width
        // libraries — lay it out. Fold it into the lead cell instead of
        // opening new cells, so the cluster stays 2 cells wide.
        if cursor_col > 0 && self.join_after_zwj(cursor_row as usize, cursor_col as usize, ch) {
            return;
        }

        // Handle line wrap - either the cursor is completely beyond the screen
        // edge, or a wide char lands exactly on the last column: it has no
        // room for its continuation cell there, and rendering later clips any
        // cell that would cross the right edge (to avoid bleeding into a
        // neighboring pane), so writing it there would silently drop it.
        // Wrapping early instead matches how real terminals avoid splitting a
        // double-width glyph across the margin.
        let wide_char_needs_early_wrap = width == 2 && cursor_col + 1 == self.cols;
        if cursor_col >= self.cols || wide_char_needs_early_wrap {
            if self.modes.auto_wrap {
                {
                    let screen = self.active_screen_mut();
                    screen.rows[cursor_row as usize].wrapped = true;
                }
                self.active_cursor_mut().col = 0;
                self.linefeed();
            } else {
                // No wrap - clamp to last position
                self.active_cursor_mut().col = self.cols.saturating_sub(1);
            }
        }

        // Get updated cursor position
        let (row, col) = {
            let cursor = self.active_cursor();
            (cursor.row as usize, cursor.col as usize)
        };
        
        // Ensure col is within bounds for writing
        if col >= self.cols as usize {
            return;
        }

        // Handle overwriting wide characters. A wide char covers two cells,
        // so both columns it lands on must be checked: overwriting only the
        // second cell can also bisect an existing wide pair there (new char
        // at cols N..N+1, old wide char at N+1..N+2 — the old char's
        // continuation at N+2 would otherwise be left orphaned).
        self.handle_wide_char_overwrite(row, col);
        if width == 2 && col + 1 < self.cols as usize {
            self.handle_wide_char_overwrite(row, col + 1);
        }

        // Clone attrs before mutable borrow
        let attrs = self.current_attrs.clone();
        let cols = self.cols;

        let screen = self.active_screen_mut();

        // Write the character
        screen.rows[row].cells[col] = Cell {
            grapheme: ch.to_string(),
            width: width as u8,
            attrs: attrs.clone(),
        };

        // For wide characters, mark next cell as continuation (only if it fits)
        if width == 2 && col + 1 < cols as usize {
            screen.rows[row].cells[col + 1] = Cell::continuation(&attrs);
        }

        screen.mark_dirty(row);

        // Move cursor by character width
        self.active_cursor_mut().col += width;
    }

    /// Grow the one-cell grapheme cluster just before the cursor by `ch`
    /// into a two-cell cluster: continuation at the cursor column, cursor
    /// advanced by one. `base_ok` decides whether the lead cell's single
    /// code point may start such a cluster with `ch`. Returns `false` (and
    /// changes nothing) when the previous cell is not a one-cell single code
    /// point accepted by `base_ok`.
    ///
    /// Terminals lay these clusters out as one two-cell glyph (a flag from
    /// two regional indicators, ❤️ from ❤ + VS16, ｶﾞ from ｶ + dakuten).
    /// Keeping them in one cell means the renderer emits them in one write,
    /// which matters: at least WezTerm drops the character that follows a
    /// cluster whose second half arrived through a separate positioned write.
    fn extend_cluster_before_cursor(&mut self, ch: char, base_ok: impl Fn(char) -> bool) -> bool {
        let (row, col) = {
            let cursor = self.active_cursor();
            (cursor.row as usize, cursor.col as usize)
        };
        if col == 0 {
            return false;
        }
        let cols = self.cols as usize;
        {
            let lead = &self.active_screen().rows[row].cells[col - 1];
            let mut chars = lead.grapheme.chars();
            let single_base = matches!((chars.next(), chars.next()), (Some(c), None) if base_ok(c));
            if lead.is_continuation() || lead.width != 1 || !single_base {
                return false;
            }
        }
        if col >= cols {
            // No room for a second cell at the right margin: keep one cell
            // (the host has the same problem and wraps or clips there).
            self.active_screen_mut().rows[row].cells[col - 1].grapheme.push(ch);
            self.active_screen_mut().mark_dirty(row);
            return true;
        }
        self.handle_wide_char_overwrite(row, col);
        let screen = self.active_screen_mut();
        let attrs = screen.rows[row].cells[col - 1].attrs.clone();
        screen.rows[row].cells[col - 1].grapheme.push(ch);
        screen.rows[row].cells[col - 1].width = 2;
        screen.rows[row].cells[col] = Cell::continuation(&attrs);
        screen.mark_dirty(row);
        self.active_cursor_mut().col += 1;
        true
    }

    /// If the grapheme just before the cursor ends with U+200D, append `ch`
    /// to it and report `true`; the cursor does not move.
    fn join_after_zwj(&mut self, row: usize, col: usize, ch: char) -> bool {
        let screen = self.active_screen_mut();
        let cells = &mut screen.rows[row].cells;
        let target = if cells[col - 1].is_continuation() && col > 1 {
            col - 2
        } else {
            col - 1
        };
        if !cells[target].grapheme.ends_with('\u{200D}') {
            return false;
        }
        cells[target].grapheme.push(ch);
        screen.mark_dirty(row);
        true
    }

    fn append_to_previous_cell(&mut self, ch: char) {
        let (row, col) = {
            let cursor = self.active_cursor();
            (cursor.row as usize, cursor.col as usize)
        };

        if col > 0 {
            let screen = self.active_screen_mut();
            let cells = &mut screen.rows[row].cells;
            // After a wide char the cursor sits past its continuation cell;
            // the grapheme lives in the lead cell one further left. Appending
            // to the continuation cell would silently drop the character
            // (continuation cells contribute nothing to rendering or copy),
            // which broke ZWJ emoji sequences such as 👨‍👩‍👧.
            let target = if cells[col - 1].is_continuation() && col > 1 {
                col - 2
            } else {
                col - 1
            };
            cells[target].grapheme.push(ch);
            screen.mark_dirty(row);
        }
    }

    fn handle_wide_char_overwrite(&mut self, row: usize, col: usize) {
        let attrs = self.current_attrs.clone();
        let cols = self.cols as usize;
        let screen = self.active_screen_mut();

        // Check if we're overwriting the right half of a wide char. Only
        // blank the left neighbor when it really is the wide lead of this
        // continuation — an orphaned continuation (its lead already
        // overwritten) must not blank an unrelated neighbor, e.g. the fresh
        // continuation of a wide char written just before this one.
        if col > 0
            && screen.rows[row].cells[col].is_continuation()
            && screen.rows[row].cells[col - 1].width == 2
        {
            screen.rows[row].cells[col - 1] = Cell {
                grapheme: " ".to_string(),
                width: 1,
                attrs: attrs.clone(),
            };
        }

        // Check if we're overwriting the left half of a wide char
        if screen.rows[row].cells[col].width == 2 && col + 1 < cols {
            screen.rows[row].cells[col + 1] = Cell {
                grapheme: " ".to_string(),
                width: 1,
                attrs,
            };
        }
    }

    /// Carriage return - move cursor to column 0
    pub fn carriage_return(&mut self) {
        let row = self.active_cursor().row as usize;
        self.active_cursor_mut().col = 0;
        // Mark the line dirty since content may be overwritten
        self.active_screen_mut().mark_dirty(row);
    }

    /// Line feed - move cursor down, scroll if needed
    pub fn linefeed(&mut self) {
        let cursor_row = self.active_cursor().row;
        let scroll_bottom = self.scroll_region.1;
        let rows = self.rows;

        if cursor_row >= scroll_bottom {
            // At bottom of scroll region - scroll up
            self.scroll_up(1);
        } else if cursor_row < rows - 1 {
            self.active_cursor_mut().row += 1;
        }
    }

    /// Backspace - move cursor left
    pub fn backspace(&mut self) {
        let cursor = self.active_cursor_mut();
        if cursor.col > 0 {
            cursor.col -= 1;
        }
    }

    /// Horizontal tab
    pub fn horizontal_tab(&mut self) {
        let cols = self.cols;
        let cursor = self.active_cursor_mut();
        // Move to next tab stop (every 8 columns)
        cursor.col = ((cursor.col / 8) + 1) * 8;
        if cursor.col >= cols {
            cursor.col = cols.saturating_sub(1);
        }
    }

    /// Scroll the screen up by n lines
    pub fn scroll_up(&mut self, n: u16) {
        let (top, bottom) = self.scroll_region;
        let cols = self.cols;
        let is_primary = !self.using_alternate;

        // Scrolling more than the region height is visually identical to
        // scrolling exactly the region height; clamp so a hostile
        // `\e[65535S` can't spin this loop for no visible effect.
        let region_height = bottom.saturating_sub(top).saturating_add(1);
        let n = n.min(region_height);

        let screen = self.active_screen_mut();

        for _ in 0..n {
            if (top as usize) < screen.rows.len() && (bottom as usize) < screen.rows.len() {
                let removed_row = screen.rows.remove(top as usize);
                // Save to scrollback only for primary screen and when scrolling from top
                if is_primary && top == 0 {
                    screen.push_to_scrollback(removed_row);
                }
                screen.rows.insert(bottom as usize, Row::new(cols));
            }
        }
        if screen.scroll_offset > 0 {
            // Viewing scrollback: every visible row's content shifts
            screen.mark_all_dirty();
        } else {
            // Only rows inside the scroll region changed
            for row in (top as usize)..=(bottom as usize).min(screen.rows.len().saturating_sub(1)) {
                screen.mark_dirty(row);
            }
        }
    }

    /// Scroll the screen down by n lines
    pub fn scroll_down(&mut self, n: u16) {
        let (top, bottom) = self.scroll_region;
        let cols = self.cols;

        let region_height = bottom.saturating_sub(top).saturating_add(1);
        let n = n.min(region_height);

        let screen = self.active_screen_mut();

        for _ in 0..n {
            if (bottom as usize) < screen.rows.len() && (top as usize) <= screen.rows.len() {
                screen.rows.remove(bottom as usize);
                screen.rows.insert(top as usize, Row::new(cols));
            }
        }
        if screen.scroll_offset > 0 {
            screen.mark_all_dirty();
        } else {
            // Only rows inside the scroll region changed
            for row in (top as usize)..=(bottom as usize).min(screen.rows.len().saturating_sub(1)) {
                screen.mark_dirty(row);
            }
        }
    }

    /// Cursor up
    pub fn cursor_up(&mut self, n: u16) {
        let cursor = self.active_cursor_mut();
        cursor.row = cursor.row.saturating_sub(n);
    }

    /// Cursor down
    pub fn cursor_down(&mut self, n: u16) {
        let rows = self.rows;
        let cursor = self.active_cursor_mut();
        cursor.row = (cursor.row + n).min(rows.saturating_sub(1));
    }

    /// Cursor forward (right)
    pub fn cursor_forward(&mut self, n: u16) {
        let cols = self.cols;
        let cursor = self.active_cursor_mut();
        cursor.col = (cursor.col + n).min(cols.saturating_sub(1));
    }

    /// Cursor backward (left)
    pub fn cursor_backward(&mut self, n: u16) {
        let cursor = self.active_cursor_mut();
        cursor.col = cursor.col.saturating_sub(n);
    }

    /// Set cursor position (1-indexed parameters)
    pub fn cursor_position(&mut self, row: u16, col: u16) {
        let rows = self.rows;
        let cols = self.cols;
        let cursor = self.active_cursor_mut();
        cursor.row = row.saturating_sub(1).min(rows.saturating_sub(1));
        cursor.col = col.saturating_sub(1).min(cols.saturating_sub(1));
    }

    /// Erase in display
    pub fn erase_in_display(&mut self, mode: u16) {
        match mode {
            0 => {
                // From cursor to end
                self.erase_in_line(0);
                let cursor_row = self.active_cursor().row as usize;
                let rows = self.rows as usize;
                let attrs = self.current_attrs.clone();
                let screen = self.active_screen_mut();
                for r in (cursor_row + 1)..rows {
                    if r < screen.rows.len() {
                        screen.rows[r].clear(&attrs);
                        screen.mark_dirty(r);
                    }
                }
            }
            1 => {
                // From start to cursor
                let cursor_row = self.active_cursor().row as usize;
                let attrs = self.current_attrs.clone();
                {
                    let screen = self.active_screen_mut();
                    for r in 0..cursor_row {
                        if r < screen.rows.len() {
                            screen.rows[r].clear(&attrs);
                            screen.mark_dirty(r);
                        }
                    }
                }
                self.erase_in_line(1);
            }
            2 | 3 => {
                // Entire screen
                let rows = self.rows as usize;
                let attrs = self.current_attrs.clone();
                let screen = self.active_screen_mut();
                for r in 0..rows {
                    if r < screen.rows.len() {
                        screen.rows[r].clear(&attrs);
                        screen.mark_dirty(r);
                    }
                }
            }
            _ => {}
        }
    }

    /// Erase in line
    pub fn erase_in_line(&mut self, mode: u16) {
        let (cursor_row, cursor_col) = {
            let cursor = self.active_cursor();
            (cursor.row as usize, cursor.col as usize)
        };
        let cols = self.cols as usize;
        let attrs = self.current_attrs.clone();

        let screen = self.active_screen_mut();
        let row = cursor_row;

        if row >= screen.rows.len() {
            return;
        }

        match mode {
            0 => {
                // From cursor to end of line
                for c in cursor_col..cols {
                    if c < screen.rows[row].cells.len() {
                        screen.rows[row].cells[c].clear(&attrs);
                    }
                }
            }
            1 => {
                // From start to cursor
                for c in 0..=cursor_col {
                    if c < screen.rows[row].cells.len() {
                        screen.rows[row].cells[c].clear(&attrs);
                    }
                }
            }
            2 => {
                // Entire line
                screen.rows[row].clear(&attrs);
            }
            _ => {}
        }
        // A partial erase can land mid wide-char (e.g. the cursor sits on a
        // continuation cell), leaving an orphaned width-2 half whose column
        // accounting no longer matches the visible glyphs.
        screen.rows[row].repair_wide_pairs();
        screen.mark_dirty(row);
    }

    /// Insert lines at cursor position
    pub fn insert_lines(&mut self, n: u16) {
        let cursor_row = self.active_cursor().row as usize;
        let total_rows = self.rows as usize;
        let cols = self.cols;

        let screen = self.active_screen_mut();

        // Inserting more lines than fit below the cursor has no further effect
        let n = (n as usize).min(screen.rows.len().saturating_sub(cursor_row));
        for _ in 0..n {
            if cursor_row < screen.rows.len() {
                screen.rows.insert(cursor_row, Row::new(cols));
                if screen.rows.len() > total_rows {
                    screen.rows.pop();
                }
            }
        }
        if screen.scroll_offset > 0 {
            screen.mark_all_dirty();
        } else {
            // Rows from the cursor down shifted
            for row in cursor_row..screen.rows.len() {
                screen.mark_dirty(row);
            }
        }
    }

    /// Delete lines at cursor position
    pub fn delete_lines(&mut self, n: u16) {
        let cursor_row = self.active_cursor().row as usize;
        let cols = self.cols;

        let screen = self.active_screen_mut();

        // Deleting more lines than exist below the cursor has no further effect
        let n = (n as usize).min(screen.rows.len().saturating_sub(cursor_row));
        for _ in 0..n {
            if cursor_row < screen.rows.len() {
                screen.rows.remove(cursor_row);
                screen.rows.push(Row::new(cols));
            }
        }
        if screen.scroll_offset > 0 {
            screen.mark_all_dirty();
        } else {
            // Rows from the cursor down shifted
            for row in cursor_row..screen.rows.len() {
                screen.mark_dirty(row);
            }
        }
    }

    /// Set scroll region
    pub fn set_scroll_region(&mut self, top: u16, bottom: u16) {
        let rows = self.rows;
        let top = top.saturating_sub(1).min(rows.saturating_sub(1));
        let bottom = bottom.saturating_sub(1).min(rows.saturating_sub(1));
        if top < bottom {
            self.scroll_region = (top, bottom);
        }
    }

    /// Save cursor position
    pub fn save_cursor(&mut self) {
        let (col, row) = {
            let cursor = self.active_cursor();
            (cursor.col, cursor.row)
        };
        let attrs = self.current_attrs.clone();
        let saved = SavedCursor { col, row, attrs };
        self.active_cursor_mut().saved = Some(saved);
    }

    /// Restore cursor position
    pub fn restore_cursor(&mut self) {
        let saved = self.active_cursor().saved.clone();
        if let Some(saved) = saved {
            let cursor = self.active_cursor_mut();
            cursor.col = saved.col;
            cursor.row = saved.row;
            self.current_attrs = saved.attrs;
        }
    }

    /// DECRQM: the DECRPM status of a mode, 1 = set, 2 = reset, 0 = not
    /// recognized. A mode is recognized only if `set_private_mode` (or the
    /// ANSI `h`/`l` handler) changes some state for it, so the answer never
    /// promises behavior wtmux does not have; modes that merely trigger an
    /// action (1048) and modes it ignores (2027) stay 0. 47, 1047 and
    /// 1049 all report whether the alternate screen is active, since the
    /// number that switched to it is not recorded.
    pub fn query_mode(&self, ansi: bool, mode: u16) -> u8 {
        let on = if ansi {
            match mode {
                4 => self.modes.insert_mode,
                20 => self.modes.linefeed_newline,
                _ => return 0,
            }
        } else {
            match mode {
                1 => self.modes.application_cursor,
                7 => self.modes.auto_wrap,
                25 => self.active_cursor().visible,
                47 | 1047 | 1049 => self.using_alternate,
                1004 => self.modes.focus_reporting,
                2004 => self.modes.bracketed_paste,
                9001 => self.modes.win32_input,
                2026 => self.modes.synchronized_output,
                1000 => self.modes.mouse_tracking,
                1002 => self.modes.mouse_button_tracking,
                1003 => self.modes.mouse_any_event,
                1006 => self.modes.mouse_sgr_mode,
                1015 => self.modes.mouse_urxvt_mode,
                _ => return 0,
            }
        };
        if on {
            1
        } else {
            2
        }
    }

    /// Set private mode
    pub fn set_private_mode(&mut self, mode: u16, enable: bool) {
        match mode {
            1 => self.modes.application_cursor = enable,
            7 => self.modes.auto_wrap = enable,
            25 => self.active_cursor_mut().visible = enable,
            47 | 1047 => {
                if enable {
                    self.using_alternate = true;
                    self.alternate_screen = ScreenBuffer::new(self.cols, self.rows);
                } else {
                    self.using_alternate = false;
                }
                self.active_screen_mut().mark_all_dirty();
            }
            1048 => {
                if enable {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            1049 => {
                if enable {
                    self.save_cursor();
                    self.using_alternate = true;
                    self.alternate_screen = ScreenBuffer::new(self.cols, self.rows);
                    self.alternate_cursor = CursorState::default();
                } else {
                    self.using_alternate = false;
                    self.restore_cursor();
                }
                self.active_screen_mut().mark_all_dirty();
            }
            1004 => self.modes.focus_reporting = enable,
            2004 => self.modes.bracketed_paste = enable,
            9001 => self.modes.win32_input = enable,
            2026 => self.modes.synchronized_output = enable,
            
            // Mouse tracking modes
            1000 => self.modes.mouse_tracking = enable,
            1002 => self.modes.mouse_button_tracking = enable,
            1003 => self.modes.mouse_any_event = enable,
            1006 => self.modes.mouse_sgr_mode = enable,
            1015 => self.modes.mouse_urxvt_mode = enable,
            
            _ => {} // Ignore unknown modes
        }
    }

    /// Reverse index - cursor up, scroll if at top
    pub fn reverse_index(&mut self) {
        let cursor_row = self.active_cursor().row;
        let scroll_top = self.scroll_region.0;

        if cursor_row == scroll_top {
            self.scroll_down(1);
        } else {
            self.cursor_up(1);
        }
    }

    /// Index - cursor down, scroll if at bottom
    pub fn index(&mut self) {
        self.linefeed();
    }

    /// Start text selection
    pub fn start_selection(&mut self, col: u16, row: u16) {
        // Convert screen row to absolute buffer row
        let screen = self.active_screen();
        let abs_row = screen.screen_to_buffer_row(row as usize);
        
        self.selection = Some(Selection {
            start: (col, abs_row),
            end: (col, abs_row),
            active: true,
        });
        self.active_screen_mut().mark_all_dirty();
    }

    /// Update selection end point
    pub fn update_selection(&mut self, col: u16, row: u16) {
        // Convert screen row to absolute buffer row first
        let abs_row = self.active_screen().screen_to_buffer_row(row as usize);
        
        if let Some(ref mut sel) = self.selection {
            sel.end = (col, abs_row);
        }
        self.active_screen_mut().mark_all_dirty();
    }

    /// End selection (mouse released)
    pub fn end_selection(&mut self) {
        if let Some(ref mut sel) = self.selection {
            sel.active = false;
        }
    }

    /// Clear selection
    pub fn clear_selection(&mut self) {
        if self.selection.is_some() {
            self.selection = None;
            self.active_screen_mut().mark_all_dirty();
        }
    }

    /// Check if a cell is within the selection (screen coordinates)
    pub fn is_selected(&self, col: u16, screen_row: u16) -> bool {
        let sel = match &self.selection {
            Some(s) => s,
            None => return false,
        };

        // Convert screen row to absolute buffer row
        let screen = self.active_screen();
        let abs_row = screen.screen_to_buffer_row(screen_row as usize);

        // Normalize selection (start before end)
        let (start, end) = self.normalize_selection(sel);
        
        // Check if (col, abs_row) is within selection
        if abs_row < start.1 || abs_row > end.1 {
            return false;
        }
        
        if start.1 == end.1 {
            // Single line selection
            col >= start.0 && col <= end.0
        } else if abs_row == start.1 {
            // First line
            col >= start.0
        } else if abs_row == end.1 {
            // Last line
            col <= end.0
        } else {
            // Middle lines - fully selected
            true
        }
    }

    /// Normalize selection so start is before end
    fn normalize_selection(&self, sel: &Selection) -> ((u16, usize), (u16, usize)) {
        let start = sel.start;
        let end = sel.end;
        
        if start.1 < end.1 || (start.1 == end.1 && start.0 <= end.0) {
            (start, end)
        } else {
            (end, start)
        }
    }

    /// Get selected text
    pub fn get_selected_text(&self) -> Option<String> {
        let sel = self.selection.as_ref()?;
        let (start, end) = self.normalize_selection(sel);
        
        let screen = self.active_screen();
        let result = screen.collect_text_between(
            (start.1, start.0 as usize),
            (end.1, end.0 as usize),
        );
        
        if result.is_empty() {
            None
        } else {
            Some(result)
        }
    }
}

/// Screen buffer with scrollback
pub struct ScreenBuffer {
    /// Visible rows
    pub rows: Vec<Row>,
    /// Scrollback history
    pub scrollback: VecDeque<Row>,
    /// Maximum scrollback lines
    pub scrollback_limit: usize,
    /// Current scroll offset (0 = at bottom, >0 = scrolled up)
    pub scroll_offset: usize,
    dirty_lines: Vec<bool>,
    pub full_redraw: bool,
}

pub struct LogicalLineView<'a> {
    screen: &'a ScreenBuffer,
    start_abs_row: usize,
    end_abs_row: usize,
}

impl<'a> LogicalLineView<'a> {
    // Phase 2 introduces these accessors ahead of broader call-site migration.
    #[allow(dead_code)]
    pub fn start_abs_row(&self) -> usize {
        self.start_abs_row
    }

    #[allow(dead_code)]
    pub fn end_abs_row(&self) -> usize {
        self.end_abs_row
    }

    #[allow(dead_code)]
    pub fn rows(&self) -> impl Iterator<Item = &'a Row> + '_ {
        (self.start_abs_row..=self.end_abs_row)
            .filter_map(|abs_row| self.screen.get_row_absolute(abs_row))
    }

    pub fn text(&self) -> String {
        self.screen
            .collect_text_between((self.start_abs_row, 0), (self.end_abs_row, usize::MAX))
    }
}

impl ScreenBuffer {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            rows: (0..rows).map(|_| Row::new(cols)).collect(),
            scrollback: VecDeque::new(),
            scrollback_limit: 10000,
            scroll_offset: 0,
            dirty_lines: vec![false; rows as usize],
            full_redraw: true,
        }
    }

    pub fn resize(&mut self, new_cols: u16, new_rows: u16) {
        while self.rows.len() < new_rows as usize {
            self.rows.push(Row::new(new_cols));
        }
        self.rows.truncate(new_rows as usize);

        for row in &mut self.rows {
            row.resize(new_cols);
        }

        // Also resize scrollback rows
        for row in &mut self.scrollback {
            row.resize(new_cols);
        }

        self.dirty_lines.resize(new_rows as usize, false);
        self.mark_all_dirty();
    }

    pub(crate) fn apply_resize_plan(
        &mut self,
        mut plan: ScreenResizePlan,
        new_cols: u16,
        new_rows: u16,
    ) {
        if plan.scrollback.len() > self.scrollback_limit {
            let overflow = plan.scrollback.len() - self.scrollback_limit;
            plan.scrollback.drain(..overflow);
        }

        self.rows = plan.rows;
        self.scrollback = plan.scrollback;
        self.scroll_offset = plan.scroll_offset.min(self.scrollback.len());
        while self.rows.len() < new_rows as usize {
            self.rows.push(Row::new(new_cols));
        }
        self.rows.truncate(new_rows as usize);
        self.dirty_lines.resize(new_rows as usize, false);
        self.mark_all_dirty();
    }

    /// Add a row to scrollback when scrolling up
    pub fn push_to_scrollback(&mut self, row: Row) {
        self.scrollback.push_back(row);
        // Trim if exceeding limit
        if self.scrollback.len() > self.scrollback_limit {
            self.scrollback.pop_front();
        }
    }

    /// Get the total number of lines (scrollback + visible)
    #[allow(dead_code)]
    pub fn total_lines(&self) -> usize {
        self.scrollback.len() + self.rows.len()
    }

    /// Get a row at the given position (accounting for scroll offset)
    pub fn get_row_at(&self, visible_row: usize) -> Option<&Row> {
        if self.scroll_offset == 0 {
            // Not scrolled, return from visible rows
            self.rows.get(visible_row)
        } else {
            // Scrolled up, calculate position in history
            let total_scrollback = self.scrollback.len();
            let start_in_scrollback = total_scrollback.saturating_sub(self.scroll_offset);
            let absolute_row = start_in_scrollback + visible_row;

            if absolute_row < total_scrollback {
                self.scrollback.get(absolute_row)
            } else {
                self.rows.get(absolute_row - total_scrollback)
            }
        }
    }

    /// Scroll view up by n lines
    pub fn scroll_view_up(&mut self, n: usize) {
        let max_offset = self.scrollback.len();
        self.scroll_offset = (self.scroll_offset + n).min(max_offset);
        self.mark_all_dirty();
    }

    /// Scroll view down by n lines
    pub fn scroll_view_down(&mut self, n: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(n);
        self.mark_all_dirty();
    }

    /// Convert screen row to absolute buffer row
    pub fn screen_to_buffer_row(&self, screen_row: usize) -> usize {
        let total_scrollback = self.scrollback.len();
        let start_in_scrollback = total_scrollback.saturating_sub(self.scroll_offset);
        start_in_scrollback + screen_row
    }

    /// Get a row by absolute buffer position (0 = first scrollback line)
    pub fn get_row_absolute(&self, abs_row: usize) -> Option<&Row> {
        let total_scrollback = self.scrollback.len();
        if abs_row < total_scrollback {
            self.scrollback.get(abs_row)
        } else {
            self.rows.get(abs_row - total_scrollback)
        }
    }

    /// Reset scroll to bottom (live view)
    pub fn scroll_to_bottom(&mut self) {
        if self.scroll_offset != 0 {
            self.scroll_offset = 0;
            self.mark_all_dirty();
        }
    }

    /// Check if currently scrolled up
    pub fn is_scrolled(&self) -> bool {
        self.scroll_offset > 0
    }

    /// Convert visible row to absolute row in buffer
    pub fn visible_row_to_absolute(&self, visible_row: u16) -> usize {
        let total_scrollback = self.scrollback.len();
        let start_in_scrollback = total_scrollback.saturating_sub(self.scroll_offset);
        start_in_scrollback + visible_row as usize
    }

    pub fn logical_line_bounds(&self, abs_row: usize) -> Option<(usize, usize)> {
        self.get_row_absolute(abs_row)?;

        let mut start = abs_row;
        while start > 0 {
            let prev = self.get_row_absolute(start - 1)?;
            if !prev.wrapped {
                break;
            }
            start -= 1;
        }

        let mut end = abs_row;
        while let Some(row) = self.get_row_absolute(end) {
            if !row.wrapped {
                break;
            }
            end += 1;
            if self.get_row_absolute(end).is_none() {
                end -= 1;
                break;
            }
        }

        Some((start, end))
    }

    pub fn logical_line_at_absolute(&self, abs_row: usize) -> Option<LogicalLineView<'_>> {
        let (start_abs_row, end_abs_row) = self.logical_line_bounds(abs_row)?;
        Some(LogicalLineView {
            screen: self,
            start_abs_row,
            end_abs_row,
        })
    }

    pub fn logical_line_at_visible(&self, visible_row: usize) -> Option<LogicalLineView<'_>> {
        let abs_row = self.screen_to_buffer_row(visible_row);
        self.logical_line_at_absolute(abs_row)
    }

    /// Get line cells at absolute row position
    pub fn get_line_at_absolute(&self, abs_row: usize) -> Option<&Vec<Cell>> {
        self.get_row_absolute(abs_row).map(|r| &r.cells)
    }

    pub fn collect_text_between(&self, start: (usize, usize), end: (usize, usize)) -> String {
        let (start, end) = if start <= end { (start, end) } else { (end, start) };
        let mut result = String::new();
        let mut current_row = start.0;

        while current_row <= end.0 {
            let Some((_, logical_end)) = self.logical_line_bounds(current_row) else {
                break;
            };
            let segment_end = logical_end.min(end.0);
            let mut chunk = String::new();

            for abs_row in current_row..=segment_end {
                let Some(row) = self.get_row_absolute(abs_row) else {
                    continue;
                };
                let row_start = if abs_row == current_row {
                    if abs_row == start.0 { start.1 } else { 0 }
                } else {
                    0
                };
                let row_end = if abs_row == segment_end {
                    if abs_row == end.0 {
                        end.1.saturating_add(1)
                    } else {
                        row.cells.len()
                    }
                } else {
                    row.cells.len()
                };
                chunk.push_str(&row_text_range(row, row_start, row_end));
            }

            while chunk.ends_with(' ') {
                chunk.pop();
            }
            result.push_str(&chunk);

            if segment_end < end.0 {
                result.push('\n');
            }

            current_row = segment_end.saturating_add(1);
        }

        result
    }

    /// Simple character view of a cell (for searching/copying)
    #[allow(dead_code)]
    pub fn get_char_at(&self, abs_row: usize, col: usize) -> Option<char> {
        self.get_line_at_absolute(abs_row)
            .and_then(|cells| cells.get(col))
            .and_then(|cell| cell.grapheme.chars().next())
            .or(Some(' '))
    }

    pub fn mark_dirty(&mut self, line: usize) {
        if line < self.dirty_lines.len() {
            self.dirty_lines[line] = true;
        }
    }

    pub fn mark_all_dirty(&mut self) {
        self.full_redraw = true;
    }

    pub fn has_dirty_lines(&self) -> bool {
        self.dirty_lines.iter().any(|dirty| *dirty)
    }

    pub fn is_line_dirty(&self, line: usize) -> bool {
        self.dirty_lines.get(line).copied().unwrap_or(false)
    }

    pub fn dirty_line_indices(&self) -> impl Iterator<Item = usize> + '_ {
        self.dirty_lines.iter().enumerate().filter_map(|(idx, dirty)| {
            if *dirty {
                Some(idx)
            } else {
                None
            }
        })
    }

    pub fn clear_dirty(&mut self) {
        self.dirty_lines.fill(false);
        self.full_redraw = false;
    }
}

fn row_text_range(row: &Row, start_col: usize, end_col: usize) -> String {
    cells_text_range(&row.cells, start_col, end_col)
}

/// Text of `cells[start_col..end_col]` as the user would read it: wide-char
/// continuation cells contribute nothing (otherwise every CJK character
/// would be followed by a stray space), empty cells become a space, and the
/// full grapheme is kept so combining marks and multi-codepoint emoji
/// survive.
pub(crate) fn cells_text_range(cells: &[Cell], start_col: usize, end_col: usize) -> String {
    let mut text = String::new();
    for cell in cells.iter().take(end_col.min(cells.len())).skip(start_col) {
        if cell.is_continuation() {
            continue;
        }
        if cell.grapheme.is_empty() {
            text.push(' ');
        } else {
            text.push_str(&cell.grapheme);
        }
    }
    text
}

/// A single row
#[derive(Clone)]
pub struct Row {
    pub cells: Vec<Cell>,
    pub wrapped: bool,
}

impl Row {
    pub fn new(cols: u16) -> Self {
        Self {
            cells: vec![Cell::default(); cols as usize],
            wrapped: false,
        }
    }

    pub fn resize(&mut self, new_cols: u16) {
        self.cells.resize(new_cols as usize, Cell::default());
    }

    pub fn clear(&mut self, attrs: &CellAttrs) {
        for cell in &mut self.cells {
            cell.clear(attrs);
        }
        self.wrapped = false;
    }

    /// Restore the wide-char invariant — every width-2 cell is immediately
    /// followed by exactly one continuation cell — after an operation that
    /// erased or shifted an arbitrary cell range (EL/ECH/ICH/DCH). Orphaned
    /// halves become blanks; left in place they make the row's column
    /// accounting disagree with the visible glyphs, which shows up as gaps
    /// or overflow at pane boundaries.
    pub fn repair_wide_pairs(&mut self) {
        let len = self.cells.len();
        let mut i = 0;
        while i < len {
            if self.cells[i].width == 2 {
                if i + 1 < len && self.cells[i + 1].is_continuation() {
                    i += 2;
                    continue;
                }
                // A wide char in the last column legitimately has no
                // continuation (put_char defers edge handling to the host);
                // only a mid-row wide cell without one is an orphan.
                if i + 1 < len {
                    let attrs = self.cells[i].attrs.clone();
                    self.cells[i].clear(&attrs);
                }
            } else if self.cells[i].is_continuation() {
                // Continuation without a preceding wide char.
                let attrs = self.cells[i].attrs.clone();
                self.cells[i].clear(&attrs);
            }
            i += 1;
        }
    }
}

/// A single cell
#[derive(Clone)]
pub struct Cell {
    pub grapheme: String,
    pub width: u8,
    pub attrs: CellAttrs,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            grapheme: String::new(),
            width: 1,
            attrs: CellAttrs::default(),
        }
    }
}

impl Cell {
    pub fn clear(&mut self, attrs: &CellAttrs) {
        self.grapheme.clear();
        self.width = 1;
        self.attrs = attrs.clone();
        // Erased blanks must not stay clickable.
        self.attrs.hyperlink = None;
    }

    pub fn continuation(attrs: &CellAttrs) -> Self {
        Self {
            grapheme: String::new(),
            width: 0,
            attrs: attrs.clone(),
        }
    }

    pub fn is_continuation(&self) -> bool {
        self.width == 0
    }

    /// Get the first character (or space if empty)
    pub fn c(&self) -> char {
        self.grapheme.chars().next().unwrap_or(' ')
    }

    /// Get the display character (space if empty)
    pub fn display_char(&self) -> &str {
        if self.grapheme.is_empty() {
            " "
        } else {
            &self.grapheme
        }
    }
}

/// OSC 8 hyperlink target. Cells written while a link is open share one
/// allocation via `Arc`, so the per-cell cost is a single pointer.
#[derive(Debug, PartialEq)]
pub struct Hyperlink {
    /// Optional `id=` parameter; terminals use it to treat separate
    /// segments (e.g. a link split across wrapped lines) as one link.
    pub id: Option<String>,
    pub uri: String,
}

/// Cell attributes
#[derive(Clone, Default, PartialEq)]
pub struct CellAttrs {
    pub fg: Color,
    pub bg: Color,
    pub flags: AttrFlags,
    /// Shape of the underline while `AttrFlags::UNDERLINE` is set
    /// (SGR `4:1`..`4:5`).
    pub underline_style: UnderlineStyle,
    /// Underline color (SGR 58 / 59). `Color::Default` follows the
    /// foreground color, matching kitty/xterm semantics.
    pub underline_color: Color,
    /// Active OSC 8 hyperlink, if any.
    pub hyperlink: Option<std::sync::Arc<Hyperlink>>,
}

/// Extended underline styles (kitty's `4:x` SGR subparameters, adopted by
/// xterm, Windows Terminal and WezTerm).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum UnderlineStyle {
    #[default]
    Single,
    Double,
    Curly,
    Dotted,
    Dashed,
}

impl CellAttrs {
    pub fn reset(&mut self) {
        // SGR 0 must not terminate an OSC 8 hyperlink — links are only
        // closed by `OSC 8 ; ; ST` (they are orthogonal to SGR state).
        let hyperlink = self.hyperlink.take();
        *self = Self::default();
        self.hyperlink = hyperlink;
    }
}

/// Color definition
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl Color {
    /// Convert to crossterm color
    #[allow(dead_code)]
    pub fn to_crossterm(&self, _is_fg: bool) -> crossterm::style::Color {
        match self {
            Color::Default => crossterm::style::Color::Reset,
            Color::Indexed(n) => crossterm::style::Color::AnsiValue(*n),
            Color::Rgb(r, g, b) => crossterm::style::Color::Rgb {
                r: *r,
                g: *g,
                b: *b,
            },
        }
    }
}

bitflags! {
    #[derive(Clone, Copy, Default, PartialEq)]
    pub struct AttrFlags: u16 {
        const BOLD          = 0b0000_0000_0001;
        const DIM           = 0b0000_0000_0010;
        const ITALIC        = 0b0000_0000_0100;
        const UNDERLINE     = 0b0000_0000_1000;
        const BLINK         = 0b0000_0001_0000;
        const INVERSE       = 0b0000_0010_0000;
        const HIDDEN        = 0b0000_0100_0000;
        const STRIKETHROUGH = 0b0000_1000_0000;
    }
}

/// Cursor shape
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CursorShape {
    /// Default (terminal dependent)
    Default,
    /// Blinking block
    BlinkingBlock,
    /// Steady block
    SteadyBlock,
    /// Blinking underline
    BlinkingUnderline,
    /// Steady underline
    SteadyUnderline,
    /// Blinking bar (|)
    BlinkingBar,
    /// Steady bar (|)
    SteadyBar,
}

impl Default for CursorShape {
    fn default() -> Self {
        Self::BlinkingBlock  // デフォルトをブリンクブロックに
    }
}

impl CursorShape {
    /// Convert to DECSCUSR parameter (for \x1b[N q sequence)
    pub fn to_decscusr(&self) -> u8 {
        match self {
            CursorShape::Default => 0,
            CursorShape::BlinkingBlock => 1,
            CursorShape::SteadyBlock => 2,
            CursorShape::BlinkingUnderline => 3,
            CursorShape::SteadyUnderline => 4,
            CursorShape::BlinkingBar => 5,
            CursorShape::SteadyBar => 6,
        }
    }

    /// Create from DECSCUSR parameter
    pub fn from_decscusr(n: u8) -> Self {
        match n {
            0 => CursorShape::Default,
            1 => CursorShape::BlinkingBlock,
            2 => CursorShape::SteadyBlock,
            3 => CursorShape::BlinkingUnderline,
            4 => CursorShape::SteadyUnderline,
            5 => CursorShape::BlinkingBar,
            6 => CursorShape::SteadyBar,
            _ => CursorShape::Default,
        }
    }
}

/// Cursor state
#[derive(Clone)]
pub struct CursorState {
    pub col: u16,
    pub row: u16,
    pub visible: bool,
    pub shape: CursorShape,
    pub saved: Option<SavedCursor>,
}

impl Default for CursorState {
    fn default() -> Self {
        Self {
            col: 0,
            row: 0,
            visible: true,
            shape: CursorShape::Default,
            saved: None,
        }
    }
}

/// Saved cursor state
#[derive(Clone)]
pub struct SavedCursor {
    pub col: u16,
    pub row: u16,
    pub attrs: CellAttrs,
}

/// Terminal modes
#[derive(Clone, PartialEq, Eq)]
pub struct TerminalModes {
    pub application_cursor: bool,
    #[allow(dead_code)]
    pub application_keypad: bool,
    pub auto_wrap: bool,
    #[allow(dead_code)]
    pub origin_mode: bool,
    pub insert_mode: bool,
    pub linefeed_newline: bool,
    pub bracketed_paste: bool,
    /// 1004 - Focus reporting (CSI I on focus in, CSI O on focus out)
    pub focus_reporting: bool,
    /// 9001 - win32-input-mode: the child's conhost asked for key input as
    /// full Win32 key records (`CSI Vk;Sc;Uc;Kd;Cs;Rc _`), which preserves
    /// modifier state legacy VT cannot express (e.g. Shift+Enter)
    pub win32_input: bool,
    /// 2026 - synchronized output: the child is mid-frame and the pane must
    /// keep showing its last complete frame (`Session` holds renders and
    /// releases them on reset, timeout, resize or exit)
    pub synchronized_output: bool,
    
    // Mouse tracking modes
    /// 1000 - X10 mouse reporting (click only)
    pub mouse_tracking: bool,
    /// 1002 - Button event mouse tracking (click + drag)
    pub mouse_button_tracking: bool,
    /// 1003 - Any event mouse tracking (all movements)
    pub mouse_any_event: bool,
    /// 1006 - SGR extended mouse mode (allows coordinates > 223)
    pub mouse_sgr_mode: bool,
    /// 1015 - URXVT mouse mode (decimal format)
    pub mouse_urxvt_mode: bool,
}

impl Default for TerminalModes {
    fn default() -> Self {
        Self {
            application_cursor: false,
            application_keypad: false,
            auto_wrap: true, // Usually enabled by default
            origin_mode: false,
            insert_mode: false,
            linefeed_newline: false,
            bracketed_paste: false,
            focus_reporting: false,
            win32_input: false,
            synchronized_output: false,
            mouse_tracking: false,
            mouse_button_tracking: false,
            mouse_any_event: false,
            mouse_sgr_mode: false,
            mouse_urxvt_mode: false,
        }
    }
}

impl TerminalModes {
    /// Returns true if any mouse tracking mode is enabled
    pub fn mouse_enabled(&self) -> bool {
        self.mouse_tracking || self.mouse_button_tracking || self.mouse_any_event
    }
}

// =============================================================================
// Shell Integration (OSC 133 / OSC 633)
// =============================================================================

/// Shell integration state, populated by OSC 133 / OSC 633 sequences.
///
/// ## How it works
///
/// Modern shells (PowerShell, bash, zsh, fish) emit OSC escape sequences that
/// mark the boundaries of prompts and commands:
///
/// ```text
/// ESC ] 133 ; A ST   ← prompt starts being drawn
/// ESC ] 133 ; B ST   ← prompt finished; cursor is now at command start
/// ESC ] 133 ; C ST   ← user pressed Enter; command is now executing
/// ESC ] 133 ; D ; N ST ← command finished; N = exit code
/// ```
///
/// OSC 633 is VS Code's extension of OSC 133, used by PowerShell's built-in
/// shell integration (`$env:TERM_PROGRAM = "vscode"`).
///
/// ## Fallback
///
/// When no OSC markers have been seen (`active == false`), wtmux falls back to
/// keystroke tracking (`KeystrokeTracker`) which intercepts every key before
/// it is sent to the PTY.
#[derive(Clone, Debug, Default)]
pub struct ShellIntegration {
    /// True once at least one OSC 133/633 marker has been received.
    /// Used to decide whether to use OSC data or the keystroke fallback.
    pub active: bool,

    /// Row at which the prompt starts being drawn (recorded on marker A).
    /// With multi-line prompts this is above `prompt_end_row`.
    pub prompt_start_row: Option<u16>,

    /// Column at which user input starts (recorded on marker B).
    pub prompt_end_col: Option<u16>,
    /// Row at which user input starts (recorded on marker B).
    pub prompt_end_row: Option<u16>,

    /// The command that was confirmed by marker C (Enter pressed).
    /// Extracted from the screen buffer between prompt_end and cursor at
    /// the time the C marker arrives.
    pub confirmed_command: Option<String>,

    /// Exit code of the most recently completed command (from marker D).
    pub last_exit_code: Option<i32>,
}

impl ShellIntegration {
    /// Called when OSC 133;A or 633;A is received (prompt start).
    /// Records where the prompt begins and clears the previous confirmed
    /// command so a fresh one can be captured.
    pub fn on_prompt_start(&mut self, row: u16) {
        self.confirmed_command = None;
        self.prompt_start_row = Some(row);
    }

    /// Called when OSC 133;B or 633;B is received (prompt end = input start).
    /// Records cursor position so we know where the command text begins.
    pub fn on_prompt_end(&mut self, col: u16, row: u16) {
        self.active = true;
        self.prompt_end_col = Some(col);
        self.prompt_end_row = Some(row);
    }

    /// Called when OSC 133;C or 633;C is received (Enter pressed).
    /// `command` is the text extracted from the screen buffer.
    pub fn on_command_start(&mut self, command: String) {
        self.active = true;
        self.confirmed_command = Some(command);
    }

    /// Called when OSC 133;D or 633;D is received (command finished).
    pub fn on_command_done(&mut self, exit_code: Option<i32>) {
        self.last_exit_code = exit_code;
    }

    /// Take the confirmed command (consumes it so it is only used once).
    pub fn take_confirmed_command(&mut self) -> Option<String> {
        self.confirmed_command.take()
    }
}

// =============================================================================
// Keystroke Tracker (fallback for shells without OSC 133/633)
// =============================================================================

/// Tracks the current command-line input by intercepting keystrokes.
///
/// This is used as a fallback when the shell does not emit OSC 133/633
/// markers (e.g. cmd.exe).  It maintains a best-effort buffer of the text
/// the user has typed since the last Enter / Ctrl+C / Ctrl+U.
///
/// Limitations:
/// - Does not handle readline-style cursor movement (←→ for insert)
/// - Ctrl+W (delete word) is approximated but may be slightly off
/// - Multi-line commands are not tracked
///
/// Despite these limitations it is far more accurate than `strip_prompt`
/// because it never needs to parse the prompt at all.
#[derive(Clone, Debug, Default)]
pub struct KeystrokeTracker {
    /// The accumulated input since the last reset.
    pub buf: String,
}

impl KeystrokeTracker {
    /// Record a printable character being typed.
    pub fn push_char(&mut self, ch: char) {
        self.buf.push(ch);
    }

    /// Handle Backspace (0x08 / 0x7F).
    pub fn backspace(&mut self) {
        self.buf.pop();
    }

    /// Handle Ctrl+W (delete last word).
    pub fn delete_word(&mut self) {
        // Trim trailing spaces then delete back to next space
        let trimmed = self.buf.trim_end_matches(' ');
        let new_len = trimmed.rfind(' ').map(|i| i + 1).unwrap_or(0);
        self.buf.truncate(new_len);
    }

    /// Handle Ctrl+U (delete to start of line).
    pub fn clear_line(&mut self) {
        self.buf.clear();
    }

    /// Take the current buffer as a command and reset for the next input.
    #[allow(dead_code)]
    pub fn take(&mut self) -> String {
        let cmd = self.buf.trim().to_string();
        self.buf.clear();
        cmd
    }

    /// Peek at the current buffer without consuming.
    pub fn peek(&self) -> &str {
        self.buf.trim_end()
    }
}

#[cfg(test)]
mod tests {
    use super::TerminalState;
    use crate::core::term::resize::ResizePolicy;

    fn row_text(state: &TerminalState, row_idx: usize) -> String {
        let Some(row) = state.active_screen().rows.get(row_idx) else {
            return String::new();
        };

        let mut text = String::new();
        for cell in &row.cells {
            if cell.is_continuation() {
                continue;
            }
            text.push_str(cell.display_char());
        }
        text.trim_end().to_string()
    }

    fn logical_lines(state: &TerminalState) -> Vec<String> {
        let screen = state.active_screen();
        let mut lines = Vec::new();
        let mut current = String::new();

        for abs_row in 0..screen.total_lines() {
            let row = screen.get_row_absolute(abs_row).unwrap();
            for cell in &row.cells {
                if cell.is_continuation() {
                    continue;
                }
                current.push_str(cell.display_char());
            }

            if !row.wrapped {
                lines.push(current.trim_end().to_string());
                current.clear();
            }
        }

        if !current.is_empty() {
            lines.push(current.trim_end().to_string());
        }

        lines
    }

    fn visible_row_text(state: &TerminalState, row_idx: usize) -> String {
        let Some(row) = state.active_screen().get_row_at(row_idx) else {
            return String::new();
        };

        let mut text = String::new();
        for cell in &row.cells {
            if cell.is_continuation() {
                continue;
            }
            text.push_str(cell.display_char());
        }
        text.trim_end().to_string()
    }

    #[test]
    fn resize_reflows_back_when_growing() {
        let mut state = TerminalState::new(10, 4);
        for ch in "abcdefghijKLM".chars() {
            state.put_char(ch);
        }

        state.resize(6, 4);
        assert_eq!(row_text(&state, 0), "abcdef");
        assert_eq!(row_text(&state, 1), "ghijKL");
        assert_eq!(row_text(&state, 2), "M");

        state.resize(10, 4);
        assert_eq!(row_text(&state, 0), "abcdefghij");
        assert_eq!(row_text(&state, 1), "KLM");
    }

    #[test]
    fn resize_preserves_hard_line_breaks() {
        let mut state = TerminalState::new(10, 4);
        for ch in "hello".chars() {
            state.put_char(ch);
        }
        state.carriage_return();
        state.linefeed();
        for ch in "world".chars() {
            state.put_char(ch);
        }

        state.resize(3, 4);
        state.resize(10, 4);

        assert_eq!(row_text(&state, 0), "hello");
        assert_eq!(row_text(&state, 1), "world");
    }

    /// With OSC 133 markers placing the cursor on the input line, the
    /// prompt + input rows must be carried through a resize physically
    /// (truncated, not rewrapped): the shell repaints them on SIGWINCH with
    /// cursor-relative sequences, which only line up if these rows keep
    /// their layout. Rows above still reflow.
    #[test]
    fn resize_pins_prompt_and_input_rows_when_at_prompt() {
        let mut state = TerminalState::new(10, 6);
        for ch in "OUTPUTLINE".chars() {
            state.put_char(ch);
        }
        state.carriage_return();
        state.linefeed();

        let prompt_row = state.active_cursor().row;
        state.shell_integration.on_prompt_start(prompt_row);
        for ch in "PROMPT1".chars() {
            state.put_char(ch);
        }
        state.carriage_return();
        state.linefeed();
        for ch in "> ".chars() {
            state.put_char(ch);
        }
        let (input_col, input_row) = {
            let cursor = state.active_cursor();
            (cursor.col, cursor.row)
        };
        state.shell_integration.on_prompt_end(input_col, input_row);
        for ch in "abc".chars() {
            state.put_char(ch);
        }

        state.resize(6, 6);

        // The output line above the prompt reflows into two rows...
        assert_eq!(visible_row_text(&state, 0), "OUTPUT");
        assert_eq!(visible_row_text(&state, 1), "LINE");
        // ...while the prompt and input rows keep their physical layout
        // (the prompt row is truncated, not wrapped).
        assert_eq!(visible_row_text(&state, 2), "PROMPT");
        assert_eq!(visible_row_text(&state, 3), "> abc");
        assert_eq!(
            (state.primary_cursor.row, state.primary_cursor.col),
            (3, 5),
            "cursor keeps its offset within the pinned region"
        );
        assert_eq!(state.shell_integration.prompt_start_row, Some(2));
        assert_eq!(state.shell_integration.prompt_end_row, Some(3));
        assert_eq!(state.shell_integration.prompt_end_col, Some(2));
    }

    /// Without OSC 133 markers there is no guarantee anything repaints the
    /// cursor row after a resize, so it must keep rewrapping as before.
    #[test]
    fn resize_without_prompt_markers_still_reflows_cursor_line() {
        let mut state = TerminalState::new(10, 4);
        for ch in "abcdefghijKL".chars() {
            state.put_char(ch);
        }

        state.resize(6, 4);
        assert_eq!(row_text(&state, 0), "abcdef");
        assert_eq!(row_text(&state, 1), "ghijKL");
    }

    #[test]
    fn erase_in_line_repairs_split_wide_char() {
        let mut state = TerminalState::new(10, 4);
        for ch in "日本".chars() {
            state.put_char(ch);
        }

        // Land the cursor on 日's continuation cell (col 1), then erase to
        // end of line — the erase wipes the continuation but not the wide
        // half, which must be repaired to a blank. Left as width 2 it makes
        // the row's column accounting disagree with the visible glyphs.
        state.active_cursor_mut().col = 1;
        state.erase_in_line(0);

        let row = &state.active_screen().rows[0];
        assert_eq!(row.cells[0].width, 1);
        assert!(row.cells[0].grapheme.is_empty());
        assert!(row.cells.iter().all(|c| !c.is_continuation()));
    }

    #[test]
    fn zero_width_char_after_wide_char_joins_the_lead_cell() {
        // 👨 (cols 0-1) followed by ZWJ: the joiner must attach to the lead
        // cell's grapheme, not to the continuation cell (where it would be
        // invisible to rendering and copy mode).
        let mut state = TerminalState::new(10, 2);
        state.put_char('\u{1F468}');
        state.put_char('\u{200D}');
        state.put_char('\u{1F469}');

        // ...and the emoji after the joiner continues the same cluster: one
        // 2-cell grapheme, exactly as Windows Terminal, WezTerm and the
        // applications' width libraries lay it out.
        let row = &state.active_screen().rows[0];
        assert_eq!(row.cells[0].grapheme, "\u{1F468}\u{200D}\u{1F469}");
        assert_eq!(row.cells[0].width, 2);
        assert!(row.cells[1].is_continuation());
        assert!(row.cells[1].grapheme.is_empty());
        assert!(row.cells[2].grapheme.is_empty());
        assert_eq!(state.active_cursor().col, 2);
    }

    #[test]
    fn zwj_family_occupies_two_cells_and_text_continues_after_it() {
        let mut state = TerminalState::new(10, 2);
        for ch in "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}x".chars() {
            state.put_char(ch);
        }
        let row = &state.active_screen().rows[0];
        assert_eq!(row.cells[0].grapheme, "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}");
        assert!(row.cells[1].is_continuation());
        assert_eq!(row.cells[2].grapheme, "x");
        assert_eq!(state.active_cursor().col, 3);
    }

    #[test]
    fn zwj_join_at_the_right_margin_stays_in_the_lead_cell() {
        // 👨‍ fills the last two columns (cursor in the pending-wrap position).
        // The continuation of the cluster still belongs to that cell — a
        // terminal never wraps in the middle of a grapheme cluster.
        let mut state = TerminalState::new(4, 2);
        for ch in "ab\u{1F468}\u{200D}".chars() {
            state.put_char(ch);
        }
        state.put_char('\u{1F469}');
        let screen = state.active_screen();
        assert_eq!(screen.rows[0].cells[2].grapheme, "\u{1F468}\u{200D}\u{1F469}");
        assert!(screen.rows[1].cells[0].grapheme.is_empty());
        assert_eq!(state.active_cursor().col, 4);
        // ...and the next real character wraps as usual.
        state.put_char('x');
        assert_eq!(state.active_screen().rows[1].cells[0].grapheme, "x");
    }

    #[test]
    fn vs16_widens_text_default_emoji_only_when_host_does() {
        // Host draws ❤️ in one cell (WezTerm 1.22): selector is a plain mark.
        let mut state = TerminalState::new(10, 2);
        state.vs16_emoji_wide = Some(false);
        for ch in "\u{2764}\u{FE0F}x".chars() {
            state.put_char(ch);
        }
        {
            let row = &state.active_screen().rows[0];
            assert_eq!(row.cells[0].grapheme, "\u{2764}\u{FE0F}");
            assert_eq!(row.cells[0].width, 1);
            assert_eq!(row.cells[1].grapheme, "x");
        }

        // Host draws ❤️ in two cells (Windows Terminal 1.24): widen the lead.
        let mut state = TerminalState::new(10, 2);
        state.vs16_emoji_wide = Some(true);
        for ch in "\u{2764}\u{FE0F}x".chars() {
            state.put_char(ch);
        }
        {
            let row = &state.active_screen().rows[0];
            assert_eq!(row.cells[0].grapheme, "\u{2764}\u{FE0F}");
            assert_eq!(row.cells[0].width, 2);
            assert!(row.cells[1].is_continuation());
            assert_eq!(row.cells[2].grapheme, "x");
            assert_eq!(state.active_cursor().col, 3);
        }

        // Not a text-default emoji: 'a' + VS16 and 😀 + VS16 stay as they are.
        let mut state = TerminalState::new(10, 2);
        state.vs16_emoji_wide = Some(true);
        for ch in "a\u{FE0F}\u{1F600}\u{FE0F}x".chars() {
            state.put_char(ch);
        }
        {
            let row = &state.active_screen().rows[0];
            assert_eq!(row.cells[0].grapheme, "a\u{FE0F}");
            assert_eq!(row.cells[0].width, 1);
            assert_eq!(row.cells[1].grapheme, "\u{1F600}\u{FE0F}");
            assert_eq!(row.cells[1].width, 2);
            assert_eq!(row.cells[3].grapheme, "x");
        }
    }

    #[test]
    fn vs16_widening_at_right_margin_keeps_one_cell() {
        let mut state = TerminalState::new(3, 2);
        state.vs16_emoji_wide = Some(true);
        for ch in "ab\u{2764}\u{FE0F}".chars() {
            state.put_char(ch);
        }
        let row = &state.active_screen().rows[0];
        assert_eq!(row.cells[2].grapheme, "\u{2764}\u{FE0F}");
        assert_eq!(row.cells[2].width, 1);
        assert_eq!(state.active_cursor().col, 3);
    }

    #[test]
    fn halfwidth_kana_and_dakuten_form_one_two_cell_cluster() {
        let mut state = TerminalState::new(10, 2);
        for ch in "\u{FF76}\u{FF9E}x".chars() {
            state.put_char(ch);
        }
        let row = &state.active_screen().rows[0];
        assert_eq!(row.cells[0].grapheme, "\u{FF76}\u{FF9E}");
        assert_eq!(row.cells[0].width, 2);
        assert!(row.cells[1].is_continuation());
        assert_eq!(row.cells[2].grapheme, "x");
        assert_eq!(state.active_cursor().col, 3);

        // A dakuten with no halfwidth kana before it stands in its own cell.
        let mut state = TerminalState::new(10, 2);
        for ch in "a\u{FF9E}x".chars() {
            state.put_char(ch);
        }
        let row = &state.active_screen().rows[0];
        assert_eq!(row.cells[0].grapheme, "a");
        assert_eq!(row.cells[1].grapheme, "\u{FF9E}");
        assert_eq!(row.cells[2].grapheme, "x");
    }

    #[test]
    fn regional_indicator_pair_forms_one_flag_cluster() {
        let mut state = TerminalState::new(12, 2);
        // 🇯🇵 then 🇺🇸 then a lone 🇯 then x
        for ch in "\u{1F1EF}\u{1F1F5}\u{1F1FA}\u{1F1F8}\u{1F1EF}x".chars() {
            state.put_char(ch);
        }
        let row = &state.active_screen().rows[0];
        assert_eq!(row.cells[0].grapheme, "\u{1F1EF}\u{1F1F5}");
        assert_eq!(row.cells[0].width, 2);
        assert!(row.cells[1].is_continuation());
        assert_eq!(row.cells[2].grapheme, "\u{1F1FA}\u{1F1F8}");
        assert_eq!(row.cells[2].width, 2);
        assert_eq!(row.cells[4].grapheme, "\u{1F1EF}");
        assert_eq!(row.cells[4].width, 1);
        assert_eq!(row.cells[5].grapheme, "x");
        assert_eq!(state.active_cursor().col, 6);
    }

    #[test]
    fn wide_char_overwriting_next_wide_lead_blanks_its_orphaned_continuation() {
        // Old: 日 at cols 1-2. New: 語 written at cols 0-1 — its continuation
        // overwrites 日's lead, so 日's old continuation at col 2 must be
        // blanked, not left as an orphan (an orphan there renders as a stray
        // blank and corrupts later overwrites at that column).
        let mut state = TerminalState::new(10, 2);
        state.put_char(' ');
        state.put_char('日'); // cols 1-2
        state.active_cursor_mut().col = 0;
        state.put_char('語'); // cols 0-1

        let row = &state.active_screen().rows[0];
        assert_eq!(row.cells[0].grapheme, "語");
        assert!(row.cells[1].is_continuation());
        assert!(
            !row.cells[2].is_continuation(),
            "old wide char's continuation must be blanked, not orphaned"
        );
    }

    #[test]
    fn orphaned_continuation_overwrite_does_not_blank_unrelated_neighbor() {
        // cells: 日 at cols 0-1, then a fabricated orphaned continuation at
        // col 2 (as left behind by a partial rewrite). Writing at col 2 must
        // not blank col 1 — col 1 is 日's continuation, not the orphan's lead.
        let mut state = TerminalState::new(10, 2);
        state.put_char('日'); // cols 0-1
        state.active_screen_mut().rows[0].cells[2] =
            crate::core::term::Cell::continuation(&crate::core::term::CellAttrs::default());

        state.active_cursor_mut().col = 2;
        state.put_char('A');

        let row = &state.active_screen().rows[0];
        assert_eq!(row.cells[0].grapheme, "日");
        assert!(
            row.cells[1].is_continuation(),
            "日 must stay intact when the orphan next to it is overwritten"
        );
        assert_eq!(row.cells[2].grapheme, "A");
    }

    #[test]
    fn wide_char_landing_on_last_column_wraps_instead_of_being_dropped() {
        // cols=10: filling 9 narrow chars leaves the cursor at col 9 (last
        // column) right as a wide CJK char arrives — the exact edge case hit
        // repeatedly when wrapping a long CJK paragraph at an arbitrary pane
        // width. A wide char placed there has no room for its continuation
        // cell, and render_row_stream clips any cell that would cross the
        // right edge (to avoid bleeding into a neighboring pane) — so without
        // an early wrap here, the glyph is silently dropped from render.
        let mut state = TerminalState::new(10, 4);
        for ch in "123456789".chars() {
            state.put_char(ch);
        }
        state.put_char('日');
        state.put_char('K');

        let row0 = &state.active_screen().rows[0];
        assert_eq!(row0.cells[9].grapheme, "", "last column stays blank; the wide char wrapped to the next row");
        assert!(row0.wrapped);

        let row1 = &state.active_screen().rows[1];
        assert_eq!(row1.cells[0].grapheme, "日");
        assert!(row1.cells[1].is_continuation());
        assert_eq!(row1.cells[2].grapheme, "K");

        let mut out = Vec::new();
        {
            use std::io::Write as _;
            crate::ui::row_stream::render_row_stream(
                &mut out,
                crate::ui::row_stream::RenderRow::new(&row1.cells, 10),
                |_col_idx, _cell| (),
                |stdout, _style| write!(stdout, ""),
            )
            .expect("row stream renders");
        }

        assert_eq!(String::from_utf8(out).expect("utf8").trim_end(), "日K");
    }

    #[test]
    fn long_cjk_paragraph_survives_wrapping_without_dropping_characters() {
        // Regression test for the wtmux bug where wrapping a long CJK
        // paragraph (the じゅげむ tongue-twister) at a narrow pane width
        // dropped characters and left stray gaps — caused by a wide char
        // landing on the last column with nowhere for its continuation cell,
        // then getting clipped out entirely by render_row_stream's
        // right-edge guard. Exercised across several widths since the exact
        // column where a wide char lands on the edge shifts with the width.
        let text = "じゅげむ じゅげむ ごこうのすりきれ かいじゃりすいぎょの すいぎょうまつ うんらいまつ ふうらいまつ くうねるところに すむところ やぶらこうじの ぶらこうじ パイポパイポ パイポのシューリンガン シューリンガンのグーリンダイ グーリンダイのポンポコピーのポンポコナーの ちょうきゅうめいの ちょうすけ";

        for cols in [20u16, 24, 33, 40, 55, 80] {
            let mut state = TerminalState::new(cols, 60);
            for ch in text.chars() {
                state.put_char(ch);
            }

            let last_row = state.active_cursor().row as usize;
            let mut rendered = String::new();
            for row in &state.active_screen().rows[0..=last_row] {
                let mut out = Vec::new();
                {
                    use std::io::Write as _;
                    crate::ui::row_stream::render_row_stream(
                        &mut out,
                        crate::ui::row_stream::RenderRow::new(&row.cells, cols as usize),
                        |_col_idx, _cell| (),
                        |stdout, _style| write!(stdout, ""),
                    )
                    .expect("row stream renders");
                }
                rendered.push_str(&String::from_utf8(out).expect("utf8"));
            }

            let expected: String = text.chars().filter(|c| !c.is_whitespace()).collect();
            let actual: String = rendered.chars().filter(|c| !c.is_whitespace()).collect();
            assert_eq!(actual, expected, "character mismatch at cols={cols}");
        }
    }

    #[test]
    fn resize_preserves_cjk_without_inserting_spaces() {
        let mut state = TerminalState::new(20, 6);
        let text = "日本語の幅テストです";
        for ch in text.chars() {
            state.put_char(ch);
        }

        state.resize(9, 6);
        state.resize(20, 6);

        assert_eq!(row_text(&state, 0), text);
    }

    #[test]
    fn resize_preserves_mixed_ascii_and_cjk_line() {
        let mut state = TerminalState::new(96, 8);
        let text = "-rw-r--r-- 1 n_fuk users 11021 Apr 21 07:02 キューバのロシア産原油受け入れの背後にあるもの.md";
        for ch in text.chars() {
            state.put_char(ch);
        }

        state.resize(54, 8);
        state.resize(96, 8);

        assert_eq!(row_text(&state, 0), text);
    }

    #[test]
    fn resize_preserves_scrollback_mixed_ascii_and_cjk_line() {
        let mut state = TerminalState::new(96, 4);
        let target = "-rw-r--r-- 1 n_fuk users 11021 Apr 21 07:02 キューバのロシア産原油受け入れの背後にあるもの.md";

        for line in ["line1", "line2", target, "line4", "line5", "line6"] {
            for ch in line.chars() {
                state.put_char(ch);
            }
            state.carriage_return();
            state.linefeed();
        }

        state.resize(54, 4);
        state.resize(96, 4);

        assert!(logical_lines(&state).iter().any(|line| line == target));
    }

    #[test]
    fn logical_line_view_merges_wrapped_rows() {
        let mut state = TerminalState::new(6, 4);
        for ch in "abcdefghi".chars() {
            state.put_char(ch);
        }

        let screen = state.active_screen();
        let logical = screen.logical_line_at_absolute(0).unwrap();
        assert_eq!(logical.start_abs_row(), 0);
        assert_eq!(logical.end_abs_row(), 1);
        assert_eq!(logical.rows().count(), 2);
        assert_eq!(logical.text(), "abcdefghi");
    }

    #[test]
    fn collect_text_between_only_breaks_on_logical_boundaries() {
        let mut state = TerminalState::new(6, 5);
        for ch in "abcdefghi".chars() {
            state.put_char(ch);
        }
        state.carriage_return();
        state.linefeed();
        for ch in "xyz".chars() {
            state.put_char(ch);
        }

        let screen = state.active_screen();
        assert_eq!(screen.collect_text_between((0, 0), (1, 2)), "abcdefghi");
        assert_eq!(screen.collect_text_between((0, 0), (2, 2)), "abcdefghi\nxyz");
    }

    #[test]
    fn resize_preserves_scrolled_view_anchor() {
        let mut state = TerminalState::new(8, 4);

        for line in ["line01", "line02", "line03", "line04", "line05", "line06", "line07"] {
            for ch in line.chars() {
                state.put_char(ch);
            }
            state.carriage_return();
            state.linefeed();
        }

        state.primary_screen.scroll_view_up(2);
        let top_before = visible_row_text(&state, 0);

        state.resize(12, 6);

        assert_eq!(visible_row_text(&state, 0), top_before);
    }

    #[test]
    fn resize_keeps_scrollback_reachable() {
        let mut state = TerminalState::new(8, 4);

        for idx in 1..=12 {
            let line = format!("l{idx:02}");
            for ch in line.chars() {
                state.put_char(ch);
            }
            state.carriage_return();
            state.linefeed();
        }

        state.resize(12, 6);
        state.primary_screen.scroll_view_up(usize::MAX);

        assert!(state.primary_screen.is_scrolled());
        assert_eq!(visible_row_text(&state, 0), "l01");
    }

    #[test]
    fn host_driven_resize_preserves_scrolled_view_anchor() {
        let mut state = TerminalState::new(8, 4);

        for line in ["line01", "line02", "line03", "line04", "line05", "line06", "line07"] {
            for ch in line.chars() {
                state.put_char(ch);
            }
            state.carriage_return();
            state.linefeed();
        }

        state.primary_screen.scroll_view_up(2);
        let top_before = visible_row_text(&state, 0);

        state.resize_with_policy(12, 6, ResizePolicy::HostDriven);

        assert_eq!(visible_row_text(&state, 0), top_before);
    }

    #[test]
    fn host_driven_resize_preserves_total_line_count() {
        let mut state = TerminalState::new(8, 4);

        for idx in 1..=12 {
            let line = format!("l{idx:02}");
            for ch in line.chars() {
                state.put_char(ch);
            }
            state.carriage_return();
            state.linefeed();
        }

        let total_before = state.primary_screen.total_lines();
        state.resize_with_policy(12, 6, ResizePolicy::HostDriven);

        assert_eq!(state.primary_screen.total_lines(), total_before);
        state.primary_screen.scroll_view_up(usize::MAX);
        assert_eq!(visible_row_text(&state, 0), "l01");
    }

    // --- ConsoleBuffer policy: mirrors the console buffer measured on the
    // inbox conhost and OpenConsole (tools: D:\tmp\conpty_resize_probe.py) ---

    fn type_lines(state: &mut TerminalState, lines: usize, long_line_at: Option<usize>) {
        for i in 0..lines {
            let text = match long_line_at {
                Some(idx) if idx == i => "L".repeat(100),
                _ => format!("line {:02}", i),
            };
            for ch in text.chars() {
                state.put_char(ch);
            }
            state.carriage_return();
            state.linefeed();
        }
        for ch in "> ".chars() {
            state.put_char(ch);
        }
    }

    #[test]
    fn console_buffer_shrink_keeps_cursor_row_while_it_fits() {
        // Experiment B: 5 lines + prompt in 20 rows, cursor at row 5.
        let mut state = TerminalState::new(80, 20);
        type_lines(&mut state, 5, None);
        assert_eq!(state.primary_cursor.row, 5);

        state.resize_with_policy(80, 10, ResizePolicy::ConsoleBuffer);
        assert_eq!(state.primary_cursor.row, 5, "cursor still fits: row unchanged");
        assert_eq!(visible_row_text(&state, 0), "line 00");
        assert_eq!(state.primary_screen.scrollback.len(), 0);

        state.resize_with_policy(80, 3, ResizePolicy::ConsoleBuffer);
        assert_eq!(state.primary_cursor.row, 2, "only as many top rows leave as needed");
        assert_eq!(visible_row_text(&state, 0), "line 03");
        assert_eq!(visible_row_text(&state, 2), ">");
        assert_eq!(state.primary_screen.scrollback.len(), 3);

        state.resize_with_policy(80, 20, ResizePolicy::ConsoleBuffer);
        assert_eq!(state.primary_cursor.row, 2, "growing appends rows below, never pulls back");
        assert_eq!(visible_row_text(&state, 0), "line 03");
        assert_eq!(visible_row_text(&state, 3), "");
        assert_eq!(state.primary_screen.scrollback.len(), 3);
    }

    #[test]
    fn console_buffer_grow_after_shrink_keeps_prompt_row() {
        // Experiment A: 30 lines in 20 rows, 20 -> 10 -> 15 -> 20.
        let mut state = TerminalState::new(80, 20);
        type_lines(&mut state, 30, None);
        assert_eq!(state.primary_cursor.row, 19);

        state.resize_with_policy(80, 10, ResizePolicy::ConsoleBuffer);
        assert_eq!(state.primary_cursor.row, 9);
        assert_eq!(visible_row_text(&state, 0), "line 21");

        state.resize_with_policy(80, 15, ResizePolicy::ConsoleBuffer);
        assert_eq!(state.primary_cursor.row, 9);
        assert_eq!(visible_row_text(&state, 9), ">");
        assert_eq!(visible_row_text(&state, 10), "");

        state.resize_with_policy(80, 20, ResizePolicy::ConsoleBuffer);
        assert_eq!(state.primary_cursor.row, 9);
        assert_eq!(visible_row_text(&state, 0), "line 21");
    }

    #[test]
    fn console_buffer_width_change_rewraps_and_cursor_follows() {
        // Experiment C: 10 lines, the 3rd is 100 chars; 80 -> 40 -> 80 cols.
        let mut state = TerminalState::new(80, 20);
        type_lines(&mut state, 10, Some(2));
        assert_eq!(state.primary_cursor.row, 11);

        state.resize_with_policy(40, 20, ResizePolicy::ConsoleBuffer);
        assert_eq!(state.primary_cursor.row, 12);
        assert_eq!(state.primary_cursor.col, 2);
        assert_eq!(visible_row_text(&state, 2), "L".repeat(40));
        assert_eq!(visible_row_text(&state, 4), "L".repeat(20));
        assert_eq!(visible_row_text(&state, 12), ">");

        state.resize_with_policy(80, 20, ResizePolicy::ConsoleBuffer);
        assert_eq!(state.primary_cursor.row, 11);
        assert_eq!(visible_row_text(&state, 2), "L".repeat(80));
        assert_eq!(visible_row_text(&state, 11), ">");
    }

    /// "text0", then one logical line of `lead_x? + spaces + trail_x?`, then
    /// a prompt; returns the prompt's 1-based row before and after resizing
    /// to `new_cols` under ConsoleBuffer.
    fn space_line_prompt_rows(
        spaces: usize,
        lead_x: bool,
        trail_x: bool,
        new_cols: u16,
    ) -> (usize, usize) {
        let mut state = TerminalState::new(80, 24);
        for ch in "text0".chars() {
            state.put_char(ch);
        }
        state.carriage_return();
        state.linefeed();
        if lead_x {
            state.put_char('x');
        }
        for _ in 0..spaces {
            state.put_char(' ');
        }
        if trail_x {
            state.put_char('x');
        }
        state.carriage_return();
        state.linefeed();
        for ch in "$ prompt".chars() {
            state.put_char(ch);
        }
        let before = state.primary_cursor.row as usize + 1;
        state.resize_with_policy(new_cols, 24, ResizePolicy::ConsoleBuffer);
        (before, state.primary_cursor.row as usize + 1)
    }

    #[test]
    fn console_buffer_space_rows_follow_console_reflow() {
        // Measured 2026-10-01 on the inbox conhost and the bundled
        // OpenConsole (identical): a child that wrote runs of spaces, then
        // the window narrowed from 80 columns. Each case is
        // (spaces, lead_x, trail_x, new_cols, prompt row before, after).
        // The trailing spaces of a line's last row are not content (even
        // exactly 80 of them), the full rows before it are, and a vanished
        // tail ending exactly on the new width keeps one extra empty row.
        let cases = [
            (40, false, false, 40, 3, 3),
            (79, false, false, 40, 3, 3),
            (80, false, false, 40, 3, 3),
            (81, false, false, 40, 4, 5),
            (120, false, false, 40, 4, 5),
            (160, false, false, 40, 4, 5),
            (161, false, false, 40, 5, 7),
            (240, false, false, 40, 5, 7),
            (80, false, true, 40, 4, 5),
            (100, false, true, 40, 4, 5),
            (160, false, true, 40, 5, 7),
            (100, true, false, 40, 4, 5),
            (120, false, false, 30, 4, 5),
            (120, false, false, 20, 4, 7),
            (120, false, false, 60, 4, 4),
            (161, false, false, 30, 5, 8),
            (161, false, false, 20, 5, 11),
            (161, false, false, 60, 5, 5),
            (100, true, false, 30, 4, 5),
            (100, true, false, 20, 4, 7),
            (100, true, false, 60, 4, 4),
        ];
        for (spaces, lead_x, trail_x, new_cols, before, after) in cases {
            assert_eq!(
                space_line_prompt_rows(spaces, lead_x, trail_x, new_cols),
                (before, after),
                "{spaces} spaces (lead x {lead_x}, trail x {trail_x}) -> {new_cols} cols"
            );
        }
    }

    /// "ab", 40 spaces drawn with `bg`, CR LF, a prompt; narrowed from 80 to
    /// 20 columns under `policy`. Returns the prompt's row afterwards and
    /// whether any cell of the first line still carries the background.
    fn styled_space_line_after_narrowing(policy: ResizePolicy, bg: super::Color) -> (usize, bool) {
        let mut state = TerminalState::new(80, 24);
        for ch in "ab".chars() {
            state.put_char(ch);
        }
        state.current_attrs.bg = bg;
        for _ in 0..40 {
            state.put_char(' ');
        }
        state.current_attrs = Default::default();
        state.carriage_return();
        state.linefeed();
        for ch in "$ prompt".chars() {
            state.put_char(ch);
        }
        assert_eq!(state.primary_cursor.row, 1);
        state.resize_with_policy(20, 24, policy);
        let kept_bg = (0..3).any(|row| {
            state.active_screen().rows[row]
                .cells
                .iter()
                .any(|cell| cell.grapheme == " " && cell.attrs.bg != super::Color::Default)
        });
        (state.primary_cursor.row as usize, kept_bg)
    }

    #[test]
    fn local_reflow_keeps_background_coloured_trailing_spaces() {
        let red = super::Color::Indexed(1);
        // "ab" + 40 coloured spaces = 42 cells = 3 rows of 20; the prompt
        // moves from row 1 to row 3 and the colour is still there.
        assert_eq!(
            styled_space_line_after_narrowing(ResizePolicy::LocalReflow, red),
            (3, true)
        );
        // The same spaces with default attributes are not content.
        assert_eq!(
            styled_space_line_after_narrowing(ResizePolicy::LocalReflow, super::Color::Default),
            (1, false)
        );
    }

    #[test]
    fn console_buffer_drops_trailing_spaces_whatever_their_attributes() {
        // Measured on the inbox conhost: 160 red-background spaces behaved
        // exactly like default ones.
        let red = super::Color::Indexed(1);
        assert_eq!(
            styled_space_line_after_narrowing(ResizePolicy::ConsoleBuffer, red),
            (1, false)
        );
    }

    #[test]
    fn console_buffer_cursor_on_emptied_continuation_row_follows_console() {
        // Measured 2026-10-01 on the inbox conhost and the bundled
        // OpenConsole (identical): "text0", then `spaces` spaces and an `x`
        // that wraps onto a new row, CR + erase-to-end-of-line so the cursor
        // sits at column 0 of that now empty continuation row, narrowed from
        // 80 columns. Each case is (spaces, new_cols, row, col), 1-based.
        // Left out: (160, 20), (240, 30) and (240, 20), where the console
        // lands 2, 1 and 5 rows above where its own wrapping puts the end of
        // the text; no rule for that is known.
        let cases = [
            (80, 40, 4, 1),
            (80, 30, 4, 21),
            (80, 20, 6, 1),
            (80, 60, 3, 21),
            (100, 40, 4, 1),
            (100, 30, 4, 21),
            (100, 20, 6, 1),
            (100, 60, 3, 21),
            (160, 40, 6, 1),
            (160, 30, 7, 11),
            (160, 60, 4, 41),
            (240, 40, 8, 1),
            (240, 60, 6, 1),
        ];
        for (spaces, new_cols, row, col) in cases {
            let mut state = TerminalState::new(80, 24);
            for ch in "text0".chars() {
                state.put_char(ch);
            }
            state.carriage_return();
            state.linefeed();
            for _ in 0..spaces {
                state.put_char(' ');
            }
            state.put_char('x');
            state.carriage_return();
            state.erase_in_line(0);
            state.resize_with_policy(new_cols, 24, ResizePolicy::ConsoleBuffer);
            assert_eq!(
                (state.primary_cursor.row as usize + 1, state.primary_cursor.col as usize + 1),
                (row, col),
                "{spaces} spaces -> {new_cols} cols"
            );
        }
    }

    #[test]
    fn console_buffer_full_width_space_rows_with_line_breaks_do_not_grow() {
        // The OpenConsole clear-by-spaces case: four rows of 80 spaces, each
        // ended by CR LF, narrowed to 40 columns. The prompt stays put (the
        // measured console moved it by 0 rows; counting the spaces as
        // content moved it by 4).
        for policy in [ResizePolicy::ConsoleBuffer, ResizePolicy::LocalReflow] {
            let mut state = TerminalState::new(80, 24);
            for i in 0..4 {
                for ch in format!("text{i}").chars() {
                    state.put_char(ch);
                }
                state.carriage_return();
                state.linefeed();
            }
            for _ in 0..4 {
                for _ in 0..80 {
                    state.put_char(' ');
                }
                state.carriage_return();
                state.linefeed();
            }
            for ch in "$ prompt".chars() {
                state.put_char(ch);
            }
            assert_eq!(state.primary_cursor.row, 8);
            state.resize_with_policy(40, 24, policy);
            assert_eq!(state.primary_cursor.row, 8, "{policy:?}");
            assert_eq!(visible_row_text(&state, 8), "$ prompt", "{policy:?}");
        }
    }
}
