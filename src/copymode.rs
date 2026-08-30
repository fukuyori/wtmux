//! Copy mode - vim-like scrollback navigation and text selection.
//!
//! Copy mode allows users to navigate the scrollback buffer, select text,
//! and copy it to the clipboard, all using vim-style keybindings.
//!
//! # Entering Copy Mode
//!
//! - `Ctrl+B, [` - Enter copy mode
//! - `Ctrl+B, /` - Enter copy mode with search
//!
//! # Navigation (vim-style)
//!
//! | Key | Action |
//! |-----|--------|
//! | h/j/k/l | Move cursor |
//! | w/b | Word forward/backward |
//! | 0/$ | Line start/end |
//! | g/G | Buffer top/bottom |
//! | Ctrl+U/D | Half page up/down |
//! | Ctrl+B/F | Full page up/down |
//!
//! # Selection and Copy
//!
//! | Key | Action |
//! |-----|--------|
//! | Space/v | Start selection |
//! | Enter/y | Copy and exit |
//! | Escape | Cancel and exit |
//!
//! # Search
//!
//! | Key | Action |
//! |-----|--------|
//! | / | Search forward |
//! | ? | Search backward |
//! | n | Next match |
//! | N | Previous match |

use crate::core::term::state::{cells_text_range, ScreenBuffer};
use crate::wm::WindowManager;

/// Text of the inclusive cell range `from..=to` ((row, col) in absolute
/// buffer coordinates, `from <= to`), joined with newlines and with trailing
/// whitespace trimmed per line.
///
/// Wide characters occupy two cells. The cursor moves one cell at a time, so
/// a selection boundary can land on the right half (continuation cell) of a
/// wide char. The highlight covers that whole char, so the copied text does
/// too: a start on a continuation cell is widened left to the char's lead
/// cell; an end on one already includes the lead cell.
fn selection_text(screen: &ScreenBuffer, from: (usize, u16), to: (usize, u16)) -> Option<String> {
    let (from_row, from_col) = from;
    let (to_row, to_col) = to;
    let mut text = String::new();

    for row in from_row..=to_row {
        let line = screen.get_line_at_absolute(row)?;

        let mut start_c = if row == from_row { from_col as usize } else { 0 };
        if start_c > 0
            && line.get(start_c).is_some_and(|c| c.is_continuation())
            && !line[start_c - 1].is_continuation()
        {
            start_c -= 1;
        }
        let end_c = if row == to_row { to_col as usize + 1 } else { line.len() };

        text.push_str(&cells_text_range(line, start_c, end_c));

        if row < to_row {
            text.push('\n');
        }
    }

    let text = text
        .lines()
        .map(|l| l.trim_end())
        .collect::<Vec<_>>()
        .join("\n");

    Some(text)
}

/// Copy mode state
pub struct CopyMode {
    /// Whether copy mode is active
    pub active: bool,
    /// Cursor position (col, row) in buffer coordinates
    pub cursor_col: u16,
    pub cursor_row: usize,
    /// Selection start (col, row) - None if not selecting
    pub selection_start: Option<(u16, usize)>,
    /// Search mode
    pub search_mode: bool,
    /// Search query
    pub search_query: String,
    /// Search direction (true = forward, false = backward)
    pub search_forward: bool,
    /// Search matches (row, col_start, col_end)
    pub search_matches: Vec<(usize, u16, u16)>,
    /// Current match index
    pub current_match: usize,
    /// Scroll offset from bottom
    pub scroll_offset: usize,
}

impl CopyMode {
    pub fn new() -> Self {
        Self {
            active: false,
            cursor_col: 0,
            cursor_row: 0,
            selection_start: None,
            search_mode: false,
            search_query: String::new(),
            search_forward: true,
            search_matches: Vec::new(),
            current_match: 0,
            scroll_offset: 0,
        }
    }

