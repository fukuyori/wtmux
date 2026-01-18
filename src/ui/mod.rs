//! User interface rendering and input handling.
//!
//! This module provides all UI-related functionality:
//!
//! - **renderer**: Single-pane renderer for simple mode
//! - **wm_renderer**: Multi-pane renderer with tabs, borders, status bar
//! - **keymapper**: Keyboard input to PTY byte sequence mapping
//! - **context_menu**: Right-click context menu for pane operations
//!
//! # Rendering Modes
//!
//! - **Simple mode**: Single pane, minimal UI (uses `Renderer`)
//! - **Multi-pane mode**: Full tmux-like UI (uses `WmRenderer`)

pub mod keymapper;
pub mod renderer;
pub mod wm_renderer;
pub mod context_menu;

pub use keymapper::*;
pub use renderer::*;
pub use wm_renderer::WmRenderer;
pub use context_menu::{ContextMenu, ContextMenuAction};
