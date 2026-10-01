//! Startup measurement of the host terminal's cell layout.
//!
//! wtmux's grid must agree with the terminal that finally draws it. Most
//! width questions have one answer across the terminals wtmux was measured
//! against, but VS16 emoji presentation (❤ + U+FE0F) does not: Windows
//! Terminal 1.24 draws ❤️ in two cells, WezTerm 1.22 in one. Rather than
//! guess, wtmux writes the sequence once at startup, asks the terminal where
//! the cursor ended up (DSR / CPR), and adopts that answer for the session.
//!
//! The probe runs right after the alternate screen is entered and clears its
//! own line, so nothing of it stays visible.

/// Number of cells the host terminal advances for "❤\u{FE0F}", or `None` when
/// the terminal does not answer within the deadline (not a tty, headless).
pub(crate) fn measure_vs16_emoji_cells() -> Option<u16> {
    const SEQ: &str = "\u{2764}\u{FE0F}";
    imp::measure(SEQ)
}

/// Ask the host for its default foreground, background and cursor colours
/// (OSC 10 / 11 / 12) and remember them for answering the same queries from
/// panes (`core::term::host_colors`). Meant to run right after
/// `measure_vs16_emoji_cells` succeeded, i.e. only on a host that answers a
/// CPR: the CPR sent after the queries then ends the wait at once even if the
/// host ignores OSC 10/11/12. Does nothing on hosts that cannot be asked.
pub(crate) fn learn_host_colors() {
    use crate::core::term::host_colors::{set_host_colors, HostColors};
    if let Some(colors) = imp::measure_colors() {
        if colors != HostColors::default() {
            set_host_colors(colors);
        }
    }
}

#[cfg(windows)]
mod imp {
    use crate::core::term::host_colors::{parse_host_replies, HostColors};