    /// Enter copy mode
    pub fn enter(&mut self, wm: &WindowManager) {
        self.active = true;
        self.selection_start = None;
        self.search_mode = false;
        self.search_query.clear();
        self.search_matches.clear();
        
        // Position cursor at current cursor position
        if let Some(tab) = wm.active_tab() {
            if let Some(pane) = tab.focused_pane() {
                let cursor = pane.session.state.active_cursor();
                self.cursor_col = cursor.col;
                self.cursor_row = pane.session.state.active_screen().visible_row_to_absolute(cursor.row);
                self.scroll_offset = 0;
            }
        }
    }

    /// Exit copy mode
    pub fn exit(&mut self) {
        self.active = false;
        self.selection_start = None;
        self.search_mode = false;
        self.search_query.clear();
        self.search_matches.clear();
    }

    /// Move cursor up
    pub fn cursor_up(&mut self, wm: &WindowManager) {
        if let Some(min_row) = self.get_min_row(wm) {
            if self.cursor_row > min_row {
                self.cursor_row -= 1;
                self.adjust_scroll(wm);
            }
        }
    }

    /// Move cursor down
    pub fn cursor_down(&mut self, wm: &WindowManager) {
        if let Some(max_row) = self.get_max_row(wm) {
            if self.cursor_row < max_row {
                self.cursor_row += 1;
                self.adjust_scroll(wm);
            }
        }
    }

