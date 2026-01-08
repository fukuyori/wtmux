//! Window Manager - Manages multiple tabs and provides tmux-like functionality

use std::collections::HashMap;
use super::tab::{Tab, TabId};
use super::pane::PaneId;
use super::layout::SplitDirection;

/// Window Manager - handles tabs and pane operations
pub struct WindowManager {
    /// All tabs
    tabs: HashMap<TabId, Tab>,
    /// Tab order (for tab bar display)
    tab_order: Vec<TabId>,
    /// Currently active tab
    active_tab: TabId,
    /// Last active tab (for toggle)
    last_active_tab: Option<TabId>,
    /// Next tab ID
    next_tab_id: TabId,
    /// Terminal dimensions
    pub width: u16,
    pub height: u16,
    /// Height reserved for tab bar
    pub tab_bar_height: u16,
    /// Height reserved for status bar
    pub status_bar_height: u16,
    /// Default shell command
    pub default_shell: Option<String>,
    /// Default codepage
    pub default_codepage: Option<u32>,
    /// Prefix key mode (like tmux Ctrl+b)
    pub prefix_mode: bool,
}

impl WindowManager {
    /// Create a new window manager
    pub fn new(width: u16, height: u16, shell: Option<String>, codepage: Option<u32>) -> Self {
        let tab_bar_height = 1;
        let status_bar_height = 1;
        let content_height = height.saturating_sub(tab_bar_height + status_bar_height);
        
        // Create initial tab
        let tab_id = 1;
        let tab = Tab::new(tab_id, "1:main".to_string(), width, content_height);
        
        let mut tabs = HashMap::new();
        tabs.insert(tab_id, tab);
        
        Self {
            tabs,
            tab_order: vec![tab_id],
            active_tab: tab_id,
            last_active_tab: None,
            next_tab_id: 2,
            width,
            height,
            tab_bar_height,
            status_bar_height,
            default_shell: shell,
            default_codepage: codepage,
            prefix_mode: false,
        }
    }

    /// Get content area dimensions (excluding tab bar and status bar)
    pub fn content_size(&self) -> (u16, u16) {
        (self.width, self.height.saturating_sub(self.tab_bar_height + self.status_bar_height))
    }

    /// Create a new tab
    pub fn new_tab(&mut self) -> TabId {
        let tab_id = self.next_tab_id;
        self.next_tab_id += 1;
        
        let (width, height) = self.content_size();
        let tab_name = format!("{}:shell", tab_id);
        let mut tab = Tab::new(tab_id, tab_name, width, height);
        
        // Start session in the initial pane
        if let Some(pane) = tab.focused_pane_mut() {
            let _ = pane.session.start_with_codepage(
                self.default_shell.as_deref(),
                self.default_codepage
            );
        }
        
        self.tabs.insert(tab_id, tab);
        self.tab_order.push(tab_id);
        self.active_tab = tab_id;
        
        tab_id
    }

    /// Close the current tab
    pub fn close_tab(&mut self) -> bool {
        if self.tabs.len() <= 1 {
            return false; // Keep at least one tab
        }
        
        let tab_id = self.active_tab;
        self.tabs.remove(&tab_id);
        self.tab_order.retain(|&id| id != tab_id);
        
        // Switch to another tab
        if let Some(&new_active) = self.tab_order.first() {
            self.active_tab = new_active;
        }
        
        true
    }

    /// Switch to next tab
    pub fn next_tab(&mut self) {
        if let Some(pos) = self.tab_order.iter().position(|&id| id == self.active_tab) {
            let next_pos = (pos + 1) % self.tab_order.len();
            self.active_tab = self.tab_order[next_pos];
        }
    }

    /// Switch to previous tab
    pub fn prev_tab(&mut self) {
        if let Some(pos) = self.tab_order.iter().position(|&id| id == self.active_tab) {
            let prev_pos = if pos == 0 { self.tab_order.len() - 1 } else { pos - 1 };
            self.active_tab = self.tab_order[prev_pos];
        }
    }

    /// Switch to tab by number (1-indexed)
    pub fn goto_tab(&mut self, num: usize) {
        if num > 0 && num <= self.tab_order.len() {
            self.active_tab = self.tab_order[num - 1];
        }
    }

