//! Window selector state (tmux `choose-tree` style).
//!
//! The selector shows windows as top-level rows; each window can be expanded
//! to show its panes as child rows. This module owns the tree state
//! (selection, expansion, kill confirmation) and flattens it into the visible
//! row list consumed by the renderer and the input handlers.

use std::collections::HashSet;

use crate::wm::{TabId, WindowInfo, WindowManager};

/// One visible row in the window selector tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TreeEntry {
    /// A window row (zero-based index into the window list)
    Window { window: usize },
    /// A pane row beneath an expanded window (zero-based indices)
    Pane { window: usize, pane: usize },
}

/// State for the tmux-style window selector overlay.
#[derive(Default)]
pub struct WindowSelector {
    /// Whether the overlay is shown
    pub visible: bool,
    /// Index of the selected row in the flattened tree
    pub selected: usize,
    /// Pending kill confirmation (y/N) for the selected row
    pub kill_confirm: bool,
    /// Windows whose panes are currently shown
    expanded: HashSet<TabId>,
}

impl WindowSelector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Open the selector on the current window, with all windows collapsed.
    pub fn open(&mut self, wm: &WindowManager) {
        self.visible = true;
        self.kill_confirm = false;
        self.expanded.clear();
        // With everything collapsed, row index == window index
        self.selected = wm.active_tab_index();
    }

    pub fn close(&mut self) {
        self.visible = false;
        self.kill_confirm = false;
    }

    /// Flatten windows and expansion state into the visible rows.
    pub fn entries(&self, windows: &[WindowInfo]) -> Vec<TreeEntry> {
        let mut out = Vec::new();
        for (window, info) in windows.iter().enumerate() {
            out.push(TreeEntry::Window { window });
            if self.expanded.contains(&info.id) {
                for pane in 0..info.panes.len() {
                    out.push(TreeEntry::Pane { window, pane });
                }
            }
        }
        out
    }

    pub fn is_expanded(&self, id: TabId) -> bool {
        self.expanded.contains(&id)
    }

    /// The currently selected row, if any.
    pub fn selected_entry(&self, windows: &[WindowInfo]) -> Option<TreeEntry> {
        self.entries(windows).get(self.selected).copied()
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

    pub fn clamp(&mut self, entry_count: usize) {
        self.selected = self.selected.min(entry_count.saturating_sub(1));
    }

    /// Expand the selected window row to show its panes.
    pub fn expand(&mut self, windows: &[WindowInfo]) {
        if let Some(TreeEntry::Window { window }) = self.selected_entry(windows) {
            self.expanded.insert(windows[window].id);
        }
    }

    /// Collapse the selected row: a window row folds its panes; a pane row
    /// folds its window and moves the selection to the window row.
    pub fn collapse(&mut self, windows: &[WindowInfo]) {
        match self.selected_entry(windows) {
            Some(TreeEntry::Window { window }) => {
                self.expanded.remove(&windows[window].id);
            }
            Some(TreeEntry::Pane { window, .. }) => {
                self.expanded.remove(&windows[window].id);
                self.select_window(windows, window);
            }
            None => {}
        }
    }

    /// Move the selection to the row of a zero-based window index.
    pub fn select_window(&mut self, windows: &[WindowInfo], window: usize) {
        if let Some(pos) = self
            .entries(windows)
            .iter()
            .position(|e| matches!(e, TreeEntry::Window { window: w } if *w == window))
        {
            self.selected = pos;
        }
    }

    /// Move the selection to the row of a 1-based window number.
    pub fn jump_to_window(&mut self, windows: &[WindowInfo], number: usize) {
        if number >= 1 && number <= windows.len() {
            self.select_window(windows, number - 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wm::manager::PaneInfo;

    fn window(id: TabId, number: usize, pane_count: usize) -> WindowInfo {
        WindowInfo {
            id,
            number,
            name: format!("{}:shell", number),
            is_active: number == 1,
            is_last: false,
            panes: (1..=pane_count)
                .map(|n| PaneInfo {
                    number: n,
                    title: format!("Pane {}", n),
                    is_active: n == 1,
                })
                .collect(),
        }
    }

    #[test]
    fn expansion_flattens_panes_and_collapse_returns_to_window_row() {
        let windows = vec![window(10, 1, 2), window(20, 2, 3)];
        let mut sel = WindowSelector::new();
        sel.visible = true;

        assert_eq!(sel.entries(&windows).len(), 2);

        // Expand window 2 and walk into its panes
        sel.selected = 1;
        sel.expand(&windows);
        let entries = sel.entries(&windows);
        assert_eq!(entries.len(), 5);
        assert_eq!(entries[1], TreeEntry::Window { window: 1 });
        assert_eq!(entries[2], TreeEntry::Pane { window: 1, pane: 0 });
        assert_eq!(entries[4], TreeEntry::Pane { window: 1, pane: 2 });

        // Collapsing from a pane row folds and reselects the window row
        sel.selected = 3;
        sel.collapse(&windows);
        assert_eq!(sel.entries(&windows).len(), 2);
        assert_eq!(sel.selected, 1);
        assert!(!sel.is_expanded(20));
    }

    #[test]
    fn jump_targets_window_rows_even_when_earlier_windows_are_expanded() {
        let windows = vec![window(10, 1, 2), window(20, 2, 1)];
        let mut sel = WindowSelector::new();
        sel.selected = 0;
        sel.expand(&windows);

        // Window 2's row sits after window 1's two pane rows
        sel.jump_to_window(&windows, 2);
        assert_eq!(
            sel.selected_entry(&windows),
            Some(TreeEntry::Window { window: 1 })
        );
        assert_eq!(sel.selected, 3);

        // Out-of-range numbers leave the selection alone
        sel.jump_to_window(&windows, 9);
        assert_eq!(sel.selected, 3);
    }
}
