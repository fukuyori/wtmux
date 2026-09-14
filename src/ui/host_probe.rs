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

#[cfg(windows)]
mod imp {
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
        unsafe {
            let hout = open("CONOUT$")?;
            let hin = match open("CONIN$") {
                Some(h) => h,
                None => {
                    let _ = CloseHandle(hout);
                    return None;
                }
            };
            let result = probe(hin, hout, seq);
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

    unsafe fn probe(hin: HANDLE, hout: HANDLE, seq: &str) -> Option<u16> {
        let mut in_mode = CONSOLE_MODE(0);
        let mut out_mode = CONSOLE_MODE(0);
        GetConsoleMode(hin, &mut in_mode).ok()?;
        GetConsoleMode(hout, &mut out_mode).ok()?;
        let _restore = RestoreModes { hin, in_mode, hout, out_mode };

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
        let start = Instant::now();
        let mut acc = String::new();
        let mut buf = [0u16; 256];
        while start.elapsed() < deadline {
            let remaining = deadline.saturating_sub(start.elapsed()).as_millis().max(1) as u32;
            if WaitForSingleObject(hin, remaining) != WAIT_OBJECT_0 {
                break;
            }
            let mut read = 0u32;
            if ReadConsoleW(hin, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut read, None).is_err() {
                break;
            }
            acc.push_str(&String::from_utf16_lossy(&buf[..read as usize]));
            if let Some(cpr) = parse_cpr(&acc) {
                return Some(cpr);
            }
        }
        None
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
    use std::io::Write;

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
