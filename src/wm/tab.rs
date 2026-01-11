//! Tab - A container for panes with a layout

use std::collections::HashMap;
use super::pane::{Pane, PaneId, BorderStyle};
use super::layout::{Layout, LayoutType, SplitDirection};

/// Unique identifier for a tab
pub type TabId = u64;

/// Reason for reflow (used for debugging and optimization)
#[derive(Debug, Clone, Copy)]
pub enum ReflowReason {
    Split,
    Close,
    ZoomToggle,
    FocusChanged,
    WindowResized,
    LayoutChanged,
}

/// A tab containing multiple panes
pub struct Tab {
    /// Unique identifier
    #[allow(dead_code)]
    pub id: TabId,
    /// Tab name
    pub name: String,
    /// Layout tree
    pub layout: Layout,
    /// All panes in this tab
    pub panes: HashMap<PaneId, Pane>,
    /// Pane order (for numbering and navigation)
    pub pane_order: Vec<PaneId>,
    /// Currently focused pane
    pub focused_pane: PaneId,
    /// Next pane ID
    next_pane_id: PaneId,
    /// Tab dimensions
    pub width: u16,
    pub height: u16,
    /// Zoomed pane (if any)
    zoomed_pane: Option<PaneId>,
    /// Current layout type
    current_layout: LayoutType,
    /// Layout generation (incremented on each reflow)
    pub layout_generation: u64,
}

impl Tab {
    /// Create a new tab with a single pane
    pub fn new(id: TabId, name: String, cols: u16, rows: u16) -> Self {
        let pane_id = 1;
        let mut pane = Pane::new_without_border(pane_id, cols, rows);
        pane.focused = true;
        
        let mut panes = HashMap::new();
        panes.insert(pane_id, pane);
        
        Self {
            id,
            name,
            layout: Layout::new(pane_id),
            panes,
            pane_order: vec![pane_id],
            focused_pane: pane_id,
            next_pane_id: 2,
            width: cols,
            height: rows,
            zoomed_pane: None,
            current_layout: LayoutType::Custom,
            layout_generation: 0,
        }
    }

    /// Split the current pane
    pub fn split(&mut self, direction: SplitDirection, shell_cmd: Option<&str>, codepage: Option<u32>) -> Option<PaneId> {
        // Unzoom if zoomed
        self.zoomed_pane = None;
        
        let new_pane_id = self.next_pane_id;
        self.next_pane_id += 1;
        
        // Split the layout
        if !self.layout.split(self.focused_pane, new_pane_id, direction) {
            return None;
        }
        
        // Recalculate positions
        let positions = self.layout.calculate_positions(0, 0, self.width, self.height);
        
        // Create new pane with calculated size
        let default_size = (new_pane_id, 0, 0, self.width / 2, self.height / 2);
        let (_, _, _, new_width, new_height) = positions.iter()
            .find(|(id, _, _, _, _)| *id == new_pane_id)
            .unwrap_or(&default_size);
        
        // Create pane with Single border (will be confirmed by reflow)
        let mut new_pane = Pane::new(new_pane_id, *new_width, *new_height);
        new_pane.border = BorderStyle::Single;
        
        // Start the session
        if let Err(e) = new_pane.session.start_with_codepage(shell_cmd, codepage) {
            eprintln!("Failed to start pane session: {}", e);
            return None;
        }
        
        self.panes.insert(new_pane_id, new_pane);
        self.pane_order.push(new_pane_id);
        
        // Reflow handles all geometry and border changes
        self.reflow(ReflowReason::Split);
        
        // Focus the new pane
        self.focus_pane(new_pane_id);
        
        Some(new_pane_id)
    }

    /// Close the focused pane
    pub fn close_pane(&mut self) -> bool {
        if self.panes.len() <= 1 {
            return false; // Can't close the last pane
        }
        
        let pane_id = self.focused_pane;
        
        // Unzoom if zoomed pane was closed
        if self.zoomed_pane == Some(pane_id) {
            self.zoomed_pane = None;
        }
        
        // Remove from layout
        if let Some(new_layout) = self.layout.remove(pane_id) {
            self.layout = new_layout;
        } else {
            return false;
        }
        
        // Remove pane
        self.panes.remove(&pane_id);
        self.pane_order.retain(|&id| id != pane_id);
        
        // Focus another pane
        if let Some(&new_focus) = self.panes.keys().next() {
            self.focus_pane(new_focus);
        }
        
        // Reflow handles all geometry and border changes
        self.reflow(ReflowReason::Close);
        
        true
    }

