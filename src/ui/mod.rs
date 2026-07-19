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
pub mod input;
pub(crate) mod app_state;
mod cursor;
mod frame;
pub(crate) mod row_stream;
pub mod renderer;
pub mod wm_renderer;
pub mod window_selector;
pub mod agent_dashboard;
pub mod context_menu;

pub use keymapper::*;
pub(crate) use app_state::{UiMode, WmAppState};
pub use renderer::*;
pub use wm_renderer::{WmOverlay, WmRenderer};
pub use window_selector::{TreeEntry, WindowSelector};
pub use agent_dashboard::AgentDashboard;
pub use context_menu::{ContextMenu, ContextMenuAction};
