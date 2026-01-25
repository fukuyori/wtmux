//! Context menu for pane operations.
//!
//! This module provides a right-click context menu that allows users to perform
//! common pane operations without using keyboard shortcuts. The menu is especially
//! useful when a pane becomes unresponsive and keyboard input is not being processed.
//!
//! # Features
//!
//! - Mouse hover highlighting
//! - Keyboard navigation (↑/↓ or j/k)
//! - Automatic screen boundary detection
//!
//! # Example
//!
//! ```ignore
//! let mut menu = ContextMenu::new();
//! menu.show(pane_id, click_x, click_y, screen_width, screen_height);
//! 
//! // Handle keyboard input
//! menu.down();  // Move selection down
//! let action = menu.selected_action();
//! ```

use crate::wm::PaneId;

/// Actions that can be triggered from the context menu.
///
/// Each action corresponds to a common pane operation that would
/// otherwise require a keyboard shortcut (Ctrl+B prefix).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContextMenuAction {
    /// Paste clipboard content to the current pane.
    Paste,
    /// Kill (close) the current pane.
    /// Equivalent to `Ctrl+B, x`.
    KillPane,
    /// Split the pane horizontally (creates top/bottom panes).
    /// Equivalent to `Ctrl+B, "`.
    SplitHorizontal,
    /// Split the pane vertically (creates left/right panes).
    /// Equivalent to `Ctrl+B, %`.
    SplitVertical,
    /// Toggle zoom state of the current pane.
    /// Equivalent to `Ctrl+B, z`.
    ToggleZoom,
    /// Cancel and close the menu without taking action.
    Cancel,
}

/// A single item in the context menu.
#[derive(Debug, Clone)]
pub struct MenuItem {
    /// Display label for the menu item.
    pub label: &'static str,
    /// Action to execute when selected.
    pub action: ContextMenuAction,
    /// Optional keyboard shortcut hint (e.g., "z" for zoom).
    pub shortcut: Option<&'static str>,
}

impl MenuItem {
    /// Creates a new menu item.
    ///
    /// # Arguments
    ///
    /// * `label` - The text displayed in the menu
    /// * `action` - The action triggered when selected
    /// * `shortcut` - Optional shortcut key hint
    pub const fn new(label: &'static str, action: ContextMenuAction, shortcut: Option<&'static str>) -> Self {
        Self { label, action, shortcut }
    }
}

/// Context menu state and behavior.
///
/// The context menu appears when right-clicking on a pane and provides
/// quick access to common pane operations. It supports both mouse and
/// keyboard interaction.
pub struct ContextMenu {
    /// Whether the menu is currently visible.
    pub visible: bool,
    /// The pane ID this menu was opened for.
    pub target_pane: Option<PaneId>,
    /// X position of the menu (screen coordinates).
    pub x: u16,
    /// Y position of the menu (screen coordinates).
    pub y: u16,
    /// Index of the currently selected/highlighted item.
    pub selected: usize,
    /// List of menu items.
    pub items: Vec<MenuItem>,
}

impl Default for ContextMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextMenu {
    /// Creates a new context menu with default menu items.
    ///
    /// The default menu includes:
    /// - Paste (Ctrl+V)
    /// - Zoom/Unzoom (z)
    /// - Split ─ (") - horizontal split
    /// - Split │ (%) - vertical split
    /// - Kill Pane (x)
    /// - Cancel (Esc)
    pub fn new() -> Self {
        Self {
            visible: false,
            target_pane: None,
            x: 0,
            y: 0,
            selected: 0,
            items: vec![
                MenuItem::new("Paste", ContextMenuAction::Paste, Some("Ctrl+V")),
                MenuItem::new("Zoom/Unzoom", ContextMenuAction::ToggleZoom, Some("z")),
                MenuItem::new("Split ─", ContextMenuAction::SplitVertical, Some("\"")),
                MenuItem::new("Split │", ContextMenuAction::SplitHorizontal, Some("%")),
                MenuItem::new("Kill Pane", ContextMenuAction::KillPane, Some("x")),
                MenuItem::new("Cancel", ContextMenuAction::Cancel, Some("Esc")),
            ],
        }
    }

    /// Show the menu at the given position for the given pane
    /// screen_width and screen_height are used to ensure menu stays on screen
    pub fn show(&mut self, pane_id: PaneId, x: u16, y: u16, screen_width: u16, screen_height: u16) {
        self.visible = true;
        self.target_pane = Some(pane_id);
        self.selected = 0;
        
        // Adjust position to keep menu on screen
        let (width, height) = self.dimensions();
        self.x = x.min(screen_width.saturating_sub(width));
        self.y = y.min(screen_height.saturating_sub(height));
    }

    /// Hide the menu
    pub fn hide(&mut self) {
        self.visible = false;
        self.target_pane = None;
    }

    /// Move selection up
    pub fn up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        } else {
            self.selected = self.items.len() - 1;
        }
    }

    /// Move selection down
    pub fn down(&mut self) {
        if self.selected < self.items.len() - 1 {
            self.selected += 1;
        } else {
            self.selected = 0;
        }
    }

    /// Get the currently selected action
    pub fn selected_action(&self) -> ContextMenuAction {
        self.items[self.selected].action
    }

    /// Get menu content width (excluding borders)
    pub fn content_width(&self) -> u16 {
        self.items.iter()
            .map(|item| {
                let shortcut_len = item.shortcut.map(|s| s.chars().count() + 3).unwrap_or(0); // " (x)"
                item.label.chars().count() + shortcut_len + 2 // " label (x) "
            })
            .max()
            .unwrap_or(18) as u16
    }

    /// Get menu dimensions (including borders)
    pub fn dimensions(&self) -> (u16, u16) {
        let width = self.content_width() + 2; // +2 for left/right borders
        let height = self.items.len() as u16 + 2; // items + top/bottom border
        (width, height)
    }

    /// Check if a position is inside the menu content area (excluding borders)
    #[allow(dead_code)]
    pub fn contains(&self, col: u16, row: u16) -> bool {
        let (width, height) = self.dimensions();
        col >= self.x && col < self.x + width && row >= self.y && row < self.y + height
    }

    /// Handle click inside menu, returns action if item was clicked
    pub fn handle_click(&mut self, col: u16, row: u16) -> Option<ContextMenuAction> {
        let (width, height) = self.dimensions();
        
        // Check if click is inside menu bounds
        if col < self.x || col >= self.x + width || row < self.y || row >= self.y + height {
            return None;
        }
        
        // Check if click is on an item row (not on border)
        let relative_row = row.saturating_sub(self.y + 1); // +1 for top border
        let item_count = self.items.len() as u16;
        
        if relative_row < item_count {
            self.selected = relative_row as usize;
            return Some(self.selected_action());
        }
        
        None
    }

    /// Update selection based on mouse hover position
    /// Returns true if selection changed
    pub fn update_hover(&mut self, col: u16, row: u16) -> bool {
        let (width, height) = self.dimensions();
        
        // Check if inside menu bounds
        if col < self.x || col >= self.x + width || row < self.y || row >= self.y + height {
            return false;
        }
        
        // Check if on an item row
        let relative_row = row.saturating_sub(self.y + 1);
        let item_count = self.items.len() as u16;
        
        if relative_row < item_count {
            let new_selected = relative_row as usize;
            if new_selected != self.selected {
                self.selected = new_selected;
                return true;
            }
        }
        
        false
    }
}
