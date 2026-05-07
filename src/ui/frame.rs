use std::io::{self, Write};

use crossterm::{
    cursor::{Hide, Show},
    execute,
};

/// Begin a render frame (synchronized update, disable autowrap).
fn begin_frame<W: Write>(out: &mut W) -> io::Result<()> {
    write!(out, "\x1b[?2026h")?;  // Begin synchronized update
    write!(out, "\x1b[?7l")?;      // Disable autowrap
    Ok(())
}

/// End a render frame (enable autowrap, end synchronized update, flush).
fn end_frame<W: Write>(out: &mut W) -> io::Result<()> {
    write!(out, "\x1b[?7h")?;      // Enable autowrap
    write!(out, "\x1b[?2026l")?;   // End synchronized update
    out.flush()?;
    Ok(())
}

/// Execute a render operation with frame guards, ensuring cleanup on error.
pub(crate) fn with_frame<W: Write, F, R>(out: &mut W, f: F) -> io::Result<R>
where
    F: FnOnce(&mut W) -> io::Result<R>,
{
    begin_frame(out)?;
    let result = f(out);
    // Always end frame, even on error.
    let _ = end_frame(out);
    result
}

/// Execute a render operation with the cursor hidden.
pub(crate) fn with_hidden_cursor_frame<W: Write, F, R>(out: &mut W, f: F) -> io::Result<R>
where
    F: FnOnce(&mut W) -> io::Result<R>,
{
    execute!(out, Hide)?;
    let result = with_frame(out, f);
    let _ = execute!(out, Show);
    let _ = out.flush();
    result
}

/// Execute an operation with cursor hidden, ensuring Show on exit.
pub(crate) fn with_cursor_hidden<W: Write, F, R>(out: &mut W, f: F) -> io::Result<R>
where
    F: FnOnce(&mut W) -> io::Result<R>,
{
    execute!(out, Hide)?;
    let result = f(out);
    let _ = execute!(out, Show);
    let _ = out.flush();
    result
}
