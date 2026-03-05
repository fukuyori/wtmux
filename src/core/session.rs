//! Session management
//!
//! Manages shell sessions, handling I/O between PTY and terminal state.

use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};

use super::pty::{ConPty, PtyError};
use super::term::{Response, TerminalState, VtParser};

/// Session events
#[allow(dead_code)]
#[derive(Debug)]
pub enum SessionEvent {
    /// Output data available (screen updated)
    Output,
    /// Session has exited
    Exited(Option<u32>),
    /// Error occurred
    Error(String),
    /// Title changed
    TitleChanged(String),
}

/// A shell session
pub struct Session {
    /// Session ID
    #[allow(dead_code)]
    pub id: u64,
    /// Terminal state
    pub state: TerminalState,
    /// VT parser
    parser: VtParser,
    /// Optional VT byte trace writer (enabled with --vt-trace)
    vt_trace: Option<BufWriter<std::fs::File>>,
    /// PTY handle (Windows only)
    #[cfg(windows)]
    pty: Option<Arc<ConPty>>,
    /// Running flag
    running: Arc<AtomicBool>,
    /// Reader thread handle
    #[cfg(windows)]
    reader_thread: Option<JoinHandle<()>>,
    /// Channel to receive PTY output
    #[cfg(windows)]
    output_rx: Option<Receiver<Vec<u8>>>,
}

// ConPty needs to be Send + Sync for Arc
#[cfg(windows)]
unsafe impl Sync for ConPty {}

impl Session {
    /// Create a new session
    pub fn new(id: u64, cols: u16, rows: u16) -> Self {
        Self {
            id,
            state: TerminalState::new(cols, rows),
            parser: VtParser::new(),
            vt_trace: None,
            #[cfg(windows)]
            pty: None,
            running: Arc::new(AtomicBool::new(false)),
            #[cfg(windows)]
            reader_thread: None,
            #[cfg(windows)]
            output_rx: None,
        }
    }