    /// Focus a specific pane
    pub fn focus_pane(&mut self, pane_id: PaneId) {
        // Check if zoom target will change
        let zoom_target_changed = self.zoomed_pane.is_some() && self.zoomed_pane != Some(pane_id);
        
        // If zoomed, update zoom target to follow focus
        if self.zoomed_pane.is_some() {
            self.zoomed_pane = Some(pane_id);
        }
        
        // Unfocus current
        if let Some(pane) = self.panes.get_mut(&self.focused_pane) {
            pane.focused = false;
        }
        
        // Focus new
        if let Some(pane) = self.panes.get_mut(&pane_id) {
            pane.focused = true;
            self.focused_pane = pane_id;
        }
        
        // If zoom target changed, reflow to update geometry
        if zoom_target_changed {
            self.reflow(ReflowReason::FocusChanged);
        }
    }

    /// Move focus in a direction
    pub fn focus_direction(&mut self, direction: SplitDirection, forward: bool) {
        if let Some(neighbor) = self.layout.find_neighbor(self.focused_pane, direction, forward) {
            self.focus_pane(neighbor);
        }
    }

    /// Get the focused pane
    pub fn focused_pane(&self) -> Option<&Pane> {
        self.panes.get(&self.focused_pane)
    }

    /// Get the focused pane mutably
    pub fn focused_pane_mut(&mut self) -> Option<&mut Pane> {
        self.panes.get_mut(&self.focused_pane)
    }

    /// Resize the tab
    /// Update all pane positions based on layout
    /// Reflow: apply layout, border, and resize to all panes
    /// This is the ONLY place that should modify pane geometry and border
    fn reflow(&mut self, reason: ReflowReason) {
        let _ = reason; // For future debugging/logging
        
        if let Some(zoomed_id) = self.zoomed_pane {
            // Zoomed mode: only the zoomed pane is visible at full size
            for (id, pane) in self.panes.iter_mut() {
                if *id == zoomed_id {
                    // Zoomed pane: full screen, no border
                    pane.apply_geometry(0, 0, self.width, self.height, BorderStyle::None);
                } else {
                    // Other panes: keep border for when unzoomed (geometry unchanged)
                    pane.border = BorderStyle::Single;
                }
            }
        } else {
            // Normal mode: apply layout positions
            let positions = self.layout.calculate_positions(0, 0, self.width, self.height);
            
            // Determine border style based on pane count
            let border = if self.panes.len() > 1 {
                BorderStyle::Single
            } else {
                BorderStyle::None
            };
            
            for (pane_id, x, y, width, height) in positions {
                if let Some(pane) = self.panes.get_mut(&pane_id) {
                    pane.apply_geometry(x, y, width, height, border);
                }
            }
        }
        
        // Increment generation to signal layout change
        self.layout_generation += 1;
    }

    /// Adjust pane size
    pub fn resize_pane(&mut self, delta: f32) {
        if self.layout.adjust_ratio(self.focused_pane, delta) {
            self.reflow(ReflowReason::LayoutChanged);
        }
    }

    /// Get pane at screen position
    pub fn pane_at(&self, col: u16, row: u16) -> Option<PaneId> {
        // If zoomed, only the zoomed pane is visible
        if let Some(zoomed_id) = self.zoomed_pane {
            return Some(zoomed_id);
        }
        
        for (id, pane) in &self.panes {
            if pane.contains(col, row) {
                return Some(*id);
            }
        }
        None
    }

    /// Process output for all panes
    pub fn process_output(&mut self) -> bool {
        let mut any_output = false;
        for pane in self.panes.values_mut() {
            if pane.session.process_output().unwrap_or(false) {
                any_output = true;
            }
        }
        any_output
    }

    /// Check if any pane is still running
    pub fn is_running(&self) -> bool {
        self.panes.values().any(|p| p.session.is_running())
    }

