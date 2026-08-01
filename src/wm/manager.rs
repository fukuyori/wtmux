//! Window Manager - Core component for managing tabs and panes.
//!
//! This module provides tmux-like terminal multiplexing functionality,
//! allowing users to create multiple tabs (windows) and split panes within each tab.
//!
//! # Architecture
//!
//! ```text
//! WindowManager
//! ├── Tab 1
//! │   ├── Pane 1 (Session)
//! │   └── Pane 2 (Session)
//! ├── Tab 2
//! │   └── Pane 1 (Session)
//! └── Tab 3
//!     ├── Pane 1 (Session)
//!     ├── Pane 2 (Session)
//!     └── Pane 3 (Session)
//! ```
//!
//! # Features
//!
//! - Multiple tabs with independent pane layouts
//! - Horizontal and vertical pane splitting
//! - Pane zoom (fullscreen toggle)
//! - Mouse support for tab switching and pane focus
//! - tmux-compatible keybindings

use std::collections::HashMap;
use super::tab::{Tab, TabId};
use super::pane::{AgentState, PaneId};
use super::layout::{SplitDirection, SplitResizeTarget};

use crate::config::PrefixKey;

/// One row of the agent dashboard: a pane anywhere in the session with its
/// herdr-style state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentEntry {
    /// Zero-based window index (for `focus_pane_at`)
    pub window_index: usize,
    /// Zero-based pane index within the window (for `focus_pane_at`)
    pub pane_index: usize,
    /// 1-based display number of the window
    pub window_number: usize,
    /// Window name
    pub window_name: String,
    /// 1-based display number of the pane
    pub pane_number: usize,
    /// Pane title
    pub pane_title: String,
    /// Current agent state
    pub state: AgentState,
    /// Whether the pane carries an unacknowledged attention flag
    pub attention: bool,
    /// Whether this is the globally focused pane
    pub is_focused: bool,
}

/// An agent state transition on one pane, drained by the main loop to
/// dispatch `[hooks]` commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStateEvent {
    /// Tab (window) id the pane lives in
    pub tab_id: TabId,
    /// Pane id within the tab
    pub pane_id: PaneId,
    /// Window name at the time of the transition
    pub window_name: String,
    /// Pane title at the time of the transition
    pub pane_title: String,
    /// State before the transition
    pub from: AgentState,
    /// State after the transition
    pub to: AgentState,
}

/// Set the per-pane environment variables inherited by a pane's child
/// process. Called immediately before each spawn (spawns happen on the main
/// thread, so the process-global env is not racy). `WTMUX_PANE` lets tools
/// inside the pane — e.g. `wtmux report-state` run from an agent's hooks —
/// address their own pane.
pub(crate) fn set_pane_spawn_env(tab_id: TabId, pane_id: PaneId) {
    std::env::set_var("WTMUX_PANE", format!("{}.{}", tab_id, pane_id));
}

/// A pane entry within a window, for the window selector tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneInfo {
    /// 1-based display number within the window
    pub number: usize,
    /// Pane title
    pub title: String,
    /// Whether this is the window's focused pane
    pub is_active: bool,
}

/// A window (tab) entry for the window selector, in display order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    /// Tab id (stable across list-order changes)
    pub id: TabId,
    /// 1-based display number
    pub number: usize,
    /// Window name
    pub name: String,
    /// Whether this is the current window (tmux flag `*`)
    pub is_active: bool,
    /// Whether this was the previously active window (tmux flag `-`)
    pub is_last: bool,
    /// The window's panes in display order
    pub panes: Vec<PaneInfo>,
}

/// The central manager for all tabs and pane operations.
///
/// `WindowManager` is the top-level component that coordinates:
/// - Tab creation, switching, and deletion
/// - Pane splitting, resizing, and focus management
/// - Terminal resize handling
/// - Mouse event routing
///
/// # Example
///
/// ```ignore
/// let mut wm = WindowManager::new(80, 24, None, None);
/// wm.start()?;  // Start the initial shell session
///
/// // Create a new tab
/// wm.new_tab();
///
/// // Split the current pane
/// wm.split_horizontal();
/// ```
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
    /// Inject shell prompt hooks that publish cwd changes.
    pub cwd_prompt_hook: bool,
    /// Prefix key mode (like tmux Ctrl+b)
    pub prefix_mode: bool,
    /// Configured prefix key
    pub prefix_key: PrefixKey,
    /// Whether the current mouse selection actually moved past the down cell.
    mouse_selection_moved: bool,
    /// Active split boundary resize drag, if any.
    mouse_resize_drag: Option<SplitResizeTarget>,
    /// Pane activity monitor (busy / attention tracking) enabled
    pub activity_monitor: bool,
    /// Quiet period after background output before a pane is flagged
    pub quiet_threshold: std::time::Duration,
}

impl WindowManager {
    /// Create a new window manager
    pub fn new(
        width: u16,
        height: u16,
        shell: Option<String>,
        codepage: Option<u32>,
        prefix_key: PrefixKey,
        cwd_prompt_hook: bool,
    ) -> Self {
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
            cwd_prompt_hook,
            prefix_mode: false,
            prefix_key,
            mouse_selection_moved: false,
            mouse_resize_drag: None,
            activity_monitor: true,
            quiet_threshold: std::time::Duration::from_millis(2000),
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
            set_pane_spawn_env(tab_id, pane.id);
            let _ = pane.session.start_with_options(
                self.default_shell.as_deref(),
                self.default_codepage,
                self.cwd_prompt_hook,
            );
        }
        
