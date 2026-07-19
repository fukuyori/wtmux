//! Pane - A single terminal pane within a tab

use std::time::{Duration, Instant};

use crate::core::session::Session;

/// Unique identifier for a pane
pub type PaneId = u64;

/// Why a pane is flagged as needing attention.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Attention {
    /// The program rang the terminal bell (or sent an OSC 9 notification)
    Bell,
    /// The pane produced output while unfocused and then went quiet —
    /// a background agent likely finished or is waiting for input
    Quiet,
}

/// Per-pane activity tracking for the agent-multiplexing monitor.
///
/// States surfaced to the UI:
/// - **busy**: output arrived within the quiet threshold (`*` in the tab bar)
/// - **attention**: bell received while unfocused, or unfocused output went
///   quiet (`!` in the tab bar, highlighted border). Cleared when the pane
///   regains focus.
#[derive(Debug, Default)]
pub struct PaneActivity {
    /// When output last arrived
    last_output: Option<Instant>,
    /// Output arrived while the pane was unfocused (candidate for the
    /// busy→quiet attention transition)
    unfocused_output: bool,
    /// Pending attention flag
    attention: Option<Attention>,
    /// Busy state as last reported to the UI (to detect display changes)
    displayed_busy: bool,
}

impl PaneActivity {
    /// Record that output arrived on this pane.
    pub fn note_output(&mut self, focused: bool) {
        self.last_output = Some(Instant::now());
        if !focused {
            self.unfocused_output = true;
        }
    }

    /// Record a bell. Bells on the focused pane are ignored — the user is
    /// already looking at it.
    pub fn note_bell(&mut self, focused: bool) {
        if !focused {
            self.attention = Some(Attention::Bell);
        }
    }

    /// Advance time-based transitions. Returns `true` when the state shown
    /// in the UI (busy marker or attention flag) changed.
    pub fn tick(&mut self, focused: bool, quiet_threshold: Duration) -> bool {
        let mut changed = false;

        // Focus acknowledges any pending attention.
        if focused && (self.attention.is_some() || self.unfocused_output) {
            self.attention = None;
            self.unfocused_output = false;
            changed = true;
        }

        let busy = self
            .last_output
            .map(|t| t.elapsed() < quiet_threshold)
            .unwrap_or(false);

        // Unfocused output that has gone quiet: the background program
        // stopped talking — flag it.
        if !focused && !busy && self.unfocused_output {
            self.attention.get_or_insert(Attention::Quiet);
            self.unfocused_output = false;
            changed = true;
        }

        if busy != self.displayed_busy {
            self.displayed_busy = busy;
            changed = true;
        }

        changed
    }

    /// Pending attention flag, if any.
    pub fn attention(&self) -> Option<Attention> {
        self.attention
    }

    /// Whether output arrived within the quiet threshold (as of last `tick`).
    pub fn is_busy(&self) -> bool {
        self.displayed_busy
    }
}

/// A single pane containing a terminal session
pub struct Pane {
    /// Unique identifier
    pub id: PaneId,
    /// Terminal session
    pub session: Session,
    /// Position (column, row) in the parent container
    pub x: u16,
    pub y: u16,
    /// Size (width, height)
    pub width: u16,
    pub height: u16,
    /// Whether this pane is focused
    pub focused: bool,
    /// Border style
    pub border: BorderStyle,
    /// Title (optional override)
    pub title: Option<String>,
    /// Activity monitor state (busy / needs-attention)
    pub activity: PaneActivity,
}

/// Border drawing style
#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)]
pub enum BorderStyle {
    None,
    Single,
    Double,
    Rounded,
}

impl Default for BorderStyle {
    fn default() -> Self {
        BorderStyle::Single
    }
}

impl Pane {
    /// Create a new pane with border (default)
    pub fn new(id: PaneId, cols: u16, rows: u16) -> Self {
        // Calculate inner size (accounting for border)
        // Default border is Single, so subtract 2 from each dimension
        let inner_cols = if cols > 2 { cols - 2 } else { 1 };
        let inner_rows = if rows > 2 { rows - 2 } else { 1 };
        
        Self {
            id,
            session: Session::new(id, inner_cols, inner_rows),
            x: 0,
            y: 0,
            width: cols,
            height: rows,
            focused: false,
            border: BorderStyle::default(),
            title: None,
            activity: PaneActivity::default(),
        }
    }

