//! Platform pseudo-terminal backends.
//!
//! Windows uses ConPTY (`conpty`); macOS and Linux use a POSIX pty via
//! openpty(3) (`unix`). Both backends expose the same API surface, re-exported
//! here under the platform-neutral name `Pty`, so `Session` drives either one
//! unchanged: non-blocking `read`, `write`, `resize_pty`, `is_running`,
//! `exit_code`, and `cancel_read`.

#[cfg(windows)]
mod conpty;
#[cfg(windows)]
mod conpty_api;
#[cfg(windows)]
#[allow(unused_imports)]
pub use conpty::{ConPty, ConPty as Pty, PtyError, Result};
#[cfg(windows)]
#[allow(unused_imports)]
pub use conpty_api::{api as conpty_api, Backend as ConPtyBackend};

#[cfg(unix)]
mod unix;
#[cfg(unix)]
#[allow(unused_imports)]
pub use unix::{PtyError, Result, UnixPty as Pty};
