//! Floating message composer overlay.
//!
//! A small multi-line editor in a centered popup for composing a message
//! and sending it to a pane's PTY (typically an AI agent such as Claude
//! Code). Opened with `Prefix + m` for the focused pane, or with `m` from
//! the agent dashboard for the selected pane. Enter inserts a newline;
//! Ctrl+Enter (or Ctrl+S on terminals that cannot report it) sends; Esc
//! cancels.

use crate::core::term::width::char_width;
use crate::wm::{PaneId, TabId};

/// Sent and cancelled messages kept for Ctrl+P/N recall.
const HISTORY_LIMIT: usize = 100;

/// Undo snapshots kept for Ctrl+Z.
const UNDO_LIMIT: usize = 100;

/// What the previous buffer mutation was, for coalescing undo steps.
/// A run of plain typing collapses into one undo step; everything else
/// (and any cursor move in between) starts a new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum EditKind {
    #[default]
    Other,
    Typing,
}

/// Buffer + cursor state captured before a mutation (Ctrl+Z/Y).
#[derive(Debug, Clone)]
struct Snapshot {
    lines: Vec<String>,
    cursor_row: usize,
    cursor_col: usize,
}

/// The pane a composed message will be sent to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposeTarget {
    pub tab_id: TabId,
    pub pane_id: PaneId,
    /// Human-readable pane description shown in the popup title
    pub label: String,
}

/// One soft-wrapped display row: chars `[start, end)` of `lines[line]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WrappedRow {
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// State for the message composer overlay (`UiMode::MessageComposer`).
#[derive(Default)]
pub struct MessageComposer {
    /// Destination pane; `None` while the composer is closed
    pub target: Option<ComposeTarget>,
    /// Edit buffer, one entry per line (never empty while open)
    pub lines: Vec<String>,
    /// Cursor line index into `lines`
    pub cursor_row: usize,
    /// Cursor position within the line, in chars
    pub cursor_col: usize,
    /// Messages sent or cancelled this session, oldest first
    pub history: Vec<String>,
    /// Position while browsing history with Ctrl+P/N; `None` = editing
    history_index: Option<usize>,
    /// Buffer stashed when history browsing started
    stashed_draft: Option<Vec<String>>,
    /// Unsent input preserved only while a send result is pending
    draft: Option<Vec<String>>,
    /// Fixed end of a Shift+Arrow selection; the cursor is the moving end
    selection_anchor: Option<(usize, usize)>,
    /// States restorable with Ctrl+Z, newest last
    undo_stack: Vec<Snapshot>,
    /// States undone with Ctrl+Z, restorable with Ctrl+Y
    redo_stack: Vec<Snapshot>,
    /// Kind of the previous mutation, for coalescing typing runs
    last_edit: EditKind,
    /// Popup size chosen by dragging the border: `(box_width, body_h)`.
    /// Kept across open/close for the session; clamped by the layout.
    pub custom_size: Option<(usize, usize)>,
}