    /// Move cursor left
    pub fn cursor_left(&mut self, wm: &WindowManager) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > self.get_min_row(wm).unwrap_or(0) {
            // Wrap to end of previous line
            self.cursor_row -= 1;
            self.cursor_col = self.get_line_width(wm).saturating_sub(1);
            self.adjust_scroll(wm);
        }
    }

    /// Move cursor right
    pub fn cursor_right(&mut self, wm: &WindowManager) {
        let line_width = self.get_line_width(wm);
        if self.cursor_col + 1 < line_width {
            self.cursor_col += 1;
        } else if self.cursor_row < self.get_max_row(wm).unwrap_or(0) {
            // Wrap to start of next line
            self.cursor_row += 1;
            self.cursor_col = 0;
            self.adjust_scroll(wm);
        }
    }

    /// Move to line start
    pub fn line_start(&mut self) {
        self.cursor_col = 0;
    }

    /// Move to line end
    pub fn line_end(&mut self, wm: &WindowManager) {
        self.cursor_col = self.get_line_width(wm).saturating_sub(1);
    }

    /// Page up
    pub fn page_up(&mut self, wm: &WindowManager) {
        if let Some(tab) = wm.active_tab() {
            if let Some(pane) = tab.focused_pane() {
                let page_size = pane.session.state.rows as usize;
                let min_row = self.get_min_row(wm).unwrap_or(0);
                self.cursor_row = self.cursor_row.saturating_sub(page_size).max(min_row);
                self.adjust_scroll(wm);
            }
        }
    }

    /// Page down
    pub fn page_down(&mut self, wm: &WindowManager) {
        if let Some(tab) = wm.active_tab() {
            if let Some(pane) = tab.focused_pane() {
                let page_size = pane.session.state.rows as usize;
                let max_row = self.get_max_row(wm).unwrap_or(0);
                self.cursor_row = (self.cursor_row + page_size).min(max_row);
                self.adjust_scroll(wm);
            }
        }
    }

    /// Go to top
    pub fn goto_top(&mut self, wm: &WindowManager) {
        self.cursor_row = self.get_min_row(wm).unwrap_or(0);
        self.cursor_col = 0;
        self.adjust_scroll(wm);
    }

    /// Go to bottom
    pub fn goto_bottom(&mut self, wm: &WindowManager) {
        self.cursor_row = self.get_max_row(wm).unwrap_or(0);
        self.cursor_col = 0;
        self.adjust_scroll(wm);
    }

    /// Half page up
    pub fn half_page_up(&mut self, wm: &WindowManager) {
        if let Some(tab) = wm.active_tab() {
            if let Some(pane) = tab.focused_pane() {
                let half_page = (pane.session.state.rows as usize) / 2;
                let min_row = self.get_min_row(wm).unwrap_or(0);
                self.cursor_row = self.cursor_row.saturating_sub(half_page).max(min_row);
                self.adjust_scroll(wm);
            }
        }
    }

    /// Half page down
    pub fn half_page_down(&mut self, wm: &WindowManager) {
        if let Some(tab) = wm.active_tab() {
            if let Some(pane) = tab.focused_pane() {
                let half_page = (pane.session.state.rows as usize) / 2;
                let max_row = self.get_max_row(wm).unwrap_or(0);
                self.cursor_row = (self.cursor_row + half_page).min(max_row);
                self.adjust_scroll(wm);
            }
        }
    }

    /// Toggle selection
    pub fn toggle_selection(&mut self) {
        if self.selection_start.is_some() {
            self.selection_start = None;
        } else {
            self.selection_start = Some((self.cursor_col, self.cursor_row));
        }
    }

    /// Start selection (Space or v)
    #[allow(dead_code)]
    pub fn start_selection(&mut self) {
        self.selection_start = Some((self.cursor_col, self.cursor_row));
    }

    /// Get selected text and copy to clipboard
    pub fn copy_selection(&mut self, wm: &WindowManager) -> Option<String> {
        let (start_col, start_row) = self.selection_start?;
        
        // Determine selection bounds
        let (from_row, from_col, to_row, to_col) = if start_row < self.cursor_row 
            || (start_row == self.cursor_row && start_col <= self.cursor_col) {
            (start_row, start_col, self.cursor_row, self.cursor_col)
        } else {
            (self.cursor_row, self.cursor_col, start_row, start_col)
        };

        let tab = wm.active_tab()?;
        let pane = tab.focused_pane()?;
        let screen = pane.session.state.active_screen();

        let text = selection_text(screen, (from_row, from_col), (to_row, to_col))?;

        self.selection_start = None;

        Some(text)
    }

    /// Enter search mode
    pub fn enter_search(&mut self, forward: bool) {
        self.search_mode = true;
        self.search_forward = forward;
        self.search_query.clear();
        self.search_matches.clear();
    }

    /// Add character to search query
    pub fn search_input(&mut self, c: char) {
        self.search_query.push(c);
    }

    /// Remove last character from search query
    pub fn search_backspace(&mut self) {
        self.search_query.pop();
    }

    /// Execute search
    pub fn execute_search(&mut self, wm: &WindowManager) {
        self.search_mode = false;
        self.search_matches.clear();
        
        if self.search_query.is_empty() {
            return;
        }

        let tab = match wm.active_tab() {
            Some(t) => t,
            None => return,
        };
        let pane = match tab.focused_pane() {
            Some(p) => p,
            None => return,
        };
        
        let screen = pane.session.state.active_screen();
        let query_lower = self.search_query.to_lowercase();
        
        // Search through all lines
        let total_lines = screen.total_lines();
        for row in 0..total_lines {
            if let Some(line) = screen.get_line_at_absolute(row) {
                let line_str: String = line.iter().map(|c| c.c()).collect();
                let line_lower = line_str.to_lowercase();
                
                let mut start = 0;
                while let Some(pos) = line_lower[start..].find(&query_lower) {
                    let col = start + pos;
                    let end_col = col + self.search_query.len();
                    self.search_matches.push((row, col as u16, end_col as u16));
                    start = col + 1;
                }
            }
        }
        
        // Jump to first match after cursor (or before if searching backward)
        if !self.search_matches.is_empty() {
            self.find_next_match(true);
        }
    }

    /// Find next match
    pub fn find_next_match(&mut self, initial: bool) {
        if self.search_matches.is_empty() {
            return;
        }
        
        if initial {
            // Find first match after cursor
            for (i, (row, col, _)) in self.search_matches.iter().enumerate() {
                if self.search_forward {
                    if *row > self.cursor_row || (*row == self.cursor_row && *col >= self.cursor_col) {
                        self.current_match = i;
                        self.jump_to_current_match();
                        return;
                    }
                } else {
                    if *row < self.cursor_row || (*row == self.cursor_row && *col <= self.cursor_col) {
                        self.current_match = i;
                    }
                }
            }
            
            // Wrap around
            self.current_match = if self.search_forward { 0 } else { self.search_matches.len() - 1 };
        } else {
            if self.search_forward {
                self.current_match = (self.current_match + 1) % self.search_matches.len();
            } else {
                self.current_match = if self.current_match == 0 {
                    self.search_matches.len() - 1
                } else {
                    self.current_match - 1
                };
            }
        }
        
        self.jump_to_current_match();
    }

    /// Find previous match
    pub fn find_prev_match(&mut self) {
        if self.search_matches.is_empty() {
            return;
        }
        
        if self.search_forward {
            self.current_match = if self.current_match == 0 {
                self.search_matches.len() - 1
            } else {
                self.current_match - 1
            };
        } else {
            self.current_match = (self.current_match + 1) % self.search_matches.len();
        }
        
        self.jump_to_current_match();
    }

    /// Jump cursor to current match
    fn jump_to_current_match(&mut self) {
        if let Some((row, col, _)) = self.search_matches.get(self.current_match) {
            self.cursor_row = *row;
            self.cursor_col = *col;
        }
    }

    /// Cancel search
    pub fn cancel_search(&mut self) {
        self.search_mode = false;
        self.search_query.clear();
    }

    /// Adjust scroll to keep cursor visible
    fn adjust_scroll(&mut self, wm: &WindowManager) {
        if let Some(tab) = wm.active_tab() {
            if let Some(pane) = tab.focused_pane() {
                let visible_rows = pane.session.state.rows as usize;
                let screen = pane.session.state.active_screen();
                let total_lines = screen.total_lines();
                
                // Calculate visible range
                let bottom_row = total_lines.saturating_sub(1);
                let visible_start = bottom_row.saturating_sub(self.scroll_offset + visible_rows - 1);
                let visible_end = bottom_row.saturating_sub(self.scroll_offset);
                
                // Adjust scroll if cursor is outside visible range
                if self.cursor_row < visible_start {
                    self.scroll_offset = bottom_row.saturating_sub(self.cursor_row + visible_rows - 1);
                } else if self.cursor_row > visible_end {
                    self.scroll_offset = bottom_row.saturating_sub(self.cursor_row);
                }
            }
        }
    }

    /// Get minimum row (top of scrollback)
    fn get_min_row(&self, wm: &WindowManager) -> Option<usize> {
        let tab = wm.active_tab()?;
        let _pane = tab.focused_pane()?;
        Some(0)
    }

    /// Get maximum row (bottom of screen)
    fn get_max_row(&self, wm: &WindowManager) -> Option<usize> {
        let tab = wm.active_tab()?;
        let pane = tab.focused_pane()?;
        let screen = pane.session.state.active_screen();
        Some(screen.total_lines().saturating_sub(1))
    }

    /// Get current line width
    fn get_line_width(&self, wm: &WindowManager) -> u16 {
        if let Some(tab) = wm.active_tab() {
            if let Some(pane) = tab.focused_pane() {
                return pane.session.state.cols;
            }
        }
        80
    }

    /// Get visible row from absolute row
    pub fn absolute_to_visible(&self, absolute_row: usize, wm: &WindowManager) -> Option<u16> {
        let tab = wm.active_tab()?;
        let pane = tab.focused_pane()?;
        let screen = pane.session.state.active_screen();
        let total_lines = screen.total_lines();
        let visible_rows = pane.session.state.rows as usize;
        
        let bottom_row = total_lines.saturating_sub(1);
        let visible_start = bottom_row.saturating_sub(self.scroll_offset + visible_rows - 1);
        let visible_end = bottom_row.saturating_sub(self.scroll_offset);
        
        if absolute_row >= visible_start && absolute_row <= visible_end {
            Some((absolute_row - visible_start) as u16)
        } else {
            None
        }
    }

    /// Check if a row is in selection
    pub fn is_selected(&self, row: usize, col: u16) -> bool {
        let (start_col, start_row) = match self.selection_start {
            Some(s) => s,
            None => return false,
        };
        
        let (from_row, from_col, to_row, to_col) = if start_row < self.cursor_row 
            || (start_row == self.cursor_row && start_col <= self.cursor_col) {
            (start_row, start_col, self.cursor_row, self.cursor_col)
        } else {
            (self.cursor_row, self.cursor_col, start_row, start_col)
        };
        
        if row < from_row || row > to_row {
            return false;
        }
        
        if row == from_row && row == to_row {
            col >= from_col && col <= to_col
        } else if row == from_row {
            col >= from_col
        } else if row == to_row {
            col <= to_col
        } else {
            true
        }
    }

    /// Check if a position is a search match
    pub fn is_search_match(&self, row: usize, col: u16) -> bool {
        for (match_row, start_col, end_col) in &self.search_matches {
            if *match_row == row && col >= *start_col && col < *end_col {
                return true;
            }
        }
        false
    }

    /// Check if a position is the current search match
    pub fn is_current_match(&self, row: usize, col: u16) -> bool {
        if let Some((match_row, start_col, end_col)) = self.search_matches.get(self.current_match) {
            *match_row == row && col >= *start_col && col < *end_col
        } else {
            false
        }
    }

    /// Get search status text
    pub fn search_status(&self) -> String {
        if self.search_mode {
            format!("/{}", self.search_query)
        } else if !self.search_matches.is_empty() {
            format!("[{}/{}] {}", 
                self.current_match + 1, 
                self.search_matches.len(),
                self.search_query
            )
        } else if !self.search_query.is_empty() {
            format!("Pattern not found: {}", self.search_query)
        } else {
            String::new()
        }
    }
}

