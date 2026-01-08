//! Pane - A single terminal pane within a tab

use crate::core::session::Session;

/// Unique identifier for a pane
pub type PaneId = u64;

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
    /// Create a new pane
    pub fn new(id: PaneId, cols: u16, rows: u16) -> Self {
        Self {
            id,
            session: Session::new(id, cols, rows),
            x: 0,
            y: 0,
            width: cols,
            height: rows,
            focused: false,
            border: BorderStyle::default(),
            title: None,
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

    /// Resize the pane
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        let (inner_w, inner_h) = self.inner_size();
        let _ = self.session.resize(inner_w, inner_h);
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
