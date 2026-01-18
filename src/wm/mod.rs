//! Window Manager - tmux-like tab and pane management.
//!
//! This module provides the core window management functionality:
//!
//! - **manager**: Top-level `WindowManager` coordinating all tabs
//! - **tab**: Individual tabs containing multiple panes
//! - **pane**: Terminal panes with PTY sessions
//! - **layout**: Pane layout algorithms (even, main, tiled)
//!
//! # Module Hierarchy
//!
//! ```text
//! wm/
//! ├── mod.rs      - Module exports
//! ├── manager.rs  - WindowManager (top-level coordinator)
//! ├── tab.rs      - Tab (container for panes)
//! ├── pane.rs     - Pane (terminal + PTY session)
//! └── layout.rs   - Layout algorithms
//! ```

pub mod pane;
pub mod tab;
pub mod layout;
pub mod manager;

pub use pane::{Pane, PaneId, BorderStyle};
pub use layout::SplitDirection;
pub use manager::WindowManager;