impl Default for CopyMode {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::session::Session;

    fn screen_with(lines: &[&str]) -> Session {
        let mut s = Session::new(0, 40, 5);
        s.feed_bytes(lines.join("\r\n").as_bytes());
        s
    }

    #[test]
    fn wide_chars_have_no_stray_spaces() {
        let s = screen_with(&["echo 日本語", "abc"]);
        let screen = s.state.active_screen();
        // "echo " = cols 0..5, 日=5-6, 本=7-8, 語=9-10.
        assert_eq!(selection_text(screen, (0, 0), (0, 10)).unwrap(), "echo 日本語");
        // Mixed line: only wide chars were affected, ASCII stays as-is.
        assert_eq!(selection_text(screen, (0, 0), (1, 2)).unwrap(), "echo 日本語\nabc");
    }

    #[test]
    fn issue_7_sample() {
        let s = screen_with(&["## 既知の制限"]);
        let screen = s.state.active_screen();
        assert_eq!(selection_text(screen, (0, 0), (0, 39)).unwrap(), "## 既知の制限");
    }

    #[test]
    fn start_on_continuation_cell_includes_whole_char() {
        let s = screen_with(&["日本語"]);
        let screen = s.state.active_screen();
        // col 1 is the right half of 日: widen left so the char is not lost.
        assert_eq!(selection_text(screen, (0, 1), (0, 5)).unwrap(), "日本語");
        // col 2 is the lead of 本: 日 is excluded.
        assert_eq!(selection_text(screen, (0, 2), (0, 5)).unwrap(), "本語");
        // end on 本's continuation (col 3) still includes 本.
        assert_eq!(selection_text(screen, (0, 0), (0, 3)).unwrap(), "日本");
    }

    #[test]
    fn multi_codepoint_grapheme_is_kept() {
        // é as e + combining acute: two codepoints, one cell.
        let s = screen_with(&["e\u{301}x"]);
        let screen = s.state.active_screen();
        assert_eq!(selection_text(screen, (0, 0), (0, 1)).unwrap(), "e\u{301}x");
    }
}