    /// Enable VT byte tracing to a file.
    /// Writes every raw byte from the PTY in an annotated hex+ASCII format.
    pub fn enable_vt_trace(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)?;
        self.vt_trace = Some(BufWriter::new(file));
        Ok(())
    }

    /// Start the session with a shell command
    #[cfg(windows)]
    #[allow(dead_code)]
    pub fn start(&mut self, command: Option<&str>) -> Result<(), PtyError> {
        self.start_with_codepage(command, None)
    }

    /// Start the session with a shell command and specific codepage
    #[cfg(windows)]
    pub fn start_with_codepage(&mut self, command: Option<&str>, codepage: Option<u32>) -> Result<(), PtyError> {
        let (cols, rows) = (self.state.cols, self.state.rows);
        let pty = Arc::new(ConPty::new_with_codepage(cols, rows, command, codepage)?);
        self.pty = Some(pty.clone());
        self.running.store(true, Ordering::SeqCst);

        // Create channel for PTY output
        let (tx, rx) = mpsc::channel::<Vec<u8>>();
        self.output_rx = Some(rx);

        // Spawn reader thread
        let running = self.running.clone();
        let reader_thread = thread::spawn(move || {
            let mut buffer = vec![0u8; 4096];

            loop {
                // First check if we should stop
                if !running.load(Ordering::SeqCst) {
                    break;
                }

                // Check if process is still running
                if !pty.is_running() {
                    running.store(false, Ordering::SeqCst);
                    break;
                }

                match pty.read(&mut buffer) {
                    Ok(0) => {
                        // No data available (non-blocking), sleep and retry
                        thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Ok(n) => {
                        // Send data to main thread
                        if tx.send(buffer[..n].to_vec()).is_err() {
                            running.store(false, Ordering::SeqCst);
                            break;
                        }
                    }
                    Err(_) => {
                        // Read error - pipe closed or process exited
                        running.store(false, Ordering::SeqCst);
                        break;
                    }
                }
            }
        });

        self.reader_thread = Some(reader_thread);
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn start(&mut self, _command: Option<&str>) -> Result<(), String> {
        Err("PTY is only supported on Windows".to_string())
    }

    /// Check if session is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Write input to the PTY
    #[cfg(windows)]
    pub fn write(&self, data: &[u8]) -> Result<usize, PtyError> {
        if let Some(pty) = &self.pty {
            pty.write(data)
        } else {
            Err(PtyError::InvalidHandle)
        }
    }

    #[cfg(not(windows))]
    pub fn write(&self, _data: &[u8]) -> Result<usize, String> {
        Err("PTY is only supported on Windows".to_string())
    }

    /// Read and process output from PTY (non-blocking)
    #[cfg(windows)]
    pub fn process_output(&mut self) -> Result<bool, PtyError> {
        // Check if PTY process is still running
        if let Some(pty) = &self.pty {
            if !pty.is_running() {
                self.running.store(false, Ordering::SeqCst);
            }
        }
        
        // First, collect all available data from the channel
        let mut all_data: Vec<Vec<u8>> = Vec::new();
        
        if let Some(rx) = &self.output_rx {
            loop {
                match rx.try_recv() {
                    Ok(data) => {
                        all_data.push(data);
                    }
                    Err(TryRecvError::Empty) => {
                        break;
                    }
                    Err(TryRecvError::Disconnected) => {
                        self.running.store(false, Ordering::SeqCst);
                        break;
                    }
                }
            }
        } else {
            return Ok(false);
        }

        // Now process all collected data
        let processed = !all_data.is_empty();
        for data in all_data {
            self.feed_bytes(&data);
        }

        Ok(processed)
    }

    #[cfg(not(windows))]
    pub fn process_output(&mut self) -> Result<bool, String> {
        Ok(false)
    }

    /// Feed raw bytes into the terminal.
    ///
    /// ConPTY always outputs well-formed UTF-8.  We decode multi-byte sequences
    /// here and route every resulting `char` through the VT parser so that the
    /// parser state machine stays in sync.  Previously, multi-byte UTF-8 chars
    /// bypassed the parser and went straight to `put_char`, which meant they
    /// were written as visible characters even when the parser was inside a
    /// string-body state (DCS, APC, …) that should consume them silently.
    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        // VT trace: write raw bytes in hex + printable-ASCII annotation
        if let Some(ref mut w) = self.vt_trace {
            // Header: byte offset + hex dump
            let _ = write!(w, "─── {} bytes ───
", bytes.len());
            for (i, chunk) in bytes.chunks(16).enumerate() {
                let _ = write!(w, "{:06X}  ", i * 16);
                for b in chunk {
                    let _ = write!(w, "{:02X} ", b);
                }
                // pad
                for _ in chunk.len()..16 {
                    let _ = write!(w, "   ");
                }
                let _ = write!(w, " |");
                for b in chunk {
                    // Show printable ASCII; replace ESC with ↯ for readability
                    if *b == 0x1B {
                        let _ = write!(w, "↯");
                    } else if *b >= 0x20 && *b < 0x7F {
                        let _ = write!(w, "{}", *b as char);
                    } else {
                        let _ = write!(w, "·");
                    }
                }
                let _ = writeln!(w, "|");
            }
            // Also write UTF-8 decoded version showing actual chars
            let _ = write!(w, "  utf8: [");
            let text = String::from_utf8_lossy(bytes);
            for ch in text.chars() {
                if ch == '' {
                    let _ = write!(w, "‹ESC›");
                } else if (ch as u32) < 0x20 {
                    let _ = write!(w, "‹{:02X}›", ch as u32);
                } else {
                    // Show codepoint for non-ASCII
                    if (ch as u32) > 0x7E {
                        let _ = write!(w, "{ch}(U+{:04X})", ch as u32);
                    } else {
                        let _ = write!(w, "{ch}");
                    }
                }
            }
            let _ = writeln!(w, "]");
            let _ = w.flush();
        }

        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];

            // ── Single-byte path ─────────────────────────────────────────
            // C0 controls (0x00–0x1F), DEL (0x7F), and ASCII printable
            // (0x20–0x7E) are fed to the parser one byte at a time.
            if b < 0x80 {
                if let Some(response) = self.parser.feed(b, &mut self.state) {
                    self.send_response(response);
                }
                i += 1;
                continue;
            }

            // ── UTF-8 multi-byte path ────────────────────────────────────
            // Determine sequence length from the leading byte.
            let seq_len: usize = if b & 0xE0 == 0xC0 { 2 }
                else if b & 0xF0 == 0xE0 { 3 }
                else if b & 0xF8 == 0xF0 { 4 }
                else {
                    // Lone continuation byte or invalid — skip.
                    i += 1;
                    continue;
                };

            if i + seq_len > bytes.len() {
                // Truncated sequence at end of buffer — skip leading byte.
                i += 1;
                continue;
            }

            match std::str::from_utf8(&bytes[i..i + seq_len]) {
                Ok(s) => {
                    // Route each decoded character through the parser.
                    // This is critical: if the parser is currently inside a
                    // DCS / APC / SOS / PM string body, the character must be
                    // consumed (ignored) rather than written to the screen.
                    for ch in s.chars() {
                        if let Some(response) = self.parser.feed_char(ch, &mut self.state) {
                            self.send_response(response);
                        }
                    }
                    i += seq_len;
                }
                Err(_) => {
                    // Invalid UTF-8 — skip the leading byte and retry.
                    i += 1;
                }
            }
        }
    }

    /// Send a response back to the PTY
    fn send_response(&self, response: Response) {
        let bytes = response.to_bytes();

        #[cfg(windows)]
        if let Some(pty) = &self.pty {
            let _ = pty.write(&bytes);
        }
    }

    /// Resize the terminal
    #[cfg(windows)]
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), PtyError> {
        // Resize terminal state
        self.state.resize(cols, rows);

        // Resize PTY
        if let Some(pty) = &self.pty {
            pty.resize_pty(cols, rows)?;
        }

        Ok(())
    }

    #[cfg(not(windows))]
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), String> {
        self.state.resize(cols, rows);
        Ok(())
    }

    /// Get the terminal title
    #[allow(dead_code)]
    pub fn title(&self) -> &str {
        &self.state.title
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);

        #[cfg(windows)]
        {
            // Cancel any pending read operations to unblock the reader thread
            if let Some(pty) = &self.pty {
                pty.cancel_read();
            }

            // Wait for reader thread to finish
            if let Some(handle) = self.reader_thread.take() {
                // Give it a moment to exit
                let _ = handle.join();
            }
        }
    }
}