        self.tabs.insert(tab_id, tab);
        self.tab_order.push(tab_id);
        self.last_active_tab = Some(self.active_tab);
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
        if num > 0 {
            self.select_tab_at(num - 1);
        }
    }

    /// Return the zero-based position of the active tab in display order.
    pub fn active_tab_index(&self) -> usize {
        self.tab_order
            .iter()
            .position(|&id| id == self.active_tab)
            .unwrap_or(0)
    }

    /// Switch to a tab by its zero-based position in display order.
    ///
    /// Returns true when the active tab changed.
    pub fn select_tab_at(&mut self, index: usize) -> bool {
        let Some(&tab_id) = self.tab_order.get(index) else {
            return false;
        };
        if tab_id == self.active_tab {
            return false;
        }

        self.last_active_tab = Some(self.active_tab);
        self.active_tab = tab_id;
        true
    }

    /// Switch to a tab by id (e.g. the tab under a mouse click).
    ///
    /// Returns true when the active tab changed.
    pub fn select_tab(&mut self, tab_id: TabId) -> bool {
        if tab_id == self.active_tab || !self.tabs.contains_key(&tab_id) {
            return false;
        }
        self.last_active_tab = Some(self.active_tab);
        self.active_tab = tab_id;
        true
    }

    /// Get the active tab
    /// Get mutable access to the active (focused) pane's session
    pub fn get_active_session_mut(&mut self) -> Option<&mut crate::core::session::Session> {
        let tab = self.active_tab_mut()?;
        let pane = tab.focused_pane_mut()?;
        Some(&mut pane.session)
    }

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
        let cwd_prompt_hook = self.cwd_prompt_hook;
        self.active_tab_mut()?.split(
            SplitDirection::Horizontal,
            shell.as_deref(),
            codepage,
            cwd_prompt_hook,
        )
    }

    /// Split the current pane vertically
    pub fn split_vertical(&mut self) -> Option<PaneId> {
        let shell = self.default_shell.clone();
        let codepage = self.default_codepage;
        let cwd_prompt_hook = self.cwd_prompt_hook;
        self.active_tab_mut()?.split(
            SplitDirection::Vertical,
            shell.as_deref(),
            codepage,
            cwd_prompt_hook,
        )
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

    /// Rename the focused pane. An empty name restores the default title.
    pub fn rename_focused_pane(&mut self, name: &str) {
        if let Some(pane) = self.active_tab_mut().and_then(|tab| tab.focused_pane_mut()) {
            pane.title = if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            };
        }
    }

    /// Custom title of the focused pane, if one has been set.
    pub fn focused_pane_title(&self) -> Option<String> {
        self.active_tab()
            .and_then(|tab| tab.panes.get(&tab.focused_pane))
            .and_then(|pane| pane.title.clone())
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

    /// Process output for all tabs and handle closed panes.
    /// Returns true when pane content or tab layout changed and a render is needed.
    pub fn process_output(&mut self) -> bool {
        let mut changed = false;
        let tabs_to_check: Vec<TabId> = self.tabs.keys().cloned().collect();
        
        for tab_id in tabs_to_check.iter() {
            let is_active = *tab_id == self.active_tab;
            if let Some(tab) = self.tabs.get_mut(tab_id) {
                if tab.process_output(is_active) {
                    changed = true;
                }
                if self.activity_monitor && tab.update_activity(is_active, self.quiet_threshold) {
                    changed = true;
                }
                // Clean up dead panes
                if tab.cleanup_dead_panes() {
                    changed = true;
                }
            }
        }
        
        // Remove empty tabs
        let empty_tabs: Vec<TabId> = self.tabs.iter()
            .filter(|(_, tab)| tab.panes.is_empty())
            .map(|(id, _)| *id)
            .collect();
        for tab_id in empty_tabs {
            self.tabs.remove(&tab_id);
            self.tab_order.retain(|&id| id != tab_id);
            changed = true;
        }
        
        // Update active tab if needed
        if !self.tabs.contains_key(&self.active_tab) {
            if let Some(&new_active) = self.tab_order.first() {
                self.active_tab = new_active;
                self.force_full_redraw();
                changed = true;
            }
        }
        
        changed
    }

    /// Check if any tab is still running
    pub fn is_running(&self) -> bool {
        !self.tabs.is_empty() && self.tabs.values().any(|t| t.is_running())
    }

    /// Clear dirty-line tracking on all panes after a render pass.
    ///
    /// Call this after every render so the next frame only redraws rows that
    /// have actually changed since the last paint, not the entire screen.
    pub fn clear_all_dirty(&mut self) {
        for tab in self.tabs.values_mut() {
            for pane in tab.panes.values_mut() {
                // A settling pane was skipped by the renderer; keep its dirty
                // lines so the deferred paint after the resize replay still
                // covers everything that changed.
                if pane.session.is_settling() {
                    continue;
                }
                pane.session.state.active_screen_mut().clear_dirty();
            }
        }
    }

    /// Force a full redraw of all panes on the next render.
    ///
    /// Used when an overlay (history selector, context menu, etc.) is closed
    /// and the underlying pane content must be repainted to clear the overlay.
    pub fn force_full_redraw(&mut self) {
        for tab in self.tabs.values_mut() {
            for pane in tab.panes.values_mut() {
                pane.session.state.active_screen_mut().full_redraw = true;
            }
        }
    }

    /// Get tab info for rendering tab bar
    /// Tab bar entries: (id, display name, is_active, needs_attention).
    ///
    /// The display name carries the activity marker (`!` = a pane needs
    /// attention, `*` = a pane is producing output) so that width-based hit
    /// testing (`tab_at_position`) stays consistent with rendering.
    pub fn tab_info(&self) -> Vec<(TabId, String, bool, bool)> {
        self.tab_order.iter().filter_map(|&id| {
            // Skip ids that are missing from the map rather than panicking if
            // tab_order and tabs ever get out of sync
            let tab = self.tabs.get(&id)?;
            let attention = tab
                .panes
                .values()
                .any(|pane| pane.activity.attention().is_some());
            let busy = tab.panes.values().any(|pane| pane.activity.is_busy());
            let marker = if attention {
                "!"
            } else if busy && id != self.active_tab {
                "*"
            } else {
                ""
            };
            Some((
                id,
                format!("{}{}", tab.name, marker),
                id == self.active_tab,
                attention,
            ))
        }).collect()
    }

    /// Get window information in display order for the window selector.
    pub fn window_info(&self) -> Vec<WindowInfo> {
        self.tab_order
            .iter()
            .enumerate()
            .filter_map(|(index, &id)| {
                let tab = self.tabs.get(&id)?;
                let panes = tab
                    .pane_order
                    .iter()
                    .enumerate()
                    .filter_map(|(pane_index, &pane_id)| {
                        let pane = tab.panes.get(&pane_id)?;
                        Some(PaneInfo {
                            number: pane_index + 1,
                            title: pane.display_title(),
                            is_active: pane_id == tab.focused_pane,
                        })
                    })
                    .collect();
                Some(WindowInfo {
                    id,
                    number: index + 1,
                    name: tab.name.clone(),
                    is_active: id == self.active_tab,
                    is_last: self.last_active_tab == Some(id) && id != self.active_tab,
                    panes,
                })
            })
            .collect()
    }

    /// Switch to the window at a display-order position and focus the pane
    /// at the given display-order position within it.
    pub fn focus_pane_at(&mut self, window_index: usize, pane_index: usize) -> bool {
        let Some(&tab_id) = self.tab_order.get(window_index) else {
            return false;
        };
        let Some(tab) = self.tabs.get_mut(&tab_id) else {
            return false;
        };
        let Some(&pane_id) = tab.pane_order.get(pane_index) else {
            return false;
        };

        tab.focus_pane(pane_id);
        if tab_id != self.active_tab {
            self.last_active_tab = Some(self.active_tab);
            self.active_tab = tab_id;
        }
        true
    }

    /// Close the pane at a display-order position within the window at the
    /// given display-order position. Closing a window's last pane closes the
    /// window itself (keeping at least one window open).
    pub fn close_pane_at(&mut self, window_index: usize, pane_index: usize) -> bool {
        let Some(&tab_id) = self.tab_order.get(window_index) else {
            return false;
        };
        let Some(tab) = self.tabs.get_mut(&tab_id) else {
            return false;
        };
        if tab.panes.len() <= 1 {
            return self.close_tab_at(window_index);
        }
        let Some(&pane_id) = tab.pane_order.get(pane_index) else {
            return false;
        };
        tab.close_pane_by_id(pane_id)
    }

    /// Get the tab at a zero-based display-order position (for previews).
    pub fn tab_at(&self, index: usize) -> Option<&Tab> {
        let id = self.tab_order.get(index)?;
        self.tabs.get(id)
    }

    /// Close the tab at a zero-based display-order position.
    ///
    /// Keeps at least one tab open. Returns true when a tab was removed.
    pub fn close_tab_at(&mut self, index: usize) -> bool {
        if self.tabs.len() <= 1 {
            return false; // Keep at least one tab
        }
        let Some(&tab_id) = self.tab_order.get(index) else {
            return false;
        };

        self.tabs.remove(&tab_id);
        self.tab_order.retain(|&id| id != tab_id);

        if self.last_active_tab == Some(tab_id) {
            self.last_active_tab = None;
        }
        if self.active_tab == tab_id {
            let new_index = index.min(self.tab_order.len().saturating_sub(1));
            if let Some(&new_active) = self.tab_order.get(new_index) {
                self.active_tab = new_active;
            }
        }

        true
    }

    /// Get status info for rendering status bar
    pub fn status_info(&self) -> String {
        if let Some(tab) = self.active_tab() {
            let pane_count = tab.panes.len();
            let focused_id = tab.focused_pane;
            let zoom_indicator = if tab.is_zoomed() { " [Z]" } else { "" };
            let sync_indicator = if tab.broadcast { " [SYNC]" } else { "" };
            let log_indicator = if tab
                .panes
                .get(&focused_id)
                .is_some_and(|pane| pane.session.pipe_log_active())
            {
                " [LOG]"
            } else {
                ""
            };
            // Agent state summary, e.g. " | 2W 1B 1D" (working/blocked/done)
            let (working, blocked, done) = self.agent_state_counts();
            let mut agents = String::new();
            if working + blocked + done > 0 {
                agents.push_str(" |");
                if working > 0 {
                    agents.push_str(&format!(" {}W", working));
                }
                if blocked > 0 {
                    agents.push_str(&format!(" {}B", blocked));
                }
                if done > 0 {
                    agents.push_str(&format!(" {}D", done));
                }
            }
            format!(
                "[{}] {}:{} | Pane {}/{}{}{}{}{}",
                self.active_tab,
                tab.name,
                focused_id,
                focused_id,
                pane_count,
                zoom_indicator,
                sync_indicator,
                log_indicator,
                agents
            )
        } else {
            "No active tab".to_string()
        }
    }

    /// Find which tab is at a given column position on the tab bar
    /// Returns Some(TabId) if a tab was clicked, None otherwise
    pub fn tab_at_position(&self, col: u16) -> Option<TabId> {
        let tabs = self.tab_info();
        let mut x: u16 = 0;
        
        for (i, (id, name, _active, _attention)) in tabs.iter().enumerate() {
            // Tab format: " name " with separator "│"
            let tab_width = name.chars().count() as u16 + 2; // " name "
            
            if col >= x && col < x + tab_width {
                return Some(*id);
            }
            
            x += tab_width;
            if i + 1 < tabs.len() {
                x += 1; // separator "│"
            }
        }
        
        None
    }

    /// Return the clickable tab-bar range for the new-tab button.
    pub fn new_tab_button_range(&self) -> Option<std::ops::Range<u16>> {
        let tabs = self.tab_info();
        let mut x: u16 = 0;

        for (i, (_id, name, _active, _attention)) in tabs.iter().enumerate() {
            x = x.saturating_add(name.chars().count() as u16 + 2);
            if i + 1 < tabs.len() {
                x = x.saturating_add(1);
            }
        }

        let start = if tabs.is_empty() {
            0
        } else {
            x.saturating_add(1)
        };
        let width = 3; // "[+]"
        let end = start.saturating_add(width);

        (end <= self.width).then_some(start..end)
    }

    pub fn is_new_tab_button_at_position(&self, col: u16) -> bool {
        self.new_tab_button_range()
            .is_some_and(|range| range.contains(&col))
    }

    /// Handle tab bar click - switches to clicked tab
    /// Returns true if tab changed or a new tab was created.
    pub fn handle_tab_click(&mut self, col: u16) -> bool {
        if self.is_new_tab_button_at_position(col) {
            self.new_tab();
            return true;
        }

        if let Some(tab_id) = self.tab_at_position(col) {
            if tab_id != self.active_tab {
                self.last_active_tab = Some(self.active_tab);
                self.active_tab = tab_id;
                return true;
            }
        }
        false
    }

    /// Handle mouse down at position (start selection)
    /// Returns true if focus changed to a different pane
    pub fn handle_mouse_down(&mut self, col: u16, row: u16) -> bool {
        self.mouse_selection_moved = false;
        if self.mouse_resize_drag.take().is_some() {
            // Lost the matching mouse-up (e.g. dropped event): make sure no
            // pane is left with PTY resizes deferred.
            self.end_pty_resize_deferral();
        }

        // Check if click is on tab bar
        if row < self.tab_bar_height {
            return self.handle_tab_click(col);
        }

        // Adjust row for content area
        let content_row = row - self.tab_bar_height;

        let resize_target = self.active_tab_mut().and_then(|tab| {
            let target = tab.split_resize_target_at(col, content_row)?;
            // Defer PTY resizes for the drag; flushed on mouse-up.
            tab.set_pty_resize_deferred(true);
            Some(target)
        });
        if let Some(target) = resize_target {
            self.mouse_resize_drag = Some(target);
            return false;
        }
        
        // Find pane at position and focus it
        if let Some(tab) = self.active_tab_mut() {
            let old_focus = tab.focused_pane;
            if let Some(pane_id) = tab.pane_at(col, content_row) {
                tab.focus_pane(pane_id);
                
                // Start selection in that pane
                if let Some(pane) = tab.panes.get_mut(&pane_id) {
                    let (inner_x, inner_y) = pane.inner_pos();
                    let pane_col = col.saturating_sub(inner_x);
                    let pane_row = content_row.saturating_sub(inner_y);
                    pane.session.state.start_selection(pane_col, pane_row);
                }
                
                // Return true if focus changed
                return old_focus != pane_id;
            }
        }
        false
    }

    /// Handle right click at position
    /// Returns Some((pane_id, pane_local_col, pane_local_row)) if clicked on a pane
    pub fn handle_right_click(&mut self, col: u16, row: u16) -> Option<(PaneId, u16, u16)> {
        // Ignore clicks on tab bar
        if row < self.tab_bar_height {
            return None;
        }
        
        let content_row = row - self.tab_bar_height;
        
        if let Some(tab) = self.active_tab_mut() {
            if let Some(pane_id) = tab.pane_at(col, content_row) {
                // Focus the pane
                tab.focus_pane(pane_id);
                
                // Clear any selection
                if let Some(pane) = tab.panes.get_mut(&pane_id) {
                    pane.session.state.clear_selection();
                }
                
                return Some((pane_id, col, row));
            }
        }
        None
    }

    /// Pane whose title row (top border) is at the given screen position.
    pub fn pane_title_at(&self, col: u16, row: u16) -> Option<PaneId> {
        if row < self.tab_bar_height {
            return None;
        }
        let content_row = row - self.tab_bar_height;
        let tab = self.active_tab()?;
        let pane_id = tab.pane_at(col, content_row)?;
        let pane = tab.panes.get(&pane_id)?;
        (content_row == pane.y).then_some(pane_id)
    }

    /// Handle mouse drag (extend selection)
    pub fn handle_mouse_drag(&mut self, col: u16, row: u16) {
        if let Some(target) = self.mouse_resize_drag.clone() {
            if row < self.tab_bar_height {
                return;
            }
            let content_row = row - self.tab_bar_height;
            if let Some(tab) = self.active_tab_mut() {
                if tab.resize_split_to(&target, col, content_row) {
                    self.mouse_selection_moved = false;
                }
            }
            return;
        }

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
                if let Some(selection) = pane.session.state.selection.as_ref() {
                    self.mouse_selection_moved |= selection.start != selection.end;
                }
            }
        }
    }

    /// Flush deferred PTY resizes on every pane (drag end / recovery).
    fn end_pty_resize_deferral(&mut self) {
        for tab in self.tabs.values_mut() {
            tab.set_pty_resize_deferred(false);
        }
    }

    /// Handle mouse up (end selection and copy)
    pub fn handle_mouse_up(&mut self) -> Option<String> {
        if self.mouse_resize_drag.take().is_some() {
            self.mouse_selection_moved = false;
            self.end_pty_resize_deferral();
            return None;
        }

        let mouse_selection_moved = self.mouse_selection_moved;
        self.mouse_selection_moved = false;

        if let Some(tab) = self.active_tab_mut() {
            if let Some(pane) = tab.focused_pane_mut() {
                if !mouse_selection_moved {
                    pane.session.state.clear_selection();
                    return None;
                }

                let text = pane.session.state.get_selected_text();
                pane.session.state.clear_selection();
                return text;
            }
        }
        None
    }

    /// Returns true if a screen coordinate is on a split boundary.
    pub fn is_split_resize_target(&self, col: u16, row: u16) -> bool {
        if row < self.tab_bar_height || row >= self.height.saturating_sub(self.status_bar_height) {
            return false;
        }

        let content_row = row - self.tab_bar_height;
        self.tabs
            .get(&self.active_tab)
            .and_then(|tab| tab.split_resize_target_at(col, content_row))
            .is_some()
    }

    /// Returns true while the user is dragging a split boundary.
    pub fn is_resizing_split(&self) -> bool {
        self.mouse_resize_drag.is_some()
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

    /// Scroll the focused pane to the top of its scrollback history.
    pub fn scroll_to_top(&mut self) {
        if let Some(state) = self.focused_state_mut() {
            let screen = state.active_screen_mut();
            screen.scroll_offset = screen.scrollback.len();
            screen.mark_all_dirty();
        }
    }

    /// Terminal state of the focused pane in the active tab.
    pub fn focused_state(&self) -> Option<&crate::core::term::TerminalState> {
        self.active_tab()?
            .focused_pane()
            .map(|pane| &pane.session.state)
    }

    /// Mutable terminal state of the focused pane in the active tab.
    pub fn focused_state_mut(&mut self) -> Option<&mut crate::core::term::TerminalState> {
        self.active_tab_mut()?
            .focused_pane_mut()
            .map(|pane| &mut pane.session.state)
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
        let cwd_prompt_hook = self.cwd_prompt_hook;
        let tab_id = self.active_tab;
        if let Some(tab) = self.active_tab_mut() {
            if let Some(pane) = tab.focused_pane_mut() {
                set_pane_spawn_env(tab_id, pane.id);
                pane.session.start_with_options(
                    shell.as_deref(),
                    codepage,
                    cwd_prompt_hook,
                ).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// Write to the focused pane — or, with input broadcast enabled on the
    /// active window, to every pane in it.
    pub fn write(&mut self, data: &[u8]) -> Result<(), String> {
        if let Some(tab) = self.active_tab_mut() {
            if tab.broadcast {
                // Dead panes are cleaned up elsewhere; don't let one failed
                // write stop the broadcast to the remaining panes.
                for pane in tab.panes.values_mut() {
                    let _ = pane.session.write(data);
                }
            } else if let Some(pane) = tab.focused_pane_mut() {
                pane.session.write(data).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
    
    /// Paste text to the focused pane with bracketed paste support.
    /// With input broadcast enabled, pastes into every pane of the active
    /// window, honoring each pane's own bracketed-paste mode.
    pub fn paste(&mut self, text: &str) -> Result<(), String> {
        // Normalise all line endings to CR only.
        // Terminals interpret CR as Enter (one keypress).
        // CRLF would be two characters and some shells (PowerShell) treat
        // them as two separate newlines, causing double-submit.
        let normalized = text.replace("\r\n", "\r").replace('\n', "\r");

        let wrap = |bracketed: bool| {
            if bracketed {
                format!("\x1b[200~{}\x1b[201~", normalized).into_bytes()
            } else {
                normalized.clone().into_bytes()
            }
        };

        if let Some(tab) = self.active_tab_mut() {
            if tab.broadcast {
                for pane in tab.panes.values_mut() {
                    let bytes = wrap(pane.session.state.modes.bracketed_paste);
                    let _ = pane.session.write(&bytes);
                }
            } else if let Some(pane) = tab.focused_pane_mut() {
                let bytes = wrap(pane.session.state.modes.bracketed_paste);
                pane.session.write(&bytes).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }

    /// Toggle input broadcast (synchronize-panes) on the active window.
    /// Returns the new state.
    pub fn toggle_broadcast(&mut self) -> bool {
        if let Some(tab) = self.active_tab_mut() {
            tab.broadcast = !tab.broadcast;
            tab.broadcast
        } else {
            false
        }
    }

    /// Whether input broadcast is enabled on the active window.
    /// Only exercised by tests right now (the renderer reads the flag via
    /// `status_info`), hence the allow.
    #[allow(dead_code)]
    pub fn broadcast_active(&self) -> bool {
        self.active_tab().map(|tab| tab.broadcast).unwrap_or(false)
    }

    /// One row of the agent dashboard: a pane anywhere in the session with
    /// its herdr-style state.
    pub fn agent_overview(&self) -> Vec<AgentEntry> {
        let mut out = Vec::new();
        for (window_index, tab_id) in self.tab_order.iter().enumerate() {
            let Some(tab) = self.tabs.get(tab_id) else {
                continue;
            };
            for (pane_index, pane_id) in tab.pane_order.iter().enumerate() {
                let Some(pane) = tab.panes.get(pane_id) else {
                    continue;
                };
                out.push(AgentEntry {
                    window_index,
                    pane_index,
                    window_number: window_index + 1,
                    window_name: tab.name.clone(),
                    pane_number: pane_index + 1,
                    pane_title: pane.display_title(),
                    state: pane.activity.state(),
                    attention: pane.activity.attention().is_some(),
                    is_focused: *tab_id == self.active_tab && *pane_id == tab.focused_pane,
                });
            }
        }
        out
    }

    /// Drain pending agent state transitions across all panes.
    /// Each transition is returned exactly once; the main loop uses this to
    /// dispatch `[hooks]` commands.
    pub fn drain_agent_state_events(&mut self) -> Vec<AgentStateEvent> {
        let mut out = Vec::new();
        for &tab_id in &self.tab_order {
            let Some(tab) = self.tabs.get_mut(&tab_id) else {
                continue;
            };
            let window_name = tab.name.clone();
            for &pane_id in &tab.pane_order {
                let Some(pane) = tab.panes.get_mut(&pane_id) else {
                    continue;
                };
                if let Some((from, to)) = pane.activity.take_state_change() {
                    out.push(AgentStateEvent {
                        tab_id,
                        pane_id,
                        window_name: window_name.clone(),
                        pane_title: pane.display_title(),
                        from,
                        to,
                    });
                }
            }
        }
        out
    }

    /// Apply an agent state reported from outside via `wtmux report-state`.
    /// Returns true when the pane exists and a render is needed.
    pub fn apply_reported_state(
        &mut self,
        tab_id: TabId,
        pane_id: PaneId,
        state: AgentState,
    ) -> bool {
        let active_tab = self.active_tab;
        let Some(tab) = self.tabs.get_mut(&tab_id) else {
            return false;
        };
        let focused = tab_id == active_tab && pane_id == tab.focused_pane;
        let Some(pane) = tab.panes.get_mut(&pane_id) else {
            return false;
        };
        pane.activity.report_state(state, focused);
        // Repaint so the border / dashboard reflect the reported state
        pane.session.state.active_screen_mut().full_redraw = true;
        true
    }

    /// Apply a specific layout preset to the active window (select-layout).
    pub fn set_layout_preset(&mut self, layout: crate::wm::layout::LayoutType) {
        if let Some(tab) = self.active_tab_mut() {
            tab.set_layout(layout);
        }
    }

    /// Resolve a `<window>.<pane>` target string (as published in
    /// `WTMUX_PANE`) to concrete ids; `None` targets the focused pane of the
    /// active window.
    pub fn resolve_target_pane(&self, target: Option<&str>) -> Result<(TabId, PaneId), String> {
        let (tab_id, pane_id) = match target {
            None => {
                let tab_id = self.active_tab;
                let tab = self.tabs.get(&tab_id).ok_or("no active window")?;
                (tab_id, tab.focused_pane)
            }
            Some(t) => t
                .split_once('.')
                .and_then(|(w, p)| Some((w.parse().ok()?, p.parse().ok()?)))
                .ok_or_else(|| format!("invalid target {t:?}: expected <window>.<pane>"))?,
        };

        let tab = self
            .tabs
            .get(&tab_id)
            .ok_or_else(|| format!("window {tab_id} not found"))?;
        if !tab.panes.contains_key(&pane_id) {
            return Err(format!("pane {tab_id}.{pane_id} not found"));
        }
        Ok((tab_id, pane_id))
    }

    /// Map an `agent_overview` entry's indices to concrete ids.
    pub fn pane_ids_at(&self, window_index: usize, pane_index: usize) -> Option<(TabId, PaneId)> {
        let tab_id = *self.tab_order.get(window_index)?;
        let tab = self.tabs.get(&tab_id)?;
        let pane_id = *tab.pane_order.get(pane_index)?;
        Some((tab_id, pane_id))
    }

    /// Send a composed message to a pane and submit it with Enter.
    ///
    /// Newlines become carriage returns (terminal paste semantics). When the
    /// pane's application has enabled bracketed paste (DECSET 2004) the body
    /// is wrapped in paste markers, so multi-line text reaches TUIs like
    /// Claude Code as one literal block instead of per-line submissions.
    pub fn send_message_to_pane(
        &mut self,
        tab_id: TabId,
        pane_id: PaneId,
        text: &str,
    ) -> Result<(), String> {
        let pane = self
            .tabs
            .get_mut(&tab_id)
            .and_then(|tab| tab.panes.get_mut(&pane_id))
            .ok_or_else(|| format!("pane {tab_id}.{pane_id} not found"))?;
        let body = text.replace('\n', "\r");
        let mut bytes = Vec::with_capacity(body.len() + 16);
        if pane.session.state.modes.bracketed_paste {
            bytes.extend_from_slice(b"\x1b[200~");
            bytes.extend_from_slice(body.as_bytes());
            bytes.extend_from_slice(b"\x1b[201~");
        } else {
            bytes.extend_from_slice(body.as_bytes());
        }
        bytes.push(b'\r');
        pane.session.write(&bytes).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Write raw bytes to a specific pane's PTY (send-keys).
    pub fn write_to_pane(
        &mut self,
        tab_id: TabId,
        pane_id: PaneId,
        bytes: &[u8],
    ) -> Result<(), String> {
        let pane = self
            .tabs
            .get_mut(&tab_id)
            .and_then(|tab| tab.panes.get_mut(&pane_id))
            .ok_or_else(|| format!("pane {tab_id}.{pane_id} not found"))?;
        pane.session.write(bytes).map_err(|e| e.to_string())?;
        Ok(())
    }

    /// Capture a pane's text content (capture-pane): the visible screen, or
    /// the full scrollback plus the screen. Trailing blank lines and
    /// per-line trailing spaces are trimmed.
    pub fn capture_pane_text(
        &self,
        tab_id: TabId,
        pane_id: PaneId,
        include_scrollback: bool,
    ) -> Result<String, String> {
        let pane = self
            .tabs
            .get(&tab_id)
            .and_then(|tab| tab.panes.get(&pane_id))
            .ok_or_else(|| format!("pane {tab_id}.{pane_id} not found"))?;

        let row_text = |row: &crate::core::term::Row| -> String {
            row.cells
                .iter()
                .filter(|c| !c.is_continuation())
                .map(|c| c.grapheme.as_str())
                .collect::<String>()
                .trim_end()
                .to_string()
        };

        let screen = pane.session.state.active_screen();
        let mut lines: Vec<String> = Vec::new();
        if include_scrollback {
            lines.extend(screen.scrollback.iter().map(row_text));
        }
        lines.extend(screen.rows.iter().map(row_text));

        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        Ok(lines.join("\n"))
    }

    /// Set input broadcast (synchronize-panes) on the active window to an
    /// explicit value. Returns the resulting state.
    pub fn set_broadcast(&mut self, enabled: bool) -> bool {
        if let Some(tab) = self.active_tab_mut() {
            tab.broadcast = enabled;
            tab.broadcast
        } else {
            false
        }
    }

    /// Toggle pipe-pane style output logging on the focused pane.
    /// Returns `(enabled, path)` on success, or None when logging could not
    /// be started (no data dir, file error, no focused pane).
    pub fn toggle_pipe_log(&mut self) -> Option<(bool, std::path::PathBuf)> {
        let tab_id = self.active_tab;
        let tab = self.tabs.get_mut(&tab_id)?;
        let pane_id = tab.focused_pane;
        let pane = tab.panes.get_mut(&pane_id)?;

        if pane.session.pipe_log_active() {
            let path = pane.session.stop_pipe_log()?;
            return Some((false, path));
        }

        let epoch_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let path = crate::config::get_data_dir()?
            .join("logs")
            .join(format!(
                "wtmux-{}-{}.{}-{}.log",
                std::process::id(),
                tab_id,
                pane_id,
                epoch_secs
            ));
        pane.session.start_pipe_log(&path).ok()?;
        Some((true, path))
    }

    /// Count panes per agent state: (working, blocked, done).
    /// Idle panes are not counted.
    pub fn agent_state_counts(&self) -> (usize, usize, usize) {
        let (mut working, mut blocked, mut done) = (0, 0, 0);
        for tab in self.tabs.values() {
            for pane in tab.panes.values() {
                match pane.activity.state() {
                    AgentState::Working => working += 1,
                    AgentState::Blocked => blocked += 1,
                    AgentState::Done => done += 1,
                    AgentState::Idle => {}
                }
            }
        }
        (working, blocked, done)
    }

    /// Focus the next pane (searching forward from the current focus, across
    /// windows) that is flagged as needing attention. Returns true if focus
    /// moved.
    pub fn focus_next_attention(&mut self) -> bool {
        // Flatten all panes into (window_index, pane_index) in display order
        let mut flat: Vec<(usize, usize, bool)> = Vec::new();
        let mut current = 0usize;
        for (w, tab_id) in self.tab_order.iter().enumerate() {
            let Some(tab) = self.tabs.get(tab_id) else {
                continue;
            };
            for (p, pane_id) in tab.pane_order.iter().enumerate() {
                let attention = tab
                    .panes
                    .get(pane_id)
                    .is_some_and(|pane| pane.activity.attention().is_some());
                if *tab_id == self.active_tab && *pane_id == tab.focused_pane {
                    current = flat.len();
                }
                flat.push((w, p, attention));
            }
        }

        // Search forward from the pane after the current one, wrapping around
        for offset in 1..=flat.len() {
            let (w, p, attention) = flat[(current + offset) % flat.len()];
            if attention {
                return self.focus_pane_at(w, p);
            }
        }
        false
    }

    /// Paste from system clipboard to the focused pane
    pub fn paste_from_clipboard(&mut self) -> Result<(), String> {
        let text = arboard::Clipboard::new()
            .and_then(|mut clipboard| clipboard.get_text())
            .map_err(|e| e.to_string())?;

        if !text.is_empty() {
            self.paste(&text)?;
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

    /// Get the current command text for history recording.
    ///
    /// Priority order:
    ///
    /// 1. **OSC 133/633 confirmed command** – the shell sent a marker C just
    ///    before Enter, so the command text was extracted at that exact moment.
    ///    This works regardless of prompt appearance (oh-my-posh, Starship, …).
    ///
    /// 2. **OSC 133/633 prompt-end position** – we know where the prompt ended
    ///    (marker B), so we can read the text to the right of that column even
    ///    if marker C was not received.
    ///
    /// 3. **Keystroke tracker** – for shells without OSC support (cmd.exe).
    ///    We intercepted every key before forwarding it to the PTY, so the
    ///    buffer contains exactly what the user typed.
    ///
    /// 4. **strip_prompt fallback** – the original heuristic, kept as a last
    ///    resort for unusual configurations.
    pub fn get_current_line(&self) -> Option<String> {
        let tab = self.active_tab()?;
        let pane = tab.focused_pane()?;
        let si = &pane.session.state.shell_integration;

        // ── Priority 1: OSC marker C confirmed command ────────────────────
        if let Some(cmd) = &si.confirmed_command {
            let trimmed = cmd.trim().to_string();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }

        // ── Priority 2: OSC marker B prompt-end position ──────────────────
        if si.active {
            if let (Some(prompt_col), Some(prompt_row)) =
                (si.prompt_end_col, si.prompt_end_row)
            {
                let screen = pane.session.state.active_screen();
                let cursor = pane.session.state.active_cursor();
                let prompt_abs_row = screen.screen_to_buffer_row(prompt_row as usize);
                let cursor_abs_row = screen.screen_to_buffer_row(cursor.row as usize);
                if let Some(_line) = screen.logical_line_at_absolute(prompt_abs_row) {
                    let cmd = screen.collect_text_between(
                        (prompt_abs_row, prompt_col as usize),
                        (cursor_abs_row, cursor.col as usize),
                    );
                    let trimmed = cmd.trim().to_string();
                    if !trimmed.is_empty() {
                        return Some(trimmed);
                    }
                }
            }
        }

        // ── Priority 3: keystroke tracker (cmd.exe fallback) ──────────────
        let kt_cmd = pane.session.state.keystroke_tracker.peek().trim().to_string();
        if !kt_cmd.is_empty() {
            return Some(kt_cmd);
        }

        // ── Priority 4: strip_prompt heuristic (last resort) ──────────────
        let cursor = pane.session.state.active_cursor();
        let screen = pane.session.state.active_screen();
        let line = screen.logical_line_at_visible(cursor.row as usize)?.text();
        Some(crate::history::strip_prompt(line.trim_end()))
    }

    /// Consume the shell-integration confirmed command (called after
    /// recording it to history so it is not recorded twice).
    pub fn take_confirmed_command(&mut self) -> Option<String> {
        let tab = self.tabs.get_mut(&self.active_tab)?;
        let pane = tab.focused_pane_mut()?;
        pane.session.state.shell_integration.take_confirmed_command()
    }

    /// Feed a printable character to the keystroke tracker of the active pane.
    pub fn keystroke_push_char(&mut self, ch: char) {
        if let Some(tab) = self.tabs.get_mut(&self.active_tab) {
            if let Some(pane) = tab.focused_pane_mut() {
                if !pane.session.state.shell_integration.active {
                    pane.session.state.keystroke_tracker.push_char(ch);
                }
            }
        }
    }

    /// Handle Backspace in the keystroke tracker.
    pub fn keystroke_backspace(&mut self) {
        if let Some(tab) = self.tabs.get_mut(&self.active_tab) {
            if let Some(pane) = tab.focused_pane_mut() {
                if !pane.session.state.shell_integration.active {
                    pane.session.state.keystroke_tracker.backspace();
                }
            }
        }
    }

    /// Handle Ctrl+W in the keystroke tracker.
    pub fn keystroke_delete_word(&mut self) {
        if let Some(tab) = self.tabs.get_mut(&self.active_tab) {
            if let Some(pane) = tab.focused_pane_mut() {
                if !pane.session.state.shell_integration.active {
                    pane.session.state.keystroke_tracker.delete_word();
                }
            }
        }
    }

    /// Handle Ctrl+U / Ctrl+C in the keystroke tracker (clear buffer).
    pub fn keystroke_clear(&mut self) {
        if let Some(tab) = self.tabs.get_mut(&self.active_tab) {
            if let Some(pane) = tab.focused_pane_mut() {
                pane.session.state.keystroke_tracker.clear_line();
            }
        }
    }

    /// Consume the keystroke buffer as a completed command.
    #[allow(dead_code)]
    pub fn keystroke_take(&mut self) -> String {
        if let Some(tab) = self.tabs.get_mut(&self.active_tab) {
            if let Some(pane) = tab.focused_pane_mut() {
                return pane.session.state.keystroke_tracker.take();
            }
        }
        String::new()
    }
    
    // =========================================================================
    // Mouse passthrough support
    // =========================================================================
    
    /// Check if the focused pane has mouse tracking enabled.
    ///
    /// Returns true if the child application has requested mouse events
    /// via DECSET 1000, 1002, or 1003.
    pub fn focused_pane_wants_mouse(&self) -> bool {
        self.tabs.get(&self.active_tab)
            .and_then(|tab| tab.focused_pane())
            .map(|pane| pane.session.state.modes.mouse_enabled())
            .unwrap_or(false)
    }
    
    /// Get mouse encoding mode for focused pane.
    ///
    /// Returns (sgr_mode, urxvt_mode) tuple indicating which extended
    /// mouse encoding the child application has requested.
    pub fn focused_pane_mouse_mode(&self) -> (bool, bool) {
        self.tabs.get(&self.active_tab)
            .and_then(|tab| tab.focused_pane())
            .map(|pane| {
                let modes = &pane.session.state.modes;
                (modes.mouse_sgr_mode, modes.mouse_urxvt_mode)
            })
            .unwrap_or((false, false))
    }

    /// Build the tmux-compatible status snapshot for the active pane.
    pub fn tmux_active_pane_snapshot(&self) -> Option<crate::tmux_compat::PaneSnapshot> {
        let tab = self.active_tab()?;
        let pane = tab.focused_pane()?;
        let pane_index = tab
            .pane_order
            .iter()
            .position(|id| *id == tab.focused_pane)
            .unwrap_or(0);

        Some(crate::tmux_compat::PaneSnapshot {
            pid: std::process::id(),
            session_id: crate::tmux_compat::session_id_for_pid(std::process::id()),
            window_index: self
                .tab_order
                .iter()
                .position(|id| *id == self.active_tab)
                .unwrap_or(0),
            pane_index,
            pane_id: pane.id,
            pane_current_path: pane.session.state.current_path.clone(),
            pane_title: pane.session.state.title.clone(),
            pane_dead: !pane.session.is_running(),
            pane_width: pane.session.state.cols,
            pane_height: pane.session.state.rows,
        })
    }
    
    /// Convert screen coordinates to pane-relative coordinates.
    ///
    /// Takes absolute screen coordinates and returns coordinates relative
    /// to the focused pane's content area, if the point is within the pane.
    ///
    /// # Arguments
    /// * `x` - Screen column (0-based)
    /// * `y` - Screen row relative to content area (excluding tab bar)
    ///
    /// # Returns
    /// Some((pane_x, pane_y)) if coordinates are within the focused pane,
    /// None otherwise.
    pub fn screen_to_pane_coords(&self, x: u16, y: u16) -> Option<(u16, u16)> {
        self.tabs.get(&self.active_tab)
            .and_then(|tab| tab.focused_pane())
            .and_then(|pane| {
                let px = pane.x;
                let py = pane.y;
                let pw = pane.width;
                let ph = pane.height;
                
                // Check if coordinates are within pane content area
                if x >= px && x < px + pw && y >= py && y < py + ph {
                    Some((x - px, y - py))
                } else {
                    None
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_manager(width: u16) -> WindowManager {
        WindowManager::new(width, 24, None, None, PrefixKey { char: 'b' }, true)
    }

    /// The global `[keybindings]` handlers in the wm event loop drive
    /// scrolling through these methods; they must act on the focused pane.
    #[test]
    fn scrollback_methods_target_the_focused_pane() {
        let mut wm = test_manager(80);

        {
            let screen = wm
                .focused_state_mut()
                .expect("focused pane")
                .active_screen_mut();
            for _ in 0..30 {
                screen.scrollback.push_back(crate::core::term::Row::new(80));
            }
        }
        let offset = |wm: &mut WindowManager| {
            wm.focused_state_mut().unwrap().active_screen().scroll_offset
        };

        wm.handle_scroll(10);
        assert_eq!(offset(&mut wm), 10);
        wm.scroll_to_top();
        assert_eq!(offset(&mut wm), 30);
        wm.handle_scroll(-10);
        assert_eq!(offset(&mut wm), 20);
        wm.scroll_to_bottom();
        assert_eq!(offset(&mut wm), 0);
        // Scrolling clamps to the available history.
        wm.handle_scroll(100);
        assert_eq!(offset(&mut wm), 30);
    }

    #[test]
    fn new_tab_button_range_starts_after_rendered_tabs() {
        let wm = test_manager(20);

        assert_eq!(wm.tab_at_position(0), Some(1));
        assert_eq!(wm.tab_at_position(7), Some(1));
        assert_eq!(wm.tab_at_position(8), None);
        assert_eq!(wm.new_tab_button_range(), Some(9..12));
        assert!(wm.is_new_tab_button_at_position(9));
        assert!(wm.is_new_tab_button_at_position(11));
        assert!(!wm.is_new_tab_button_at_position(12));
    }

    #[test]
    fn new_tab_button_is_hidden_when_tab_bar_is_too_narrow() {
        let wm = test_manager(10);

        assert_eq!(wm.new_tab_button_range(), None);
        assert!(!wm.is_new_tab_button_at_position(9));
    }

    #[test]
    fn process_output_reports_tab_removal_when_only_pane_exited() {
        let mut wm = test_manager(80);

        assert!(wm.process_output());
        assert!(wm.tabs.is_empty());
    }

    #[test]
    fn window_info_follows_tab_order_and_marks_active_window() {
        let mut wm = test_manager(80);
        wm.new_tab();

        assert_eq!(
            wm.window_info(),
            vec![
                WindowInfo {
                    id: 1,
                    number: 1,
                    name: "1:main".to_string(),
                    is_active: false,
                    is_last: true,
                    panes: vec![PaneInfo {
                        number: 1,
                        title: "Pane 1".to_string(),
                        is_active: true,
                    }],
                },
                WindowInfo {
                    id: 2,
                    number: 2,
                    name: "2:shell".to_string(),
                    is_active: true,
                    is_last: false,
                    panes: vec![PaneInfo {
                        number: 1,
                        title: "Pane 1".to_string(),
                        is_active: true,
                    }],
                },
            ]
        );
        assert_eq!(wm.active_tab_index(), 1);
    }

    #[test]
    fn pane_level_actions_fall_back_to_window_level_for_single_pane_windows() {
        let mut wm = test_manager(80);
        wm.new_tab();
        assert_eq!(wm.active_tab_index(), 1);

        // Focusing a pane in another window switches to that window
        assert!(wm.focus_pane_at(0, 0));
        assert_eq!(wm.active_tab_index(), 0);
        assert!(!wm.focus_pane_at(0, 5));
        assert!(!wm.focus_pane_at(9, 0));

        // Closing a window's only pane closes the window itself
        assert!(wm.close_pane_at(1, 0));
        assert_eq!(wm.window_info().len(), 1);
        // ...but never the last remaining window
        assert!(!wm.close_pane_at(0, 0));
    }

    #[test]
    fn close_tab_at_removes_window_and_keeps_a_valid_active_tab() {
        let mut wm = test_manager(80);
        wm.new_tab();
        wm.new_tab();
        assert_eq!(wm.active_tab_index(), 2);

        // Closing a non-active window keeps the active window selected
        assert!(wm.close_tab_at(0));
        assert_eq!(wm.window_info().len(), 2);
        assert_eq!(wm.active_tab_index(), 1);

        // Closing the active window moves to the nearest remaining one
        assert!(wm.close_tab_at(1));
        assert_eq!(wm.window_info().len(), 1);
        assert_eq!(wm.active_tab_index(), 0);

        // The last window cannot be closed
        assert!(!wm.close_tab_at(0));
        assert!(!wm.close_tab_at(99));
    }

    #[test]
    fn select_tab_at_switches_window_and_tracks_last_window() {
        let mut wm = test_manager(80);
        wm.new_tab();

        assert!(wm.select_tab_at(0));
        assert_eq!(wm.active_tab_index(), 0);

        wm.last_tab();
        assert_eq!(wm.active_tab_index(), 1);
        assert!(!wm.select_tab_at(99));
    }

    #[test]
    fn select_tab_activates_by_id_and_tracks_last_window() {
        let mut wm = test_manager(80);
        let first = wm.active_tab;
        wm.new_tab();
        let second = wm.active_tab;

        assert!(wm.select_tab(first));
        assert_eq!(wm.active_tab, first);
        assert_eq!(wm.last_active_tab, Some(second));

        assert!(!wm.select_tab(first)); // already active
        assert!(!wm.select_tab(u64::MAX)); // unknown id
    }

    #[test]
    fn rename_focused_pane_sets_and_clears_custom_title() {
        let mut wm = test_manager(80);
        assert_eq!(wm.focused_pane_title(), None);

        wm.rename_focused_pane("build");
        assert_eq!(wm.focused_pane_title().as_deref(), Some("build"));
        let pane_title = wm
            .active_tab()
            .and_then(|tab| tab.panes.get(&tab.focused_pane))
            .map(|pane| pane.display_title());
        assert_eq!(pane_title.as_deref(), Some("build"));

        // Empty name restores the default title
        wm.rename_focused_pane("");
        assert_eq!(wm.focused_pane_title(), None);
    }

    #[test]
    fn pane_title_at_matches_only_top_border_row() {
        let wm = test_manager(80);
        let title_row = wm.tab_bar_height; // first content row = top border

        assert!(wm.pane_title_at(10, title_row).is_some());
        assert_eq!(wm.pane_title_at(10, title_row + 1), None);
        // Tab bar rows are never a pane title
        assert_eq!(wm.pane_title_at(10, 0), None);
    }

    #[test]
    fn toggle_broadcast_flips_per_window_and_shows_in_status() {
        let mut wm = test_manager(80);

        assert!(!wm.broadcast_active());
        assert!(wm.toggle_broadcast());
        assert!(wm.broadcast_active());
        assert!(wm.status_info().contains("[SYNC]"));

        // A new window starts with broadcast off; the first window keeps it
        wm.new_tab();
        assert!(!wm.broadcast_active());
        assert!(!wm.status_info().contains("[SYNC]"));
        wm.select_tab_at(0);
        assert!(wm.broadcast_active());

        assert!(!wm.toggle_broadcast());
        assert!(!wm.broadcast_active());
    }

    #[test]
    fn reported_state_overrides_pane_and_emits_hook_event() {
        let mut wm = test_manager(80);
        let tab_id = wm.active_tab;
        let pane_id = wm.tabs.get(&tab_id).unwrap().focused_pane;

        assert!(wm.drain_agent_state_events().is_empty(), "fresh manager");

        assert!(wm.apply_reported_state(tab_id, pane_id, AgentState::Blocked));
        let events = wm.drain_agent_state_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].from, AgentState::Idle);
        assert_eq!(events[0].to, AgentState::Blocked);
        assert_eq!((events[0].tab_id, events[0].pane_id), (tab_id, pane_id));
        assert!(
            wm.drain_agent_state_events().is_empty(),
            "each transition is drained exactly once"
        );

        // Unknown targets are rejected without panicking
        assert!(!wm.apply_reported_state(tab_id, 999, AgentState::Done));
        assert!(!wm.apply_reported_state(999, pane_id, AgentState::Done));
    }

    #[test]
    fn capture_pane_resolves_targets_and_returns_screen_text() {
        let mut wm = test_manager(80);
        let tab_id = wm.active_tab;
        let pane_id = wm.tabs.get(&tab_id).unwrap().focused_pane;

        // Default target = focused pane of the active window
        assert_eq!(
            wm.resolve_target_pane(None).unwrap(),
            (tab_id, pane_id)
        );
        assert_eq!(
            wm.resolve_target_pane(Some(&format!("{tab_id}.{pane_id}")))
                .unwrap(),
            (tab_id, pane_id)
        );
        assert!(wm.resolve_target_pane(Some("9.9")).is_err());
        assert!(wm.resolve_target_pane(Some("bogus")).is_err());

        let tab = wm.tabs.get_mut(&tab_id).unwrap();
        let pane = tab.panes.get_mut(&pane_id).unwrap();
        pane.session.feed_bytes(b"hello world\r\nsecond line  \r\n");

        let text = wm.capture_pane_text(tab_id, pane_id, false).unwrap();
        assert_eq!(text, "hello world\nsecond line");
    }

    #[test]
    fn tab_info_marks_attention_windows() {
        let mut wm = test_manager(80);
        wm.new_tab(); // window 2 becomes active

        // A bell arrives in the (now background) first window
        let first_id = wm.tab_order[0];
        let tab = wm.tabs.get_mut(&first_id).unwrap();
        let focused = tab.focused_pane;
        tab.panes.get_mut(&focused).unwrap().activity.note_bell(false);

        let tabs = wm.tab_info();
        assert_eq!(tabs[0].1, "1:main!");
        assert!(tabs[0].3, "background window must be flagged");
        assert!(!tabs[1].3);
    }

    #[test]
    fn focus_next_attention_jumps_to_flagged_pane_across_windows() {
        let mut wm = test_manager(80);
        wm.new_tab();
        wm.new_tab(); // three windows, third active

        // Flag a pane in the first (background) window
        let first_id = wm.tab_order[0];
        let tab = wm.tabs.get_mut(&first_id).unwrap();
        let focused = tab.focused_pane;
        tab.panes.get_mut(&focused).unwrap().activity.note_bell(false);

        assert!(wm.focus_next_attention());
        assert_eq!(wm.active_tab_index(), 0);

        // Focusing acknowledges the flag on the next activity tick
        let tab = wm.tabs.get_mut(&first_id).unwrap();
        assert!(tab.update_activity(true, std::time::Duration::from_secs(2)));
        assert!(!wm.focus_next_attention(), "no flagged panes remain");
    }
}
