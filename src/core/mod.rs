//! Core terminal emulation components.
//!
//! This module contains the low-level terminal emulation logic:
//!
//! - **pty**: Windows ConPTY wrapper for pseudo-terminal operations
//! - **term**: VT100/VT220 terminal state and ANSI escape sequence parser
//! - **session**: High-level session combining PTY + terminal state
//!
//! # Architecture
//!
//! ```text
//! Session
//! ├── ConPty (PTY I/O with shell process)
//! └── TerminalState
//!     ├── Screen (cell grid + attributes)
//!     ├── Cursor (position + visibility)
//!     └── Parser (ANSI escape sequences)
//! ```

pub mod pty;
pub mod term;
pub mod session;