    /// Clean up dead panes (where shell has exited)
    pub fn cleanup_dead_panes(&mut self) {
        let dead_panes: Vec<PaneId> = self.panes
            .iter()
            .filter(|(_, pane)| !pane.session.is_running())
            .map(|(id, _)| *id)
            .collect();
        
        if dead_panes.is_empty() {
            return;
        }
        
        for pane_id in dead_panes {
            // Remove from layout
            if let Some(new_layout) = self.layout.remove(pane_id) {
                self.layout = new_layout;
            }
            self.panes.remove(&pane_id);
            self.pane_order.retain(|&id| id != pane_id);
            
            // Unzoom if zoomed pane was closed
            if self.zoomed_pane == Some(pane_id) {
                self.zoomed_pane = None;
            }
        }
        
        // Update focus if needed
        if !self.panes.contains_key(&self.focused_pane) {
            if let Some(&new_focus) = self.panes.keys().next() {
                self.focus_pane(new_focus);
            }
        }
        
        // Single reflow handles all geometry and border changes
        if !self.panes.is_empty() {
            self.reflow(ReflowReason::Close);
        }
    }

    /// Toggle zoom on focused pane
    pub fn toggle_zoom(&mut self) {
        if self.panes.len() <= 1 {
            return; // Nothing to zoom
        }
        
        if self.zoomed_pane.is_some() {
            // Unzoom
            self.zoomed_pane = None;
        } else {
            // Zoom the focused pane
            self.zoomed_pane = Some(self.focused_pane);
        }
        
        // reflow() handles all geometry and border changes
        self.reflow(ReflowReason::ZoomToggle);
    }

    /// Check if currently zoomed
    pub fn is_zoomed(&self) -> bool {
        self.zoomed_pane.is_some()
    }

    /// Get zoomed pane ID
    pub fn zoomed_pane_id(&self) -> Option<PaneId> {
        self.zoomed_pane
    }

    /// Resize the tab
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        self.reflow(ReflowReason::WindowResized);
    }

    /// Resize pane in a specific direction (tmux compatible)
    /// arrow_up_or_left: true = up/left arrow, false = down/right arrow
    pub fn resize_pane_direction(&mut self, direction: SplitDirection, arrow_up_or_left: bool) {
        if self.zoomed_pane.is_some() {
            return;
        }
        self.layout.resize_in_direction(self.focused_pane, direction, arrow_up_or_left);
        self.reflow(ReflowReason::LayoutChanged);
    }

    /// Swap current pane with next pane in order
    pub fn swap_pane_next(&mut self) {
        if self.pane_order.len() <= 1 {
            return;
        }
        
        let current_idx = self.pane_order.iter()
            .position(|&id| id == self.focused_pane)
            .unwrap_or(0);
        
        let next_idx = (current_idx + 1) % self.pane_order.len();
        
        // Swap in layout
        let other_id = self.pane_order[next_idx];
        self.layout.swap_panes(self.focused_pane, other_id);
        
        // Swap in order
        self.pane_order.swap(current_idx, next_idx);
        
        self.reflow(ReflowReason::LayoutChanged);
    }

    /// Swap current pane with previous pane in order
    pub fn swap_pane_prev(&mut self) {
        if self.pane_order.len() <= 1 {
            return;
        }
        
        let current_idx = self.pane_order.iter()
            .position(|&id| id == self.focused_pane)
            .unwrap_or(0);
        
        let prev_idx = if current_idx == 0 {
            self.pane_order.len() - 1
        } else {
            current_idx - 1
        };
        
        // Swap in layout
        let other_id = self.pane_order[prev_idx];
        self.layout.swap_panes(self.focused_pane, other_id);
        
        // Swap in order
        self.pane_order.swap(current_idx, prev_idx);
        
        self.reflow(ReflowReason::LayoutChanged);
    }

    /// Get focused pane index (for display)
    #[allow(dead_code)]
    pub fn focused_pane_index(&self) -> usize {
        self.pane_order.iter()
            .position(|&id| id == self.focused_pane)
            .unwrap_or(0)
    }

    /// Switch to next layout preset
    pub fn next_layout(&mut self) {
        if self.panes.len() <= 1 {
            return; // No layout change needed for single pane
        }
        
        // Unzoom if zoomed
        self.zoomed_pane = None;
        
        // Switch to next layout type
        self.current_layout = self.current_layout.next();
        
        // Rebuild layout with new type
        self.layout = Layout::from_preset(self.current_layout, &self.pane_order);
        
        // Update pane positions
        self.reflow(ReflowReason::LayoutChanged);
    }
}
