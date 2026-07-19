//! Agent dashboard overlay (herdr-style).
//!
//! A popup listing every pane across all windows with its agent state
//! (WORKING / BLOCKED / DONE / IDLE). The list refreshes live while open,
//! and Enter jumps to the selected pane.

use crate::wm::{AgentEntry, WindowManager};

/// State for the agent dashboard overlay.
#[derive(Default)]
pub struct AgentDashboard {
    /// Whether the overlay is shown
    pub visible: bool,
    /// Index of the selected row
    pub selected: usize,
}

impl AgentDashboard {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the dashboard with the currently focused pane selected.
    pub fn open(&mut self, wm: &WindowManager) {
        self.visible = true;
        self.selected = wm
            .agent_overview()
            .iter()
            .position(|entry| entry.is_focused)
            .unwrap_or(0);
    }

    pub fn close(&mut self) {
        self.visible = false;
    }

    pub fn move_up(&mut self, entry_count: usize) {
        if entry_count > 0 {
            self.selected = if self.selected == 0 {
                entry_count - 1
            } else {
                self.selected - 1
            };
        }
    }

    pub fn move_down(&mut self, entry_count: usize) {
        if entry_count > 0 {
            self.selected = (self.selected + 1) % entry_count;
        }
    }

    /// Keep the selection valid when panes appear or disappear while open.
    pub fn clamp(&mut self, entry_count: usize) {
        if entry_count == 0 {
            self.selected = 0;
        } else if self.selected >= entry_count {
            self.selected = entry_count - 1;
        }
    }

    /// The currently selected entry, if any.
    pub fn selected_entry(&self, entries: &[AgentEntry]) -> Option<AgentEntry> {
        entries.get(self.selected).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selection_wraps_in_both_directions() {
        let mut dash = AgentDashboard::new();
        dash.move_up(3);
        assert_eq!(dash.selected, 2);
        dash.move_down(3);
        assert_eq!(dash.selected, 0);
    }

    #[test]
    fn clamp_keeps_selection_in_range() {
        let mut dash = AgentDashboard::new();
        dash.selected = 5;
        dash.clamp(2);
        assert_eq!(dash.selected, 1);
        dash.clamp(0);
        assert_eq!(dash.selected, 0);
    }
}