impl MessageComposer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the composer targeting the given pane. A failed-send draft is
    /// restored; after a normal cancel the buffer starts empty.
    pub fn open(&mut self, tab_id: TabId, pane_id: PaneId, label: String) {
        self.target = Some(ComposeTarget {
            tab_id,
            pane_id,
            label,
        });
        self.lines = self.draft.take().unwrap_or_default();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.history_index = None;
        self.stashed_draft = None;
        self.selection_anchor = None;
        self.reset_undo();
        self.move_to_end();
    }

    /// Close the composer. Cancelling an open composer records its unfinished
    /// text in history and starts a fresh message next time. When the target
    /// was already taken for sending, preserve the text only until the send
    /// succeeds so a failed send can still be retried.
    pub fn close(&mut self) {
        let was_cancelled = self.target.take().is_some();
        let text = self.text();
        if was_cancelled {
            self.record_history(&text);
            self.draft = None;
        } else if self.lines.iter().any(|line| !line.is_empty()) {
            self.draft = Some(std::mem::take(&mut self.lines));
        }
        self.lines.clear();
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.history_index = None;
        self.stashed_draft = None;
        self.selection_anchor = None;
        self.reset_undo();
    }

    /// Take the target out of the composer (used on send).
    pub fn take_target(&mut self) -> Option<ComposeTarget> {
        self.target.take()
    }

    /// The composed text with `\n` line separators.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    fn line_len(&self, row: usize) -> usize {
        self.lines.get(row).map_or(0, |l| l.chars().count())
    }

    /// Byte offset of `char_index` within `line`.
    fn byte_at(line: &str, char_index: usize) -> usize {
        line.char_indices()
            .nth(char_index)
            .map_or(line.len(), |(i, _)| i)
    }

    pub fn insert_char(&mut self, c: char) {
        // Replacing a selection is its own undo step; plain typing coalesces
        let kind = if self.selection_range().is_some() {
            EditKind::Other
        } else {
            EditKind::Typing
        };
        self.push_undo(kind);
        self.insert_char_raw(c);
    }

    fn insert_char_raw(&mut self, c: char) {
        self.delete_selection();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        let row = self.cursor_row.min(self.lines.len() - 1);
        let line = &mut self.lines[row];
        let at = Self::byte_at(line, self.cursor_col);
        line.insert(at, c);
        self.cursor_col += 1;
    }

    /// Insert text as one undo step, splitting on newlines (`\r\n`, `\n`,
    /// and lone `\r` from terminal paste all count as line breaks).
    pub fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.push_undo(EditKind::Other);
        for c in text.replace("\r\n", "\n").replace('\r', "\n").chars() {
            if c == '\n' {
                self.insert_newline_raw();
            } else if !c.is_control() {
                self.insert_char_raw(c);
            }
        }
    }

    pub fn insert_newline(&mut self) {
        self.push_undo(EditKind::Other);
        self.insert_newline_raw();
    }

    fn insert_newline_raw(&mut self) {
        self.delete_selection();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        let row = self.cursor_row.min(self.lines.len() - 1);
        let line = &mut self.lines[row];
        let at = Self::byte_at(line, self.cursor_col);
        let rest = line.split_off(at);
        self.lines.insert(row + 1, rest);
        self.cursor_row = row + 1;
        self.cursor_col = 0;
    }

    /// Delete the char before the cursor; at column 0, join with the
    /// previous line.
    pub fn backspace(&mut self) {
        if self.selection_range().is_some() {
            self.push_undo(EditKind::Other);
            self.delete_selection();
            return;
        }
        if self.cursor_row == 0 && self.cursor_col == 0 {
            return;
        }
        self.push_undo(EditKind::Other);
        if self.cursor_col > 0 {
            let line = &mut self.lines[self.cursor_row];
            let at = Self::byte_at(line, self.cursor_col - 1);
            line.remove(at);
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            let current = self.lines.remove(self.cursor_row);
            self.cursor_row -= 1;
            self.cursor_col = self.line_len(self.cursor_row);
            self.lines[self.cursor_row].push_str(&current);
        }
    }

    /// Delete the char under the cursor; at end of line, join with the
    /// next line.
    pub fn delete(&mut self) {
        if self.selection_range().is_some() {
            self.push_undo(EditKind::Other);
            self.delete_selection();
            return;
        }
        if self.cursor_row + 1 >= self.lines.len()
            && self.cursor_col >= self.line_len(self.cursor_row)
        {
            return;
        }
        self.push_undo(EditKind::Other);
        let len = self.line_len(self.cursor_row);
        if self.cursor_col < len {
            let line = &mut self.lines[self.cursor_row];
            let at = Self::byte_at(line, self.cursor_col);
            line.remove(at);
        } else if self.cursor_row + 1 < self.lines.len() {
            let next = self.lines.remove(self.cursor_row + 1);
            self.lines[self.cursor_row].push_str(&next);
        }
    }

    /// A cursor move ends the current typing run, so the next insertion
    /// starts a fresh undo step.
    fn break_typing_run(&mut self) {
        self.last_edit = EditKind::Other;
    }

    pub fn move_left(&mut self) {
        self.break_typing_run();
        if let Some((start, _)) = self.selection_range() {
            (self.cursor_row, self.cursor_col) = start;
            self.selection_anchor = None;
            return;
        }
        self.selection_anchor = None;
        self.move_left_raw();
    }

    fn move_left_raw(&mut self) {
        if self.cursor_col > 0 {
            self.cursor_col -= 1;
        } else if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.line_len(self.cursor_row);
        }
    }

    pub fn move_right(&mut self) {
        self.break_typing_run();
        if let Some((_, end)) = self.selection_range() {
            (self.cursor_row, self.cursor_col) = end;
            self.selection_anchor = None;
            return;
        }
        self.selection_anchor = None;
        self.move_right_raw();
    }

    fn move_right_raw(&mut self) {
        if self.cursor_col < self.line_len(self.cursor_row) {
            self.cursor_col += 1;
        } else if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = 0;
        }
    }

    pub fn move_up(&mut self) {
        self.break_typing_run();
        self.selection_anchor = None;
        self.move_up_raw();
    }

    fn move_up_raw(&mut self) {
        if self.cursor_row > 0 {
            self.cursor_row -= 1;
            self.cursor_col = self.cursor_col.min(self.line_len(self.cursor_row));
        }
    }

    pub fn move_down(&mut self) {
        self.break_typing_run();
        self.selection_anchor = None;
        self.move_down_raw();
    }

    fn move_down_raw(&mut self) {
        if self.cursor_row + 1 < self.lines.len() {
            self.cursor_row += 1;
            self.cursor_col = self.cursor_col.min(self.line_len(self.cursor_row));
        }
    }

    pub fn move_home(&mut self) {
        self.break_typing_run();
        self.selection_anchor = None;
        self.cursor_col = 0;
    }

    pub fn move_end(&mut self) {
        self.break_typing_run();
        self.selection_anchor = None;
        self.cursor_col = self.line_len(self.cursor_row);
    }

    /// Ctrl+Home: move to the very start of the buffer.
    pub fn move_buffer_start(&mut self) {
        self.break_typing_run();
        self.selection_anchor = None;
        self.cursor_row = 0;
        self.cursor_col = 0;
    }

    /// Ctrl+End: move to the very end of the buffer.
    pub fn move_buffer_end(&mut self) {
        self.break_typing_run();
        self.selection_anchor = None;
        self.move_to_end();
    }

    pub fn select_left(&mut self) {
        self.begin_selection();
        self.move_left_raw();
        self.finish_selection();
    }

    pub fn select_right(&mut self) {
        self.begin_selection();
        self.move_right_raw();
        self.finish_selection();
    }

    pub fn select_up(&mut self) {
        self.begin_selection();
        self.move_up_raw();
        self.finish_selection();
    }

    pub fn select_down(&mut self) {
        self.begin_selection();
        self.move_down_raw();
        self.finish_selection();
    }

    /// Shift+Home: extend the selection to the start of the line.
    pub fn select_home(&mut self) {
        self.begin_selection();
        self.cursor_col = 0;
        self.finish_selection();
    }

    /// Shift+End: extend the selection to the end of the line.
    pub fn select_end(&mut self) {
        self.begin_selection();
        self.cursor_col = self.line_len(self.cursor_row);
        self.finish_selection();
    }

    /// Ctrl+A: select the whole buffer, cursor at the end.
    pub fn select_all(&mut self) {
        self.break_typing_run();
        self.selection_anchor = Some((0, 0));
        self.move_to_end();
        self.finish_selection();
    }

    fn begin_selection(&mut self) {
        self.break_typing_run();
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some((self.cursor_row, self.cursor_col));
        }
    }

    fn finish_selection(&mut self) {
        if self.selection_anchor == Some((self.cursor_row, self.cursor_col)) {
            self.selection_anchor = None;
        }
    }

    /// Ordered selection endpoints. The end position is exclusive.
    pub fn selection_range(&self) -> Option<((usize, usize), (usize, usize))> {
        let anchor = self.selection_anchor?;
        let cursor = (self.cursor_row, self.cursor_col);
        if anchor == cursor {
            None
        } else if anchor < cursor {
            Some((anchor, cursor))
        } else {
            Some((cursor, anchor))
        }
    }

    /// Whether the character at `(row, col)` belongs to the selection.
    pub fn is_selected(&self, row: usize, col: usize) -> bool {
        self.selection_range()
            .is_some_and(|(start, end)| (row, col) >= start && (row, col) < end)
    }

    /// The text inside the current selection (Ctrl+C), with `\n` line
    /// separators.
    pub fn selected_text(&self) -> Option<String> {
        let (start, end) = self.selection_range()?;
        let mut out = String::new();
        for row in start.0..=end.0 {
            let line = self.lines.get(row).map_or("", String::as_str);
            let from = if row == start.0 {
                Self::byte_at(line, start.1)
            } else {
                0
            };
            let to = if row == end.0 {
                Self::byte_at(line, end.1)
            } else {
                line.len()
            };
            if row > start.0 {
                out.push('\n');
            }
            out.push_str(&line[from..to]);
        }
        Some(out)
    }

    /// Remove the current selection and return its text (Ctrl+X).
    pub fn cut_selection(&mut self) -> Option<String> {
        let text = self.selected_text()?;
        self.push_undo(EditKind::Other);
        self.delete_selection();
        Some(text)
    }

    /// Delete the current selection and place the cursor at its start.
    fn delete_selection(&mut self) -> bool {
        let Some((start, end)) = self.selection_range() else {
            return false;
        };
        let start_byte = Self::byte_at(&self.lines[start.0], start.1);
        let end_byte = Self::byte_at(&self.lines[end.0], end.1);
        let prefix = self.lines[start.0][..start_byte].to_string();
        let suffix = self.lines[end.0][end_byte..].to_string();
        self.lines
            .splice(start.0..=end.0, [format!("{prefix}{suffix}")]);
        (self.cursor_row, self.cursor_col) = start;
        self.selection_anchor = None;
        true
    }

    /// Clear the buffer, keeping the target (Ctrl+U).
    pub fn clear(&mut self) {
        if self.text().is_empty() {
            return;
        }
        self.push_undo(EditKind::Other);
        self.lines = vec![String::new()];
        self.cursor_row = 0;
        self.cursor_col = 0;
        self.selection_anchor = None;
    }

    fn move_to_end(&mut self) {
        self.cursor_row = self.lines.len().saturating_sub(1);
        self.cursor_col = self.line_len(self.cursor_row);
    }

    // ── Undo / redo ──────────────────────────────────────────────────────

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.lines.clone(),
            cursor_row: self.cursor_row,
            cursor_col: self.cursor_col,
        }
    }

    /// Record the current state before a mutation. Consecutive plain typing
    /// coalesces into the step already on the stack; any other mutation (or
    /// a cursor move in between) starts a new step. Any new edit invalidates
    /// the redo stack.
    fn push_undo(&mut self, kind: EditKind) {
        if !(kind == EditKind::Typing && self.last_edit == EditKind::Typing) {
            self.undo_stack.push(self.snapshot());
            if self.undo_stack.len() > UNDO_LIMIT {
                self.undo_stack.remove(0);
            }
        }
        self.redo_stack.clear();
        self.last_edit = kind;
    }

    /// Ctrl+Z: restore the state before the most recent edit.
    pub fn undo(&mut self) {
        let Some(prev) = self.undo_stack.pop() else {
            return;
        };
        self.redo_stack.push(self.snapshot());
        self.restore(prev);
    }

    /// Ctrl+Y: re-apply the most recently undone edit.
    pub fn redo(&mut self) {
        let Some(next) = self.redo_stack.pop() else {
            return;
        };
        self.undo_stack.push(self.snapshot());
        self.restore(next);
    }

    /// Restoring a snapshot also leaves history browsing, since the
    /// snapshot only captures the buffer, not the browse position.
    fn restore(&mut self, s: Snapshot) {
        self.lines = s.lines;
        self.cursor_row = s.cursor_row;
        self.cursor_col = s.cursor_col;
        self.selection_anchor = None;
        self.history_index = None;
        self.stashed_draft = None;
        self.last_edit = EditKind::Other;
    }

    fn reset_undo(&mut self) {
        self.undo_stack.clear();
        self.redo_stack.clear();
        self.last_edit = EditKind::Other;
    }

    // ── Mouse ────────────────────────────────────────────────────────────

    /// Char position `x_cells` display cells into wrapped row `wrow`,
    /// counting wide chars as two cells. Past the end of the row the last
    /// position wins.
    fn char_at_cells(&self, wrow: WrappedRow, x_cells: usize) -> usize {
        let Some(line) = self.lines.get(wrow.line) else {
            return 0;
        };
        let mut cells = 0;
        let mut col = wrow.start;
        for ch in line.chars().skip(wrow.start).take(wrow.end - wrow.start) {
            let w = char_width(ch);
            if cells + w > x_cells {
                break;
            }
            cells += w;
            col += 1;
        }
        col
    }

    fn move_cursor_display(&mut self, rows: &[WrappedRow], display_row: usize, x_cells: usize) {
        let Some(&wrow) = rows.get(display_row.min(rows.len().saturating_sub(1))) else {
            return;
        };
        self.cursor_row = wrow.line;
        self.cursor_col = self.char_at_cells(wrow, x_cells);
    }

    /// Place the cursor at a clicked display position. `rows` must come
    /// from [`Self::wrapped_rows`] with the same width the box was drawn at.
    pub fn set_cursor_display(&mut self, rows: &[WrappedRow], display_row: usize, x_cells: usize) {
        self.break_typing_run();
        self.selection_anchor = None;
        self.move_cursor_display(rows, display_row, x_cells);
    }

    /// Extend the selection to a dragged display position, anchored where
    /// the drag started.
    pub fn drag_to_display(&mut self, rows: &[WrappedRow], display_row: usize, x_cells: usize) {
        self.begin_selection();
        self.move_cursor_display(rows, display_row, x_cells);
        self.finish_selection();
    }

    // ── Soft wrap ────────────────────────────────────────────────────────

    /// Split every line into display rows at most `width` cells wide.
    /// An empty line still yields one (empty) row.
    pub fn wrapped_rows(&self, width: usize) -> Vec<WrappedRow> {
        let width = width.max(2);
        let mut rows = Vec::new();
        for (line_index, line) in self.lines.iter().enumerate() {
            let mut start = 0;
            let mut cols = 0;
            let mut char_count = 0;
            for (i, ch) in line.chars().enumerate() {
                let w = char_width(ch);
                if cols + w > width && char_count > 0 {
                    rows.push(WrappedRow {
                        line: line_index,
                        start,
                        end: i,
                    });
                    start = i;
                    cols = 0;
                    char_count = 0;
                }
                cols += w;
                char_count += 1;
            }
            rows.push(WrappedRow {
                line: line_index,
                start,
                end: line.chars().count(),
            });
        }
        if rows.is_empty() {
            rows.push(WrappedRow {
                line: 0,
                start: 0,
                end: 0,
            });
        }
        rows
    }

    /// The display row (index into `rows`) and char offset within it where
    /// the cursor sits. `rows` must come from [`Self::wrapped_rows`].
    pub fn cursor_display_pos(&self, rows: &[WrappedRow]) -> (usize, usize) {
        let cursor_line = self.cursor_row.min(self.lines.len().saturating_sub(1));
        let mut fallback = (0, 0);
        for (index, row) in rows.iter().enumerate() {
            if row.line != cursor_line {
                continue;
            }
            let is_last_of_line = rows
                .get(index + 1)
                .map_or(true, |next| next.line != cursor_line);
            if self.cursor_col < row.end || is_last_of_line {
                let offset = self.cursor_col.clamp(row.start, row.end) - row.start;
                return (index, offset);
            }
            fallback = (index, row.end - row.start);
        }
        fallback
    }

    // ── Sent-message history ─────────────────────────────────────────────

    /// Record a successfully sent message and drop the pending draft.
    /// Consecutive duplicates are skipped.
    pub fn record_sent(&mut self, text: &str) {
        self.record_history(text);
        self.draft = None;
    }

    fn record_history(&mut self, text: &str) {
        if !text.trim().is_empty() && self.history.last().map(String::as_str) != Some(text) {
            self.history.push(text.to_string());
            if self.history.len() > HISTORY_LIMIT {
                self.history.remove(0);
            }
        }
    }

    /// Current history browse position as `(index, total)`, if browsing.
    pub fn history_position(&self) -> Option<(usize, usize)> {
        self.history_index.map(|i| (i, self.history.len()))
    }

    /// Ctrl+P: recall the previous (older) sent message. The first press
    /// stashes the in-progress input so Ctrl+N can bring it back.
    pub fn history_prev(&mut self) {
        if self.history.is_empty() {
            return;
        }
        let index = match self.history_index {
            None => {
                self.push_undo(EditKind::Other);
                self.stashed_draft = Some(std::mem::take(&mut self.lines));
                self.history.len() - 1
            }
            // Already at the oldest entry: stay there
            Some(0) => return,
            Some(i) => {
                self.push_undo(EditKind::Other);
                i - 1
            }
        };
        self.load_history(index);
    }

    /// Ctrl+N: move to the next (newer) sent message; past the newest one,
    /// restore the stashed in-progress input.
    pub fn history_next(&mut self) {
        match self.history_index {
            None => {}
            Some(i) if i + 1 < self.history.len() => {
                self.push_undo(EditKind::Other);
                self.load_history(i + 1);
            }
            Some(_) => {
                self.push_undo(EditKind::Other);
                self.history_index = None;
                self.lines = self.stashed_draft.take().unwrap_or_default();
                if self.lines.is_empty() {
                    self.lines.push(String::new());
                }
                self.selection_anchor = None;
                self.move_to_end();
            }
        }
    }

    fn load_history(&mut self, index: usize) {
        self.history_index = Some(index);
        self.lines = self.history[index].split('\n').map(str::to_string).collect();
        self.selection_anchor = None;
        self.move_to_end();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_composer() -> MessageComposer {
        let mut composer = MessageComposer::new();
        composer.open(1, 2, "1:main · 1: agent".to_string());
        composer
    }

    #[test]
    fn typing_and_newlines_build_multiline_text() {
        let mut c = open_composer();
        c.insert_str("fix the bug");
        c.insert_newline();
        c.insert_str("then run tests");
        assert_eq!(c.text(), "fix the bug\nthen run tests");
        assert_eq!((c.cursor_row, c.cursor_col), (1, 14));
    }

    #[test]
    fn newline_splits_line_at_cursor() {
        let mut c = open_composer();
        c.insert_str("abcd");
        c.cursor_col = 2;
        c.insert_newline();
        assert_eq!(c.lines, vec!["ab", "cd"]);
        assert_eq!((c.cursor_row, c.cursor_col), (1, 0));
    }

    #[test]
    fn backspace_joins_lines_at_column_zero() {
        let mut c = open_composer();
        c.insert_str("ab\ncd");
        c.cursor_row = 1;
        c.cursor_col = 0;
        c.backspace();
        assert_eq!(c.lines, vec!["abcd"]);
        assert_eq!((c.cursor_row, c.cursor_col), (0, 2));
    }

    #[test]
    fn delete_at_line_end_joins_next_line() {
        let mut c = open_composer();
        c.insert_str("ab\ncd");
        c.cursor_row = 0;
        c.cursor_col = 2;
        c.delete();
        assert_eq!(c.lines, vec!["abcd"]);
    }

    #[test]
    fn cursor_movement_wraps_across_lines_and_clamps() {
        let mut c = open_composer();
        c.insert_str("long line\nab");
        // Right at end of last line stays put
        c.move_right();
        assert_eq!((c.cursor_row, c.cursor_col), (1, 2));
        // Left at column 0 wraps to previous line end
        c.cursor_col = 0;
        c.move_left();
        assert_eq!((c.cursor_row, c.cursor_col), (0, 9));
        // Down clamps the column to the shorter line
        c.move_down();
        assert_eq!((c.cursor_row, c.cursor_col), (1, 2));
    }

    #[test]
    fn shift_arrows_extend_and_shrink_selection_across_lines() {
        let mut c = open_composer();
        c.insert_str("ab\ncd");

        c.select_left();
        assert_eq!(c.selection_range(), Some(((1, 1), (1, 2))));
        assert!(c.is_selected(1, 1));

        c.select_up();
        assert_eq!(c.selection_range(), Some(((0, 1), (1, 2))));
        assert!(c.is_selected(0, 1));
        assert!(c.is_selected(0, 2)); // line break after "ab"
        assert!(c.is_selected(1, 0));

        c.select_down();
        assert_eq!(c.selection_range(), Some(((1, 1), (1, 2))));
        c.select_right();
        assert_eq!(c.selection_range(), None);
    }

    #[test]
    fn typing_and_deletion_replace_selected_text() {
        let mut c = open_composer();
        c.insert_str("hello");
        c.move_left();
        c.move_left();
        c.select_left();
        c.select_left();
        c.insert_char('X');
        assert_eq!(c.text(), "hXlo");
        assert_eq!((c.cursor_row, c.cursor_col), (0, 2));
        assert_eq!(c.selection_range(), None);

        c.clear();
        c.insert_str("ab\ncd");
        c.select_up();
        c.backspace();
        assert_eq!(c.text(), "ab");
        assert_eq!((c.cursor_row, c.cursor_col), (0, 2));
    }

    #[test]
    fn multibyte_editing_uses_char_positions() {
        let mut c = open_composer();
        c.insert_str("こんにちは");
        c.cursor_col = 2;
        c.insert_char('！');
        assert_eq!(c.text(), "こん！にちは");
        c.backspace();
        assert_eq!(c.text(), "こんにちは");
    }

    #[test]
    fn paste_normalizes_crlf_and_cr() {
        let mut c = open_composer();
        c.insert_str("a\r\nb\rc");
        assert_eq!(c.lines, vec!["a", "b", "c"]);
    }

    #[test]
    fn open_resets_state_and_take_target_empties_it() {
        let mut c = open_composer();
        c.insert_str("draft");
        c.open(3, 4, "2:build · 1: agent".to_string());
        assert_eq!(c.text(), "");
        let target = c.take_target().unwrap();
        assert_eq!((target.tab_id, target.pane_id), (3, 4));
        assert!(c.take_target().is_none());
    }

    #[test]
    fn wrapping_splits_long_lines_by_display_width() {
        let mut c = open_composer();
        c.insert_str("abcdefgh\nxy");
        let rows = c.wrapped_rows(3);
        let spans: Vec<(usize, usize, usize)> =
            rows.iter().map(|r| (r.line, r.start, r.end)).collect();
        assert_eq!(spans, vec![(0, 0, 3), (0, 3, 6), (0, 6, 8), (1, 0, 2)]);
    }

    #[test]
    fn wrapping_counts_wide_chars_as_two_cells() {
        let mut c = open_composer();
        c.insert_str("あいうえ");
        // Width 5 fits two double-width chars per row
        let rows = c.wrapped_rows(5);
        let spans: Vec<(usize, usize, usize)> =
            rows.iter().map(|r| (r.line, r.start, r.end)).collect();
        assert_eq!(spans, vec![(0, 0, 2), (0, 2, 4)]);
    }

    #[test]
    fn empty_lines_keep_one_display_row() {
        let mut c = open_composer();
        c.insert_str("a\n\nb");
        let rows = c.wrapped_rows(10);
        assert_eq!(rows.len(), 3);
        assert_eq!((rows[1].line, rows[1].start, rows[1].end), (1, 0, 0));
    }

    #[test]
    fn cursor_maps_into_wrapped_rows() {
        let mut c = open_composer();
        c.insert_str("abcdefgh");
        let rows = c.wrapped_rows(3);
        // Cursor after "abcd" → second row, offset 1
        c.cursor_col = 4;
        assert_eq!(c.cursor_display_pos(&rows), (1, 1));
        // Cursor at a wrap boundary belongs to the next row
        c.cursor_col = 3;
        assert_eq!(c.cursor_display_pos(&rows), (1, 0));
        // Cursor at end of line stays on the line's last row
        c.cursor_col = 8;
        assert_eq!(c.cursor_display_pos(&rows), (2, 2));
    }

    #[test]
    fn history_recall_cycles_and_restores_draft() {
        let mut c = open_composer();
        c.record_sent("first");
        c.record_sent("second\nline");
        c.insert_str("typing");

        c.history_prev();
        assert_eq!(c.text(), "second\nline");
        assert_eq!(c.history_position(), Some((1, 2)));
        c.history_prev();
        assert_eq!(c.text(), "first");
        // Ctrl+P at the oldest entry stays there
        c.history_prev();
        assert_eq!(c.text(), "first");

        c.history_next();
        assert_eq!(c.text(), "second\nline");
        // Past the newest entry the in-progress input comes back
        c.history_next();
        assert_eq!(c.text(), "typing");
        assert_eq!(c.history_position(), None);
    }

    #[test]
    fn history_skips_consecutive_duplicates_and_empty() {
        let mut c = open_composer();
        c.record_sent("same");
        c.record_sent("same");
        c.record_sent("   ");
        assert_eq!(c.history, vec!["same"]);
    }

    #[test]
    fn cancel_records_unfinished_text_and_reopens_empty() {
        let mut c = open_composer();
        c.insert_str("unsent\ndraft");
        c.close();
        assert_eq!(c.history, vec!["unsent\ndraft"]);

        c.open(1, 2, "1:main · 1: agent".to_string());
        assert_eq!(c.text(), "");
        c.history_prev();
        assert_eq!(c.text(), "unsent\ndraft");
    }

    #[test]
    fn failed_send_draft_is_restored_until_send_succeeds() {
        let mut c = open_composer();
        c.insert_str("retry me");
        let target = c.take_target().unwrap();
        c.close();
        c.open(target.tab_id, target.pane_id, target.label.clone());
        assert_eq!(c.text(), "retry me");

        c.take_target();
        c.close();
        c.record_sent("retry me");
        c.open(1, 2, "1:main · 1: agent".to_string());
        assert_eq!(c.text(), "");
        assert_eq!(c.history, vec!["retry me"]);
    }

    #[test]
    fn selected_text_extracts_partial_and_multiline_ranges() {
        let mut c = open_composer();
        c.insert_str("あbc\nde");
        assert_eq!(c.selected_text(), None);

        c.select_left();
        assert_eq!(c.selected_text().as_deref(), Some("e"));
        c.select_up();
        assert_eq!(c.selected_text().as_deref(), Some("bc\nde"));
    }

    #[test]
    fn select_all_and_cut_remove_everything() {
        let mut c = open_composer();
        c.select_all();
        assert_eq!(c.selected_text(), None); // empty buffer selects nothing

        c.insert_str("ab\ncd");
        c.select_all();
        assert_eq!(c.selected_text().as_deref(), Some("ab\ncd"));

        assert_eq!(c.cut_selection().as_deref(), Some("ab\ncd"));
        assert_eq!(c.text(), "");
        assert_eq!(c.cut_selection(), None); // no selection left

        c.undo();
        assert_eq!(c.text(), "ab\ncd");
    }

    #[test]
    fn shift_home_end_select_to_line_edges() {
        let mut c = open_composer();
        c.insert_str("hello");
        c.move_left();
        c.move_left();
        c.move_left();

        c.select_end();
        assert_eq!(c.selection_range(), Some(((0, 2), (0, 5))));
        c.select_home();
        assert_eq!(c.selection_range(), Some(((0, 0), (0, 2))));
    }

    #[test]
    fn ctrl_home_end_jump_across_the_buffer() {
        let mut c = open_composer();
        c.insert_str("ab\ncd\nef");
        c.move_buffer_start();
        assert_eq!((c.cursor_row, c.cursor_col), (0, 0));
        c.move_buffer_end();
        assert_eq!((c.cursor_row, c.cursor_col), (2, 2));
    }

    #[test]
    fn undo_coalesces_typing_and_redo_replays_it() {
        let mut c = open_composer();
        c.insert_str("first ");
        c.insert_char('a');
        c.insert_char('b'); // coalesces with 'a'
        assert_eq!(c.text(), "first ab");

        c.undo();
        assert_eq!(c.text(), "first ");
        c.undo();
        assert_eq!(c.text(), "");
        c.redo();
        assert_eq!(c.text(), "first ");
        c.redo();
        assert_eq!(c.text(), "first ab");

        // A new edit clears the redo stack
        c.undo();
        c.insert_char('x');
        c.redo();
        assert_eq!(c.text(), "first x");
    }

    #[test]
    fn cursor_move_breaks_typing_coalescing() {
        let mut c = open_composer();
        c.insert_char('a');
        c.insert_char('b');
        c.move_left();
        c.insert_char('X');
        assert_eq!(c.text(), "aXb");

        c.undo();
        assert_eq!(c.text(), "ab");
        c.undo();
        assert_eq!(c.text(), "");
    }

    #[test]
    fn selection_replacement_and_clear_are_single_undo_steps() {
        let mut c = open_composer();
        c.insert_str("hello");
        c.select_left();
        c.select_left();
        c.insert_char('X');
        assert_eq!(c.text(), "helX");

        c.undo();
        assert_eq!(c.text(), "hello");
        assert_eq!(c.selection_range(), None);

        c.clear();
        assert_eq!(c.text(), "");
        c.undo();
        assert_eq!(c.text(), "hello");
    }

    #[test]
    fn history_recall_is_undoable() {
        let mut c = open_composer();
        c.record_sent("older");
        c.insert_str("draft");

        c.history_prev();
        assert_eq!(c.text(), "older");
        c.undo();
        assert_eq!(c.text(), "draft");
        assert_eq!(c.history_position(), None);
    }

    #[test]
    fn tab_inserts_spaces_as_one_undo_step() {
        let mut c = open_composer();
        c.insert_str("a");
        c.insert_str("    ");
        assert_eq!(c.text(), "a    ");
        c.undo();
        assert_eq!(c.text(), "a");
    }

    #[test]
    fn mouse_positions_map_display_cells_to_chars() {
        let mut c = open_composer();
        c.insert_str("あいうえ\nxy");
        // Width 5 → display rows (0,0,2), (0,2,4), (1,0,2)
        let rows = c.wrapped_rows(5);

        // A click on the second cell of い lands before it
        c.set_cursor_display(&rows, 0, 3);
        assert_eq!((c.cursor_row, c.cursor_col), (0, 1));
        // Past the end of a wrapped row clamps to its last position
        c.set_cursor_display(&rows, 1, 99);
        assert_eq!((c.cursor_row, c.cursor_col), (0, 4));
        // A row beyond the last clamps to the last row
        c.set_cursor_display(&rows, 99, 0);
        assert_eq!((c.cursor_row, c.cursor_col), (1, 0));
    }

    #[test]
    fn mouse_drag_selects_from_press_point() {
        let mut c = open_composer();
        c.insert_str("abcdef");
        let rows = c.wrapped_rows(10);

        c.set_cursor_display(&rows, 0, 1);
        c.drag_to_display(&rows, 0, 4);
        assert_eq!(c.selection_range(), Some(((0, 1), (0, 4))));
        assert_eq!(c.selected_text().as_deref(), Some("bcd"));
    }
}
