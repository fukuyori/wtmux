//! Terminal state management
//!
//! This module defines the terminal's screen buffer, cursor state, and attributes.

use bitflags::bitflags;
use std::collections::HashSet;
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
        self.cols = cols;
        self.rows = rows;
        self.primary_screen.resize(cols, rows);
        self.alternate_screen.resize(cols, rows);
        self.scroll_region = (0, rows.saturating_sub(1));

        // Clamp cursor positions
        let max_col = cols.saturating_sub(1);
        let max_row = rows.saturating_sub(1);
        
        self.primary_cursor.col = self.primary_cursor.col.min(max_col);
        self.primary_cursor.row = self.primary_cursor.row.min(max_row);
        self.alternate_cursor.col = self.alternate_cursor.col.min(max_col);
        self.alternate_cursor.row = self.alternate_cursor.row.min(max_row);
    }

    /// Put a character at the current cursor position
    pub fn put_char(&mut self, ch: char) {
        let width = ch.width().unwrap_or(0) as u16;

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
        let mut result = String::new();
        
        for abs_row in start.1..=end.1 {
            let row = match screen.get_row_absolute(abs_row) {
                Some(r) => r,
                None => continue,
            };
            
            let col_start = if abs_row == start.1 { start.0 as usize } else { 0 };
            let col_end = if abs_row == end.1 { end.0 as usize + 1 } else { row.cells.len() };
            
            for col_idx in col_start..col_end.min(row.cells.len()) {
                let cell = &row.cells[col_idx];
                if !cell.is_continuation() {
                    if cell.grapheme.is_empty() {
                        result.push(' ');
                    } else {
                        result.push_str(&cell.grapheme);
                    }
                }
            }
            
            // Add newline between rows (but not for wrapped lines)
            if abs_row < end.1 && !row.wrapped {
                // Trim trailing spaces from line
                while result.ends_with(' ') {
                    result.pop();
                }
                result.push('\n');
            }
        }
        
        // Trim trailing spaces
        while result.ends_with(' ') {
            result.pop();
        }
        
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
    pub scrollback: Vec<Row>,
    /// Maximum scrollback lines
    pub scrollback_limit: usize,
    /// Current scroll offset (0 = at bottom, >0 = scrolled up)
    pub scroll_offset: usize,
    pub dirty_lines: HashSet<usize>,
    pub full_redraw: bool,
}

impl ScreenBuffer {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            rows: (0..rows).map(|_| Row::new(cols)).collect(),
            scrollback: Vec::new(),
            scrollback_limit: 10000,
            scroll_offset: 0,
            dirty_lines: HashSet::new(),
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

        self.mark_all_dirty();
    }

    /// Add a row to scrollback when scrolling up
    pub fn push_to_scrollback(&mut self, row: Row) {
        self.scrollback.push(row);
        // Trim if exceeding limit
        if self.scrollback.len() > self.scrollback_limit {
            self.scrollback.remove(0);
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

    /// Get line cells at absolute row position
    pub fn get_line_at_absolute(&self, abs_row: usize) -> Option<&Vec<Cell>> {
        self.get_row_absolute(abs_row).map(|r| &r.cells)
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
        self.dirty_lines.insert(line);
    }

    pub fn mark_all_dirty(&mut self) {
        self.full_redraw = true;
    }

    pub fn clear_dirty(&mut self) {
        self.dirty_lines.clear();
        self.full_redraw = false;
    }
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
#[derive(Clone, Copy, PartialEq, Default)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl Color {
    /// Convert to crossterm color
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
        }
    }
}
