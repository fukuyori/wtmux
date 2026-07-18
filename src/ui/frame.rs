use std::io::{self, Write};

use crossterm::{
    cursor::{Hide, Show},
    queue,
};

/// Writer adapter that swallows flush requests.
///
/// crossterm's `execute!` flushes after queueing, so inside a render frame it
/// would split the frame into many small console writes and let the host
/// terminal present intermediate states — most visibly the cursor hopping
/// through whichever pane is being repainted. Wrapping the frame writer in
/// this adapter turns every mid-frame `execute!` into a plain `queue!`; the
/// frame is flushed once, at `end_frame`.
pub(crate) struct NonFlushing<'a, W: Write>(&'a mut W);

impl<W: Write> Write for NonFlushing<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

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
pub(crate) fn with_frame<'a, W: Write, F, R>(out: &'a mut W, f: F) -> io::Result<R>
where
    F: FnOnce(&mut NonFlushing<'a, W>) -> io::Result<R>,
{
    let mut nf = NonFlushing(out);
    begin_frame(&mut nf)?;
    let result = f(&mut nf);
    // Always end frame, even on error.
    let _ = end_frame(nf.0);
    result
}

/// Execute an operation with cursor hidden, ensuring Show on exit. Everything
/// is queued and flushed once so the hidden interval stays in a single write.
pub(crate) fn with_cursor_hidden<'a, W: Write, F, R>(out: &'a mut W, f: F) -> io::Result<R>
where
    F: FnOnce(&mut NonFlushing<'a, W>) -> io::Result<R>,
{
    let mut nf = NonFlushing(out);
    let _ = queue!(nf, Hide);
    let result = f(&mut nf);
    let _ = queue!(nf, Show);
    let _ = nf.0.flush();
    result
}