    /// Create a new pane without border (full size)
    pub fn new_without_border(id: PaneId, cols: u16, rows: u16) -> Self {
        Self {
            id,
            session: Session::new(id, cols, rows),
            x: 0,
            y: 0,
            width: cols,
            height: rows,
            focused: false,
            border: BorderStyle::None,
            title: None,
            activity: PaneActivity::default(),
        }
    }

    /// Get the inner dimensions (excluding border)
    pub fn inner_size(&self) -> (u16, u16) {
        match self.border {
            BorderStyle::None => (self.width, self.height),
            _ => {
                let w = if self.width > 2 { self.width - 2 } else { 1 };
                let h = if self.height > 2 { self.height - 2 } else { 1 };
                (w, h)
            }
        }
    }

    /// Get the inner position (excluding border)
    pub fn inner_pos(&self) -> (u16, u16) {
        match self.border {
            BorderStyle::None => (self.x, self.y),
            _ => (self.x + 1, self.y + 1),
        }
    }

    /// Apply geometry (border, position, size) in a consistent order
    /// This is the single entry point for geometry changes
    pub(crate) fn apply_geometry(&mut self, x: u16, y: u16, width: u16, height: u16, border: BorderStyle) {
        // Order is important: border affects inner_size calculation
        self.border = border;
        self.move_to(x, y);
        self.resize(width, height);
    }

    /// Resize the pane
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        let (inner_w, inner_h) = self.inner_size();
        if let Err(e) = self.session.resize(inner_w, inner_h) {
            eprintln!(
                "Pane {} resize failed: outer={}x{}, inner={}x{}: {}",
                self.id, width, height, inner_w, inner_h, e
            );
        }
    }

    /// Move the pane
    pub fn move_to(&mut self, x: u16, y: u16) {
        self.x = x;
        self.y = y;
    }

    /// Get display title
    pub fn display_title(&self) -> String {
        if let Some(ref title) = self.title {
            title.clone()
        } else {
            format!("Pane {}", self.id)
        }
    }

    /// Check if a position is inside this pane
    pub fn contains(&self, col: u16, row: u16) -> bool {
        col >= self.x && col < self.x + self.width &&
        row >= self.y && row < self.y + self.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const QUIET: Duration = Duration::from_millis(0);

    #[test]
    fn unfocused_output_going_quiet_raises_attention() {
        let mut a = PaneActivity::default();
        a.note_output(false);
        // Zero threshold: the pane counts as quiet immediately
        assert!(a.tick(false, QUIET));
        assert_eq!(a.attention(), Some(Attention::Quiet));
    }

    #[test]
    fn focused_output_going_quiet_stays_calm() {
        let mut a = PaneActivity::default();
        a.note_output(true);
        a.tick(true, QUIET);
        assert_eq!(a.attention(), None);
    }

    #[test]
    fn bell_on_unfocused_pane_raises_attention_and_wins_over_quiet() {
        let mut a = PaneActivity::default();
        a.note_output(false);
        a.note_bell(false);
        a.tick(false, QUIET);
        assert_eq!(a.attention(), Some(Attention::Bell));
    }

    #[test]
    fn bell_on_focused_pane_is_ignored() {
        let mut a = PaneActivity::default();
        a.note_bell(true);
        a.tick(true, QUIET);
        assert_eq!(a.attention(), None);
    }

    #[test]
    fn regaining_focus_clears_attention() {
        let mut a = PaneActivity::default();
        a.note_output(false);
        a.tick(false, QUIET);
        assert_eq!(a.attention(), Some(Attention::Quiet));

        assert!(a.tick(true, QUIET));
        assert_eq!(a.attention(), None);
        // ...and it does not come back on the next tick
        a.tick(false, QUIET);
        assert_eq!(a.attention(), None);
    }

    #[test]
    fn busy_state_follows_recent_output() {
        let mut a = PaneActivity::default();
        assert!(!a.is_busy());
        a.note_output(true);
        assert!(a.tick(true, Duration::from_secs(60)));
        assert!(a.is_busy());
    }
}