    use std::os::windows::ffi::OsStrExt;
    use std::time::{Duration, Instant};

    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        OPEN_EXISTING,
    };
    use windows::Win32::System::Console::{
        FlushConsoleInputBuffer, GetConsoleMode, ReadConsoleW, SetConsoleMode, WriteConsoleW,
        CONSOLE_MODE, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT, ENABLE_PROCESSED_OUTPUT,
        ENABLE_VIRTUAL_TERMINAL_INPUT, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
    };
    use windows::Win32::System::Threading::WaitForSingleObject;

    /// Talk to CONIN$/CONOUT$ directly: the CPR must be read as text, which
    /// needs ENABLE_VIRTUAL_TERMINAL_INPUT, and the modes are restored before
    /// the regular input reader (INPUT_RECORD based) starts.
    pub(super) fn measure(seq: &str) -> Option<u16> {
        with_console(|hin, hout| unsafe { probe(hin, hout, seq) })
    }

    /// The host's reply to OSC 10 / 11 / 12 queries, or `None` when it did
    /// not even answer the CPR sent after them.
    pub(super) fn measure_colors() -> Option<HostColors> {
        with_console(|hin, hout| unsafe { probe_colors(hin, hout) })
    }

    fn with_console<T>(f: impl FnOnce(HANDLE, HANDLE) -> Option<T>) -> Option<T> {
        unsafe {
            let hout = open("CONOUT$")?;
            let hin = match open("CONIN$") {
                Some(h) => h,
                None => {
                    let _ = CloseHandle(hout);
                    return None;
                }
            };
            let result = f(hin, hout);
            let _ = CloseHandle(hin);
            let _ = CloseHandle(hout);
            result
        }
    }

    unsafe fn open(name: &str) -> Option<HANDLE> {
        let wide: Vec<u16> = std::ffi::OsStr::new(name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            Default::default(),
            None,
        )
        .ok()
    }

    /// Put both handles in VT mode with line editing off, flush stale input,
    /// and restore the modes when the guard drops.
    unsafe fn raw_modes(hin: HANDLE, hout: HANDLE) -> Option<RestoreModes> {
        let mut in_mode = CONSOLE_MODE(0);
        let mut out_mode = CONSOLE_MODE(0);
        GetConsoleMode(hin, &mut in_mode).ok()?;
        GetConsoleMode(hout, &mut out_mode).ok()?;
        let restore = RestoreModes { hin, in_mode, hout, out_mode };

        SetConsoleMode(
            hout,
            CONSOLE_MODE(out_mode.0 | ENABLE_PROCESSED_OUTPUT.0 | ENABLE_VIRTUAL_TERMINAL_PROCESSING.0),
        )
        .ok()?;
        SetConsoleMode(
            hin,
            CONSOLE_MODE((in_mode.0 | ENABLE_VIRTUAL_TERMINAL_INPUT.0) & !(ENABLE_LINE_INPUT.0 | ENABLE_ECHO_INPUT.0)),
        )
        .ok()?;
        let _ = FlushConsoleInputBuffer(hin);
        Some(restore)
    }

    /// Ask for the three colours, then for the cursor: hosts answer in query
    /// order (measured on WezTerm, Windows Terminal and Ghostty), so the CPR
    /// arrives after every colour the host will give and ends the read.
    /// Nothing is read after the CPR: the handle can be signalled by events
    /// that carry no text, and `ReadConsoleW` would then block.
    unsafe fn probe_colors(hin: HANDLE, hout: HANDLE) -> Option<HostColors> {
        let _restore = raw_modes(hin, hout)?;
        write(hout, "\x1b]10;?\x1b\\\x1b]11;?\x1b\\\x1b]12;?\x1b\\\x1b[6n");
        let (acc, _cpr) = read_until_cpr(hin, Duration::from_millis(500))?;
        Some(parse_host_replies(&acc))
    }

    unsafe fn probe(hin: HANDLE, hout: HANDLE, seq: &str) -> Option<u16> {
        let _restore = raw_modes(hin, hout)?;

        // Home, clear the line, write the sequence, ask for the cursor.
        write(hout, &format!("\x1b[H\x1b[2K{}\x1b[6n", seq));
        let reply = read_cpr(hin, Duration::from_millis(500));
        write(hout, "\x1b[H\x1b[2K");
        reply.map(|(_, col)| col.saturating_sub(1))
    }

    unsafe fn write(hout: HANDLE, s: &str) {
        let wide: Vec<u16> = s.encode_utf16().collect();
        let mut written = 0u32;
        let _ = WriteConsoleW(hout, &wide, Some(&mut written), None);
    }

    /// Read until a `ESC [ row ; col R` arrives or the deadline passes.
    unsafe fn read_cpr(hin: HANDLE, deadline: Duration) -> Option<(u16, u16)> {
        read_until_cpr(hin, deadline).map(|(_, cpr)| cpr)
    }

    /// Like `read_cpr`, but also returns everything read so far.
    unsafe fn read_until_cpr(hin: HANDLE, deadline: Duration) -> Option<(String, (u16, u16))> {
        let start = Instant::now();
        let mut acc = String::new();
        while start.elapsed() < deadline {
            let remaining = deadline.saturating_sub(start.elapsed());
            if !read_chunk(hin, &mut acc, remaining) {
                break;
            }
            if let Some(cpr) = parse_cpr(&acc) {
                return Some((acc, cpr));
            }
        }
        None
    }

    /// Wait up to `wait` for input and append it to `acc`; false when nothing
    /// came or reading failed.
    unsafe fn read_chunk(hin: HANDLE, acc: &mut String, wait: Duration) -> bool {
        let mut buf = [0u16; 256];
        let ms = wait.as_millis().max(1) as u32;
        if WaitForSingleObject(hin, ms) != WAIT_OBJECT_0 {
            return false;
        }
        let mut read = 0u32;
        if ReadConsoleW(hin, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut read, None).is_err() {
            return false;
        }
        acc.push_str(&String::from_utf16_lossy(&buf[..read as usize]));
        true
    }

    struct RestoreModes {
        hin: HANDLE,
        in_mode: CONSOLE_MODE,
        hout: HANDLE,
        out_mode: CONSOLE_MODE,
    }

    impl Drop for RestoreModes {
        fn drop(&mut self) {
            unsafe {
                let _ = SetConsoleMode(self.hin, self.in_mode);
                let _ = SetConsoleMode(self.hout, self.out_mode);
            }
        }
    }

    pub(super) fn parse_cpr(s: &str) -> Option<(u16, u16)> {
        super::parse_cpr(s)
    }
}

#[cfg(unix)]
mod imp {
    use crate::core::term::host_colors::HostColors;
    use std::io::Write;

    /// Not implemented: crossterm's reader would take the OSC replies for
    /// key presses, so reading them needs its own raw stdin read.
    pub(super) fn measure_colors() -> Option<HostColors> {
        None
    }

    /// crossterm's `cursor::position()` issues DSR on the tty and parses the
    /// CPR (with its own timeout); raw mode is already on when this runs.
    pub(super) fn measure(seq: &str) -> Option<u16> {
        let mut stdout = std::io::stdout();
        write!(stdout, "\x1b[H\x1b[2K{}", seq).ok()?;
        stdout.flush().ok()?;
        let pos = crossterm::cursor::position().ok();
        let _ = write!(stdout, "\x1b[H\x1b[2K");
        let _ = stdout.flush();
        pos.map(|(col, _row)| col)
    }
}

/// Extract the last `ESC [ row ; col R` from `s`.
fn parse_cpr(s: &str) -> Option<(u16, u16)> {
    let start = s.rfind("\x1b[")?;
    let body = &s[start + 2..];
    let end = body.find('R')?;
    let mut parts = body[..end].split(';');
    let row = parts.next()?.parse().ok()?;
    let col = parts.next()?.parse().ok()?;
    Some((row, col))
}

#[cfg(test)]
mod tests {
    use super::parse_cpr;

    #[test]
    fn parses_cursor_position_report() {
        assert_eq!(parse_cpr("\x1b[1;3R"), Some((1, 3)));
        assert_eq!(parse_cpr("junk\x1b[12;40R"), Some((12, 40)));
        assert_eq!(parse_cpr("\x1b[1;2"), None);
        assert_eq!(parse_cpr(""), None);
    }
}