    /// Get the active tab
    pub fn active_tab(&self) -> Option<&Tab> {
        self.tabs.get(&self.active_tab)
    }

    /// Get the active tab mutably
    pub fn active_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(&self.active_tab)
    }

    /// Split the current pane horizontally
    pub fn split_horizontal(&mut self) -> Option<PaneId> {
        let shell = self.default_shell.clone();
        let codepage = self.default_codepage;
        self.active_tab_mut()?.split(SplitDirection::Horizontal, shell.as_deref(), codepage)
    }

    /// Split the current pane vertically
    pub fn split_vertical(&mut self) -> Option<PaneId> {
        let shell = self.default_shell.clone();
        let codepage = self.default_codepage;
        self.active_tab_mut()?.split(SplitDirection::Vertical, shell.as_deref(), codepage)
    }

    /// Close the current pane
    pub fn close_pane(&mut self) -> bool {
        if let Some(tab) = self.active_tab_mut() {
            if tab.close_pane() {
                return true;
            }
        }
        // If last pane in tab, close the tab
        self.close_tab()
    }

    /// Move focus to next pane
    pub fn focus_next_pane(&mut self) {
        if let Some(tab) = self.active_tab_mut() {
            let pane_ids = tab.layout.pane_ids();
            if let Some(pos) = pane_ids.iter().position(|&id| id == tab.focused_pane) {
                let next_pos = (pos + 1) % pane_ids.len();
                tab.focus_pane(pane_ids[next_pos]);
            }
        }
    }

    /// Move focus to previous pane
    pub fn focus_prev_pane(&mut self) {
        if let Some(tab) = self.active_tab_mut() {
            let pane_ids = tab.layout.pane_ids();
            if let Some(pos) = pane_ids.iter().position(|&id| id == tab.focused_pane) {
                let prev_pos = if pos == 0 { pane_ids.len() - 1 } else { pos - 1 };
                tab.focus_pane(pane_ids[prev_pos]);
            }
        }
    }

    /// Move focus in a direction
    pub fn focus_direction(&mut self, direction: SplitDirection, forward: bool) {
        if let Some(tab) = self.active_tab_mut() {
            tab.focus_direction(direction, forward);
        }
    }

    /// Switch to last active tab
    pub fn last_tab(&mut self) {
        if let Some(last) = self.last_active_tab {
            if self.tabs.contains_key(&last) {
                let current = self.active_tab;
                self.active_tab = last;
                self.last_active_tab = Some(current);
            }
        }
    }

    /// Rename the active tab
    pub fn rename_active_tab(&mut self, name: &str) {
        if let Some(tab) = self.active_tab_mut() {
            tab.name = name.to_string();
        }
    }

    /// Switch to next layout
    pub fn next_layout(&mut self) {
        if let Some(tab) = self.active_tab_mut() {
            tab.next_layout();
        }
    }

    /// Toggle zoom on current pane
    pub fn toggle_zoom(&mut self) {
        if let Some(tab) = self.active_tab_mut() {
            tab.toggle_zoom();
        }
    }

    /// Resize the window manager
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        let (content_width, content_height) = self.content_size();
        
        for tab in self.tabs.values_mut() {
            tab.resize(content_width, content_height);
        }
    }

    /// Resize the current pane
    pub fn resize_pane(&mut self, grow: bool) {
        let delta = if grow { 0.05 } else { -0.05 };
        if let Some(tab) = self.active_tab_mut() {
            tab.resize_pane(delta);
        }
    }

    /// Resize pane in a specific direction (tmux compatible)
    /// arrow_up_or_left: true = up/left arrow, false = down/right arrow
    pub fn resize_pane_direction(&mut self, direction: SplitDirection, arrow_up_or_left: bool) {
        if let Some(tab) = self.active_tab_mut() {
            tab.resize_pane_direction(direction, arrow_up_or_left);
        }
    }

    /// Swap current pane with next pane (Ctrl+B, })
    pub fn swap_pane_next(&mut self) {
        if let Some(tab) = self.active_tab_mut() {
            tab.swap_pane_next();
        }
    }

    /// Swap current pane with previous pane (Ctrl+B, {)
    pub fn swap_pane_prev(&mut self) {
        if let Some(tab) = self.active_tab_mut() {
            tab.swap_pane_prev();
        }
    }

    /// Get pane numbers for display (for Ctrl+B, q)
    /// Returns in pane_order order to match select_pane_by_number
    pub fn get_pane_numbers(&self) -> Vec<(PaneId, u16, u16, u16, u16)> {
        // Returns: (pane_id, x, y, width, height) in pane_order order
        if let Some(tab) = self.active_tab() {
            tab.pane_order.iter()
                .filter_map(|&id| tab.panes.get(&id))
                .map(|p| (p.id, p.x, p.y, p.width, p.height))
                .collect()
        } else {
            Vec::new()
        }
    }

    /// Select pane by number (0-9)
    pub fn select_pane_by_number(&mut self, num: usize) {
        if let Some(tab) = self.active_tab_mut() {
            let pane_ids: Vec<PaneId> = tab.pane_order.clone();
            if num < pane_ids.len() {
                tab.focus_pane(pane_ids[num]);
            }
        }
    }

    /// Process output for all tabs and handle closed panes
    pub fn process_output(&mut self) -> bool {
        let mut any_output = false;
        let mut tabs_to_check: Vec<TabId> = self.tabs.keys().cloned().collect();
        
        for tab_id in tabs_to_check.iter() {
            if let Some(tab) = self.tabs.get_mut(tab_id) {
                if tab.process_output() {
                    any_output = true;
                }
                // Clean up dead panes
                tab.cleanup_dead_panes();
            }
        }
        
        // Remove empty tabs
        tabs_to_check.retain(|id| {
            self.tabs.get(id).map(|t| t.panes.is_empty()).unwrap_or(false)
        });
        for tab_id in tabs_to_check {
            self.tabs.remove(&tab_id);
            self.tab_order.retain(|&id| id != tab_id);
        }
        
        // Update active tab if needed
        if !self.tabs.contains_key(&self.active_tab) {
            if let Some(&new_active) = self.tab_order.first() {
                self.active_tab = new_active;
            }
        }
        
        any_output
    }

    /// Check if any tab is still running
    pub fn is_running(&self) -> bool {
        !self.tabs.is_empty() && self.tabs.values().any(|t| t.is_running())
    }

    /// Get tab info for rendering tab bar
    pub fn tab_info(&self) -> Vec<(TabId, String, bool)> {
        self.tab_order.iter().map(|&id| {
            let tab = self.tabs.get(&id).unwrap();
            (id, tab.name.clone(), id == self.active_tab)
        }).collect()
    }

    /// Get status info for rendering status bar
    pub fn status_info(&self) -> String {
        if let Some(tab) = self.active_tab() {
            let pane_count = tab.panes.len();
            let focused_id = tab.focused_pane;
            let zoom_indicator = if tab.is_zoomed() { " [Z]" } else { "" };
            format!(
                "[{}] {}:{} | Pane {}/{}{}",
                self.active_tab,
                tab.name,
                focused_id,
                focused_id,
                pane_count,
                zoom_indicator
            )
        } else {
            "No active tab".to_string()
        }
    }

    /// Handle mouse down at position (start selection)
    pub fn handle_mouse_down(&mut self, col: u16, row: u16) {
        // Check if click is on tab bar
        if row < self.tab_bar_height {
            // TODO: Calculate which tab was clicked
            return;
        }
        
        // Adjust row for content area
        let content_row = row - self.tab_bar_height;
        
        // Find pane at position and focus it
        if let Some(tab) = self.active_tab_mut() {
            if let Some(pane_id) = tab.pane_at(col, content_row) {
                tab.focus_pane(pane_id);
                
                // Start selection in that pane
                if let Some(pane) = tab.panes.get_mut(&pane_id) {
                    let (inner_x, inner_y) = pane.inner_pos();
                    let pane_col = col.saturating_sub(inner_x);
                    let pane_row = content_row.saturating_sub(inner_y);
                    pane.session.state.start_selection(pane_col, pane_row);
                }
            }
        }
    }

    /// Handle mouse drag (extend selection)
    pub fn handle_mouse_drag(&mut self, col: u16, row: u16) {
        if row < self.tab_bar_height {
            return;
        }
        
        let content_row = row - self.tab_bar_height;
        
        if let Some(tab) = self.active_tab_mut() {
            if let Some(pane) = tab.focused_pane_mut() {
                let (inner_x, inner_y) = pane.inner_pos();
                let pane_col = col.saturating_sub(inner_x);
                let pane_row = content_row.saturating_sub(inner_y);
                pane.session.state.update_selection(pane_col, pane_row);
            }
        }
    }

    /// Handle mouse up (end selection and copy)
    pub fn handle_mouse_up(&mut self) -> Option<String> {
        if let Some(tab) = self.active_tab_mut() {
            if let Some(pane) = tab.focused_pane_mut() {
                let text = pane.session.state.get_selected_text();
                pane.session.state.clear_selection();
                return text;
            }
        }
        None
    }

    /// Handle scroll
    pub fn handle_scroll(&mut self, delta: i16) {
        if let Some(tab) = self.active_tab_mut() {
            if let Some(pane) = tab.focused_pane_mut() {
                let screen = pane.session.state.active_screen_mut();
                if delta > 0 {
                    screen.scroll_view_up(delta as usize);
                } else {
                    screen.scroll_view_down((-delta) as usize);
                }
            }
        }
    }

    /// Scroll to bottom (return to live view)
    pub fn scroll_to_bottom(&mut self) {
        if let Some(tab) = self.active_tab_mut() {
            if let Some(pane) = tab.focused_pane_mut() {
                pane.session.state.active_screen_mut().scroll_to_bottom();
            }
        }
    }

    /// Clear selection in focused pane
    pub fn clear_selection(&mut self) {
        if let Some(tab) = self.active_tab_mut() {
            if let Some(pane) = tab.focused_pane_mut() {
                pane.session.state.clear_selection();
            }
        }
    }

    /// Start the initial session
    pub fn start(&mut self) -> Result<(), String> {
        let shell = self.default_shell.clone();
        let codepage = self.default_codepage;
        if let Some(tab) = self.active_tab_mut() {
            if let Some(pane) = tab.focused_pane_mut() {
                pane.session.start_with_codepage(
                    shell.as_deref(),
                    codepage
                ).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// Write to the focused pane
    pub fn write(&mut self, data: &[u8]) -> Result<(), String> {
        if let Some(tab) = self.active_tab_mut() {
            if let Some(pane) = tab.focused_pane_mut() {
                pane.session.write(data).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// Toggle prefix mode
    #[allow(dead_code)]
    pub fn toggle_prefix_mode(&mut self) {
        self.prefix_mode = !self.prefix_mode;
    }

    /// Check if focused pane is using alternate screen (vim, less, etc.)
    pub fn is_in_alternate_screen(&self) -> bool {
        if let Some(tab) = self.active_tab() {
            if let Some(pane) = tab.focused_pane() {
                return pane.session.state.using_alternate;
            }
        }
        false
    }

    /// Clear current input line by sending Backspace for each character
    pub fn clear_current_input(&mut self) {
        // Get current line length to know how many backspaces to send
        if let Some(line) = self.get_current_line() {
            let stripped = crate::history::strip_prompt(&line);
            // Send backspace for each character in the current input
            for _ in stripped.chars() {
                let _ = self.write(&[0x08]); // Backspace
            }
        }
    }

    /// Get the current line at cursor position (for history recording)
    pub fn get_current_line(&self) -> Option<String> {
        let tab = self.active_tab()?;
        let pane = tab.focused_pane()?;
        let cursor = pane.session.state.active_cursor();
        let screen = pane.session.state.active_screen();
        
        // Get the row at cursor position
        let row = screen.get_row_at(cursor.row as usize)?;
        
        // Build the line text
        let mut line = String::new();
        for cell in &row.cells {
            if !cell.is_continuation() {
                if cell.grapheme.is_empty() {
                    line.push(' ');
                } else {
                    line.push_str(&cell.grapheme);
                }
            }
        }
        
        // Trim trailing spaces
        Some(line.trim_end().to_string())
    }
}