/// Session manager for multiple sessions
#[allow(dead_code)]
pub struct SessionManager {
    sessions: Vec<Session>,
    next_id: u64,
    active_session: Option<usize>,
}

#[allow(dead_code)]
impl Default for SessionManager {
    fn default() -> Self {
        Self::new()
    }
}

#[allow(dead_code)]
impl SessionManager {
    pub fn new() -> Self {
        Self {
            sessions: Vec::new(),
            next_id: 1,
            active_session: None,
        }
    }

    /// Create a new session
    pub fn create_session(&mut self, cols: u16, rows: u16) -> &mut Session {
        let id = self.next_id;
        self.next_id += 1;

        let session = Session::new(id, cols, rows);
        self.sessions.push(session);

        if self.active_session.is_none() {
            self.active_session = Some(self.sessions.len() - 1);
        }

        self.sessions.last_mut().unwrap()
    }

    /// Get the active session
    pub fn active(&self) -> Option<&Session> {
        self.active_session.and_then(|i| self.sessions.get(i))
    }

    /// Get the active session mutably
    pub fn active_mut(&mut self) -> Option<&mut Session> {
        self.active_session.and_then(|i| self.sessions.get_mut(i))
    }

    /// Set active session by index
    pub fn set_active(&mut self, index: usize) {
        if index < self.sessions.len() {
            self.active_session = Some(index);
        }
    }

    /// Get all sessions
    pub fn sessions(&self) -> &[Session] {
        &self.sessions
    }

    /// Remove a session by index
    pub fn remove_session(&mut self, index: usize) {
        if index < self.sessions.len() {
            self.sessions.remove(index);

            // Adjust active session
            if let Some(active) = self.active_session {
                if active >= self.sessions.len() {
                    self.active_session = if self.sessions.is_empty() {
                        None
                    } else {
                        Some(self.sessions.len() - 1)
                    };
                } else if active > index {
                    self.active_session = Some(active - 1);
                }
            }
        }
    }

    /// Get session count
    pub fn count(&self) -> usize {
        self.sessions.len()
    }
}
