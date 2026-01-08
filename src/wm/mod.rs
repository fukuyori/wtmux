//! Window Manager - Manages tabs and panes (tmux-like functionality)

pub mod pane;
pub mod tab;
pub mod layout;
pub mod manager;

pub use pane::{Pane, BorderStyle};
pub use layout::SplitDirection;
pub use manager::WindowManager;
