//! Terminal state management
//!
//! This module defines the terminal's screen buffer, cursor state, and attributes.

use super::resize::{reflow_screen, ReflowAnchor, ResizeOutcome, ResizePolicy, ScreenResizePlan};
use bitflags::bitflags;
use std::collections::VecDeque;
use unicode_width::UnicodeWidthChar;

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
    /// Scroll region (top, bottom) - 0-indexed, inclusive
    pub scroll_region: (u16, u16),
    /// Text selection state
    pub selection: Option<Selection>,
    /// Shell integration state (OSC 133 / OSC 633)
    pub shell_integration: ShellIntegration,
    /// Keystroke tracker (fallback when shell integration is inactive)
    pub keystroke_tracker: KeystrokeTracker,
}

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

/// Determine the display width of a character for terminal cell layout.
///
/// `unicode_width` returns `None` (→ 0) for Private Use Area characters
/// (U+E000–U+F8FF, U+F0000–U+FFFFF) which include all Nerd Font glyphs
/// and Powerline symbols.  Windows Terminal treats these as width 1, so
/// we do the same to keep cell accounting in sync with the host terminal.
///
/// Soft Nerd Font "wide" glyphs (some icon sets) are rendered as width 2
/// by certain terminals, but Windows Terminal defaults to width 1 for all
/// PUA characters unless the profile has `"experimental.rendering.forceFullRepaint"`
/// or similar flags set.  Width-1 is therefore the safest default.
fn char_display_width(ch: char) -> u16 {
    let cp = ch as u32;
    // Private Use Area — always treat as width 1
    if (0xE000..=0xF8FF).contains(&cp)
        || (0xF0000..=0xFFFFF).contains(&cp)
        || (0x100000..=0x10FFFF).contains(&cp)
    {
        return 1;
    }
    ch.width().unwrap_or(1) as u16
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
            scroll_region: (0, rows.saturating_sub(1)),
            selection: None,
            shell_integration: ShellIntegration::default(),
            keystroke_tracker: KeystrokeTracker::default(),
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

    /// Resize the terminal
    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.resize_with_policy(cols, rows, ResizePolicy::LocalReflow);
    }

    pub fn resize_with_policy(&mut self, cols: u16, rows: u16, policy: ResizePolicy) -> ResizeOutcome {
        self.cols = cols;
        self.rows = rows;
        let mut outcome = ResizeOutcome::default();

        match policy {
            ResizePolicy::LocalReflow => {
                let mut primary_anchors = vec![ReflowAnchor {
                    abs_row: self.primary_screen.scrollback.len() + self.primary_cursor.row as usize,
                    col: self.primary_cursor.col,
                }];
                let prompt_anchor_idx = if let (Some(prompt_row), Some(prompt_col)) = (
                    self.shell_integration.prompt_end_row,
                    self.shell_integration.prompt_end_col,
                ) {
                    primary_anchors.push(ReflowAnchor {
                        abs_row: self.primary_screen.scrollback.len() + prompt_row as usize,
                        col: prompt_col,
                    });
                    Some(primary_anchors.len() - 1)
                } else {
                    None
                };

                let primary_plan = reflow_screen(&self.primary_screen, cols, rows, &primary_anchors);
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
            }
            ResizePolicy::HostDriven | ResizePolicy::NoReflow => {
                self.primary_screen.resize(cols, rows);
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
        let width = char_display_width(ch);

        if width == 0 {
            // Combining character - append to previous cell
            self.append_to_previous_cell(ch);
            return;
        }

        // Get cursor position first
        let (cursor_row, cursor_col) = {
            let cursor = self.active_cursor();
            (cursor.row, cursor.col)
        };

        // Handle line wrap - only when cursor is completely beyond the screen edge
        // We allow writing at cols-1 even for wide characters, trusting ConPTY to handle wrapping
        // This prevents premature wrapping when unicode-width differs from ConPTY's calculation
        if cursor_col >= self.cols {
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

        // Handle overwriting wide characters
        self.handle_wide_char_overwrite(row, col);

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

    fn append_to_previous_cell(&mut self, ch: char) {
        let (row, col) = {
            let cursor = self.active_cursor();
            (cursor.row as usize, cursor.col as usize)
        };

        if col > 0 {
            let screen = self.active_screen_mut();
            screen.rows[row].cells[col - 1].grapheme.push(ch);
            screen.mark_dirty(row);
        }
    }

    fn handle_wide_char_overwrite(&mut self, row: usize, col: usize) {
        let attrs = self.current_attrs.clone();
        let cols = self.cols as usize;
        let screen = self.active_screen_mut();

        // Check if we're overwriting the right half of a wide char
        if col > 0 && screen.rows[row].cells[col].is_continuation() {
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
        screen.mark_all_dirty();
    }

    /// Scroll the screen down by n lines
    pub fn scroll_down(&mut self, n: u16) {
        let (top, bottom) = self.scroll_region;
        let cols = self.cols;

        let screen = self.active_screen_mut();

        for _ in 0..n {
            if (bottom as usize) < screen.rows.len() && (top as usize) <= screen.rows.len() {
                screen.rows.remove(bottom as usize);
                screen.rows.insert(top as usize, Row::new(cols));
            }
        }
        screen.mark_all_dirty();
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
        screen.mark_dirty(row);
    }

    /// Insert lines at cursor position
    pub fn insert_lines(&mut self, n: u16) {
        let cursor_row = self.active_cursor().row as usize;
        let total_rows = self.rows as usize;
        let cols = self.cols;

        let screen = self.active_screen_mut();

        for _ in 0..n {
            if cursor_row < screen.rows.len() {
                screen.rows.insert(cursor_row, Row::new(cols));
                if screen.rows.len() > total_rows {
                    screen.rows.pop();
                }
            }
        }
        screen.mark_all_dirty();
    }

    /// Delete lines at cursor position
    pub fn delete_lines(&mut self, n: u16) {
        let cursor_row = self.active_cursor().row as usize;
        let cols = self.cols;

        let screen = self.active_screen_mut();

        for _ in 0..n {
            if cursor_row < screen.rows.len() {
                screen.rows.remove(cursor_row);
                screen.rows.push(Row::new(cols));
            }
        }
        screen.mark_all_dirty();
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
            2004 => self.modes.bracketed_paste = enable,
            
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
    let mut text = String::new();
    for col_idx in start_col..end_col.min(row.cells.len()) {
        let cell = &row.cells[col_idx];
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

/// Cell attributes
#[derive(Clone, Default, PartialEq)]
pub struct CellAttrs {
    pub fg: Color,
    pub bg: Color,
    pub flags: AttrFlags,
}

impl CellAttrs {
    pub fn reset(&mut self) {
        *self = Self::default();
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
#[derive(Clone)]
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
    /// Clears the previous confirmed command so a fresh one can be captured.
    pub fn on_prompt_start(&mut self) {
        self.confirmed_command = None;
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
}
