//! Session management
//!
//! Manages shell sessions, handling I/O between PTY and terminal state.

use std::io::{BufWriter, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
#[cfg(windows)]
use std::time::{Duration, Instant};

use super::pty::{Pty, PtyError};
use super::term::resize::DEFAULT_SESSION_RESIZE_POLICY;
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

/// After a resize the shell has been quiet for this long, the ConPTY
/// buffer replay is considered finished and rendering resumes.
#[cfg(windows)]
const RESIZE_SETTLE_QUIET: Duration = Duration::from_millis(60);

/// Hard cap on render suppression after a resize, so a shell that streams
/// output continuously (e.g. `ping -t`) cannot keep the pane frozen.
#[cfg(windows)]
const RESIZE_SETTLE_MAX: Duration = Duration::from_millis(400);

/// Tracks the ConPTY buffer replay that follows `ResizePseudoConsole`.
///
/// conhost re-emits the pane's entire text buffer after a resize. Rendering
/// each arriving chunk makes the old content visibly scroll past from the
/// top of the buffer. While this state is present, output is still parsed
/// into the grid but renders are suppressed; once the replay goes quiet the
/// final state is painted in one frame.
#[cfg(windows)]
struct ResizeSettle {
    /// When the resize was issued (for the hard cap)
    started: Instant,
    /// When the last output chunk arrived (for quiescence detection)
    last_output: Instant,
    /// Whether any output arrived during the settle window
    pending: bool,
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
    /// Optional raw output log (tmux pipe-pane analog), toggled at runtime
    pipe_log: Option<(std::path::PathBuf, BufWriter<std::fs::File>)>,
    /// Tail of an incomplete UTF-8 sequence left over from the previous
    /// `feed_bytes` call. PTY reads arrive in arbitrary-sized chunks, so a
    /// multi-byte character can be split across two reads; its leading bytes
    /// are held here until the continuation bytes arrive. At most 3 bytes.
    utf8_pending: Vec<u8>,
    /// PTY handle
    pty: Option<Arc<Pty>>,
    /// Running flag
    running: Arc<AtomicBool>,
    /// Reader thread handle
    reader_thread: Option<JoinHandle<()>>,
    /// Channel to receive PTY output
    output_rx: Option<Receiver<Vec<u8>>>,
    /// While true (during a split-border drag), `resize()` updates only the
    /// local state and records the size in `pending_pty_resize`; the PTY is
    /// resized once when the deferral ends, so the shell sees a single
    /// SIGWINCH instead of a redraw storm racing the local reflow.
    defer_pty_resize: bool,
    /// Last size deferred while `defer_pty_resize` was set.
    pending_pty_resize: Option<(u16, u16)>,
    /// Last size actually sent to the PTY.
    pty_size: (u16, u16),
    /// In-progress ConPTY buffer replay after a resize (renders suppressed)
    #[cfg(windows)]
    resize_settle: Option<ResizeSettle>,
}

// ConPty needs to be Send + Sync for Arc (the Unix pty is naturally both)
#[cfg(windows)]
unsafe impl Sync for Pty {}

impl Session {
    /// Create a new session
    pub fn new(id: u64, cols: u16, rows: u16) -> Self {
        Self {
            id,
            state: TerminalState::new(cols, rows),
            parser: VtParser::new(),
            vt_trace: None,
            pipe_log: None,
            utf8_pending: Vec::new(),
            pty: None,
            running: Arc::new(AtomicBool::new(false)),
            reader_thread: None,
            output_rx: None,
            defer_pty_resize: false,
            pending_pty_resize: None,
            pty_size: (cols, rows),
            #[cfg(windows)]
            resize_settle: None,
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

    /// Start logging raw PTY output to a file (tmux pipe-pane analog).
    /// The log receives the exact byte stream, including escape sequences.
    pub fn start_pipe_log(&mut self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;
        self.pipe_log = Some((path.to_path_buf(), BufWriter::new(file)));
        Ok(())
    }

    /// Stop the pipe log, returning the path it was writing to.
    pub fn stop_pipe_log(&mut self) -> Option<std::path::PathBuf> {
        self.pipe_log.take().map(|(path, mut writer)| {
            let _ = writer.flush();
            path
        })
    }

    /// Whether a pipe log is currently attached.
    pub fn pipe_log_active(&self) -> bool {
        self.pipe_log.is_some()
    }

    /// Start the session with a shell command
    #[allow(dead_code)]
    pub fn start(&mut self, command: Option<&str>) -> Result<(), PtyError> {
        self.start_with_codepage(command, None)
    }

    /// Start the session with a shell command and specific codepage
    #[allow(dead_code)]
    pub fn start_with_codepage(&mut self, command: Option<&str>, codepage: Option<u32>) -> Result<(), PtyError> {
        self.start_with_options(command, codepage, false)
    }

    /// Start the session with a shell command, codepage, and shell options.
    pub fn start_with_options(
        &mut self,
        command: Option<&str>,
        codepage: Option<u32>,
        cwd_prompt_hook: bool,
    ) -> Result<(), PtyError> {
        let (cols, rows) = (self.state.cols, self.state.rows);
        let pty = Arc::new(Pty::new_with_options(
            cols,
            rows,
            command,
            codepage,
            cwd_prompt_hook,
        )?);
        self.pty = Some(pty.clone());
        self.pty_size = (cols, rows);
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

    /// Check if session is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }

    /// Write input to the PTY
    pub fn write(&self, data: &[u8]) -> Result<usize, PtyError> {
        if let Some(pty) = &self.pty {
            pty.write(data)
        } else {
            Err(PtyError::InvalidHandle)
        }
    }

    /// Read and process output from PTY (non-blocking)
    pub fn process_output(&mut self) -> Result<bool, PtyError> {
        // Check if PTY process is still running
        if let Some(pty) = &self.pty {
            if !pty.is_running() {
                self.running.store(false, Ordering::SeqCst);
            }
        }

        if self.output_rx.is_none() {
            #[cfg(windows)]
            {
                self.resize_settle = None;
            }
            return Ok(false);
        }

        let mut processed = false;
        loop {
            let recv_result = match self.output_rx.as_ref() {
                Some(rx) => rx.try_recv(),
                None => return Ok(processed),
            };

            match recv_result {
                Ok(data) => {
                    processed = true;
                    self.feed_bytes(&data);
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

        // While the post-resize buffer replay is in flight, keep parsing but
        // hold back the "render needed" signal until the stream goes quiet
        // (or the hard cap expires), then release one render with the final
        // state. Dirty-line tracking accumulates across the window, so that
        // single render repaints everything the replay touched.
        #[cfg(windows)]
        if let Some(settle) = &mut self.resize_settle {
            let now = Instant::now();
            if processed {
                settle.last_output = now;
                settle.pending = true;
            }
            let quiet = now.duration_since(settle.last_output) >= RESIZE_SETTLE_QUIET;
            let expired = now.duration_since(settle.started) >= RESIZE_SETTLE_MAX;
            if quiet || expired {
                let render = settle.pending;
                self.resize_settle = None;
                return Ok(render || processed);
            }
            return Ok(false);
        }

        Ok(processed)
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
        // Pipe log: raw byte stream, flushed per chunk so `tail -f` works
        if let Some((_, ref mut w)) = self.pipe_log {
            let _ = w.write_all(bytes);
            let _ = w.flush();
        }

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

        // A previous chunk may have ended mid UTF-8 sequence (PTY reads are
        // chunked at arbitrary byte boundaries); prepend the held-over lead
        // bytes so the character is decoded whole instead of being dropped.
        if self.utf8_pending.is_empty() {
            self.decode_and_feed(bytes);
        } else {
            let mut joined = std::mem::take(&mut self.utf8_pending);
            joined.extend_from_slice(bytes);
            self.decode_and_feed(&joined);
        }
    }

    fn decode_and_feed(&mut self, bytes: &[u8]) {
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
                // Truncated sequence at the end of this chunk — hold the
                // lead bytes until the continuation arrives in the next read.
                self.utf8_pending.extend_from_slice(&bytes[i..]);
                break;
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

        if let Some(pty) = &self.pty {
            let _ = pty.write(&bytes);
        }
    }

    /// Resize the terminal
    pub fn resize(&mut self, cols: u16, rows: u16) -> Result<(), PtyError> {
        // Layout recomputes call resize on every pane; skip the local reflow
        // when this pane's size didn't actually change.
        let unchanged = cols == self.state.cols && rows == self.state.rows;

        if self.defer_pty_resize {
            // A split-border drag is in progress: update local state only.
            // The PTY gets one resize (and the shell one SIGWINCH) when the
            // drag ends, so its prompt redraw runs against the final size
            // instead of racing the per-step local reflows.
            if !unchanged {
                self.pending_pty_resize = Some((cols, rows));
                self.state
                    .resize_with_policy(cols, rows, DEFAULT_SESSION_RESIZE_POLICY);
            }
            return Ok(());
        }

        // Let the PTY / the host terminal recalculate wrapping first, then
        // update our local state using the selected resize policy.
        self.resize_pty_now(cols, rows)?;
        if !unchanged {
            self.state
                .resize_with_policy(cols, rows, DEFAULT_SESSION_RESIZE_POLICY);
        }
        Ok(())
    }

    /// Send a size to the PTY, skipping sizes it already has.
    fn resize_pty_now(&mut self, cols: u16, rows: u16) -> Result<(), PtyError> {
        if (cols, rows) == self.pty_size {
            return Ok(());
        }
        if let Some(pty) = &self.pty {
            pty.resize_pty(cols, rows)?;
            // conhost replays the whole buffer after a real size change;
            // suppress renders until it settles so only the final state is
            // painted. Unix ptys don't replay, so no settle window is
            // needed there.
            #[cfg(windows)]
            {
                let now = Instant::now();
                self.resize_settle = Some(ResizeSettle {
                    started: now,
                    last_output: now,
                    pending: false,
                });
            }
        }
        self.pty_size = (cols, rows);
        Ok(())
    }

    /// Enable or disable PTY-resize deferral (set while a split-border drag
    /// is in progress). Disabling flushes the last deferred size to the PTY.
    pub fn set_pty_resize_deferred(&mut self, defer: bool) {
        self.defer_pty_resize = defer;
        if !defer {
            if let Some((cols, rows)) = self.pending_pty_resize.take() {
                let _ = self.resize_pty_now(cols, rows);
            }
        }
    }

    /// Whether the post-resize ConPTY buffer replay is still in flight.
    /// The renderer skips incremental paints of this session while true.
    pub fn is_settling(&self) -> bool {
        #[cfg(windows)]
        {
            self.resize_settle.is_some()
        }
        #[cfg(not(windows))]
        {
            false
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn screen_text(session: &Session, row: usize) -> String {
        session.state.active_screen().rows[row]
            .cells
            .iter()
            .map(|c| c.grapheme.as_str())
            .collect()
    }

    /// While a resize-settle window is open, arriving output is parsed but
    /// process_output must not signal a render; once the stream has been
    /// quiet past the threshold, the deferred render is released exactly once.
    #[cfg(windows)]
    #[test]
    fn resize_settle_suppresses_renders_until_replay_goes_quiet() {
        let mut session = Session::new(1, 80, 24);
        let (tx, rx) = mpsc::channel();
        session.output_rx = Some(rx);

        let now = Instant::now();
        session.resize_settle = Some(ResizeSettle {
            started: now,
            last_output: now,
            pending: false,
        });

        tx.send(b"replayed buffer chunk".to_vec()).unwrap();
        assert!(
            !session.process_output().unwrap(),
            "render must be suppressed while the replay is in flight"
        );
        assert!(session.is_settling());
        assert_eq!(
            screen_text(&session, 0).trim_end(),
            "replayed buffer chunk",
            "output must still be parsed into the grid during the settle window"
        );

        // Pretend the stream has been quiet past the threshold.
        if let Some(settle) = &mut session.resize_settle {
            settle.last_output = now - RESIZE_SETTLE_QUIET;
        }
        assert!(
            session.process_output().unwrap(),
            "the deferred render must be released once the replay goes quiet"
        );
        assert!(!session.is_settling());
    }

    /// A settle window with no replay output at all must close quietly
    /// without forcing a render.
    #[cfg(windows)]
    #[test]
    fn resize_settle_with_no_output_closes_without_render() {
        let mut session = Session::new(1, 80, 24);
        let (_tx, rx) = mpsc::channel();
        session.output_rx = Some(rx);

        let past = Instant::now() - Duration::from_millis(100);
        session.resize_settle = Some(ResizeSettle {
            started: past,
            last_output: past,
            pending: false,
        });

        assert!(!session.process_output().unwrap());
        assert!(!session.is_settling());
    }

    #[test]
    fn pipe_log_captures_raw_output_bytes_only_while_active() {
        let path = std::env::temp_dir()
            .join("wtmux_pipe_log_test")
            .join(format!("pane-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&path);

        let mut session = Session::new(0, 80, 24);
        session.feed_bytes(b"before logging\r\n");

        session.start_pipe_log(&path).unwrap();
        assert!(session.pipe_log_active());
        session.feed_bytes(b"hello \x1b[31mred\x1b[0m\r\n");

        let logged = std::fs::read(&path).unwrap();
        assert_eq!(
            logged, b"hello \x1b[31mred\x1b[0m\r\n",
            "log holds exactly the bytes fed while active, escapes included"
        );

        assert_eq!(session.stop_pipe_log().as_deref(), Some(path.as_path()));
        assert!(!session.pipe_log_active());
        session.feed_bytes(b"after logging\r\n");
        assert_eq!(
            std::fs::read(&path).unwrap(),
            logged,
            "nothing is appended after the log is stopped"
        );
    }

    /// During a split-border drag, resize() must touch only the local state;
    /// the PTY receives a single resize with the final size when the
    /// deferral ends.
    #[test]
    fn deferred_resize_updates_local_state_only_and_flushes_on_release() {
        let mut session = Session::new(0, 80, 24);
        session.set_pty_resize_deferred(true);

        session.resize(60, 24).unwrap();
        session.resize(50, 20).unwrap();
        assert_eq!((session.state.cols, session.state.rows), (50, 20));
        assert_eq!(
            session.pty_size,
            (80, 24),
            "PTY size must stay untouched mid-drag"
        );
        assert_eq!(session.pending_pty_resize, Some((50, 20)));

        session.set_pty_resize_deferred(false);
        assert_eq!(
            session.pty_size,
            (50, 20),
            "only the final size is flushed to the PTY on drag end"
        );
        assert_eq!(session.pending_pty_resize, None);
    }

    /// Layout recomputes resize every pane; a call with the unchanged size
    /// must be a no-op (no pending PTY resize recorded).
    #[test]
    fn unchanged_resize_is_a_noop() {
        let mut session = Session::new(0, 80, 24);
        session.set_pty_resize_deferred(true);
        session.resize(80, 24).unwrap();
        assert_eq!(session.pending_pty_resize, None);

        session.set_pty_resize_deferred(false);
        assert_eq!(session.pty_size, (80, 24));
    }

    #[test]
    fn utf8_char_split_across_read_chunks_is_not_dropped() {
        // PTY reads arrive in arbitrary-sized chunks (4 KiB reader buffer), so
        // a 3-byte kana can be split anywhere. Every split position of the
        // stream must decode to the same screen content.
        let text = "じゅげむ ごこうのすりきれ";
        let bytes = text.as_bytes();

        for split in 1..bytes.len() {
            let mut session = Session::new(0, 80, 4);
            session.feed_bytes(&bytes[..split]);
            session.feed_bytes(&bytes[split..]);

            let rendered = screen_text(&session, 0);
            assert_eq!(
                rendered.trim_end(),
                text,
                "characters dropped when the stream is split at byte {split}"
            );
        }
    }

    #[test]
    fn utf8_char_split_into_single_byte_chunks_is_not_dropped() {
        // Worst case: every byte arrives in its own read.
        let text = "パイポパイポ";
        let mut session = Session::new(0, 80, 4);
        for b in text.as_bytes() {
            session.feed_bytes(std::slice::from_ref(b));
        }
        assert_eq!(screen_text(&session, 0).trim_end(), text);
    }

    #[cfg(windows)]
    fn repro_wrap_text(text: &str, width: usize) -> Vec<String> {
        use unicode_width::UnicodeWidthChar;
        let mut lines = Vec::new();
        let mut cur = String::new();
        let mut w = 0usize;
        for ch in text.chars() {
            let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
            if w + cw > width {
                lines.push(std::mem::take(&mut cur));
                w = 0;
                if ch == ' ' {
                    continue;
                }
            }
            cur.push(ch);
            w += cw;
        }
        if !cur.is_empty() {
            lines.push(cur);
        }
        lines
    }

    /// Ink-style frame: erase the previous block (CUU+EL per line) and
    /// rewrite the re-wrapped block, one line per row, 2-col indent.
    #[cfg(windows)]
    fn repro_ink_frame(lines: &[String], prev_lines: usize, styled: bool) -> Vec<u8> {
        let mut f = Vec::new();
        for _ in 0..prev_lines {
            f.extend_from_slice(b"\x1b[1A\x1b[2K");
        }
        f.extend_from_slice(b"\r");
        for l in lines {
            if styled {
                f.extend_from_slice(b"\x1b[38;2;190;190;190m");
            }
            f.extend_from_slice(b"  ");
            f.extend_from_slice(l.as_bytes());
            if styled {
                f.extend_from_slice(b"\x1b[39m");
            }
            f.extend_from_slice(b"\r\n");
        }
        f
    }

    /// Run a ConPTY child that types the given frame files, capture ConPTY's
    /// re-encoded output stream (with real read-chunk boundaries).
    #[cfg(windows)]
    fn repro_run_conpty(
        dir: &std::path::Path,
        tag: &str,
        frames: &[Vec<u8>],
        delay_between_frames: bool,
        cols: u16,
        rows: u16,
    ) -> Vec<Vec<u8>> {
        let mut script = String::from("@echo off\r\nchcp 65001 >nul\r\n");
        for (i, frame) in frames.iter().enumerate() {
            let p = dir.join(format!("{}_{:03}.vt", tag, i));
            std::fs::write(&p, frame).expect("write frame");
            // `>CON` forces the output to the process's attached console (the
            // ConPTY): under `cargo test` the child inherits the test
            // harness's piped stdout, which would bypass the ConPTY entirely.
            script.push_str(&format!("type \"{}\" >CON\r\n", p.display()));
            if delay_between_frames {
                script.push_str("ping -n 2 127.0.0.1 >nul\r\n");
            }
        }
        let script_path = dir.join(format!("{}.cmd", tag));
        std::fs::write(&script_path, script.as_bytes()).expect("write script");

        let pty = crate::core::pty::ConPty::new(
            cols,
            rows,
            Some(&format!("cmd.exe /c \"{}\"", script_path.display())),
        )
        .expect("spawn conpty");

        let mut chunks: Vec<Vec<u8>> = Vec::new();
        let mut buf = [0u8; 4096];
        let start = std::time::Instant::now();
        let mut quiet_after_exit = 0;
        loop {
            match pty.read(&mut buf) {
                Ok(0) => {
                    if !pty.is_running() {
                        quiet_after_exit += 1;
                        if quiet_after_exit > 30 {
                            break;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Ok(n) => {
                    quiet_after_exit = 0;
                    chunks.push(buf[..n].to_vec());
                }
                Err(_) => break,
            }
            if start.elapsed().as_secs() > 60 {
                break;
            }
        }

        let captured: Vec<u8> = chunks.iter().flatten().copied().collect();
        let cap_path = dir.join(format!("captured_{}.bin", tag));
        std::fs::write(&cap_path, &captured).expect("write capture");
        println!(
            "[{}] captured {} bytes in {} chunks -> {}",
            tag,
            captured.len(),
            chunks.len(),
            cap_path.display()
        );
        chunks
    }

    /// Dump the grid: '·' marks a never-written/erased cell (empty grapheme),
    /// a real space means a space was written there.
    #[cfg(windows)]
    fn repro_dump_grid(session: &Session, rows: u16) -> Vec<String> {
        let mut out = Vec::new();
        for row in session.state.active_screen().rows.iter().take(rows as usize) {
            let mut s = String::new();
            for cell in &row.cells {
                if cell.is_continuation() {
                    continue;
                }
                if cell.grapheme.is_empty() {
                    s.push('·');
                } else {
                    s.push_str(&cell.grapheme);
                }
            }
            out.push(s);
        }
        out
    }

    #[cfg(windows)]
    const REPRO_TEXT: &str = "じゅげむ じゅげむ ごこうのすりきれ かいじゃりすいぎょの すいぎょうまつ うんらいまつ ふうらいまつ くうねるところに すむところ やぶらこうじの ぶらこうじ パイポパイポ パイポのシューリンガン シューリンガンのグーリンダイ グーリンダイのポンポコピーのポンポコナーの ちょうきゅうめいの ちょうすけ";

    /// Diagnostic harness: drive a real ConPTY through an Ink-style streaming
    /// repaint of a CJK paragraph (what Claude Code does while streaming
    /// markdown) and replay ConPTY's re-encoded byte stream through the
    /// session parser. Hunts the "blank spans inside CJK text" bug.
    /// Run with: cargo test conpty_ink_repaint_cjk_repro -- --nocapture --ignored
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn conpty_ink_repaint_cjk_repro() {
        const COLS: u16 = 58;
        const ROWS: u16 = 24;
        const WRAP: usize = 54; // text columns after the 2-col indent

        let dir = std::env::temp_dir().join("wtmux_conpty_repro");
        std::fs::create_dir_all(&dir).expect("create repro dir");

        let full_lines = repro_wrap_text(REPRO_TEXT, WRAP);
        let chars: Vec<char> = REPRO_TEXT.chars().collect();

        // Storm scenario: fine-grained streaming frames (a few chars at a
        // time), written back-to-back with no pacing — ConPTY renders while
        // the child is mid-write, so its diff optimizer emits partial-line
        // updates against half-drawn states, like a live Claude Code session.
        let step = 3usize;
        let mut frames = Vec::new();
        let mut prev_lines = 0usize;
        let mut end = step;
        loop {
            let end_c = end.min(chars.len());
            let prefix: String = chars[..end_c].iter().collect();
            let lines = repro_wrap_text(&prefix, WRAP);
            frames.push(repro_ink_frame(&lines, prev_lines, true));
            prev_lines = lines.len();
            if end_c == chars.len() {
                break;
            }
            end += step;
        }

        let mut corrupted = 0usize;
        for round in 0..5 {
            let chunks = repro_run_conpty(&dir, &format!("storm{}", round), &frames, false, COLS, ROWS);

            let mut session = Session::new(0, COLS, ROWS);
            for c in &chunks {
                session.feed_bytes(c);
            }

            let grid = repro_dump_grid(&session, ROWS);
            // The final frame's block must appear intact somewhere on screen.
            let missing: Vec<&String> = full_lines
                .iter()
                .filter(|l| !grid.iter().any(|g| g.contains(l.as_str())))
                .collect();
            if missing.is_empty() {
                println!("[round {}] grid OK", round);
            } else {
                corrupted += 1;
                println!("[round {}] CORRUPTED — {} expected lines missing:", round, missing.len());
                for l in &missing {
                    println!("  expected |{}|", l);
                }
                println!("--- grid ---");
                for (i, g) in grid.iter().enumerate() {
                    println!("{:2}|{}|", i, g);
                }
            }
        }
        println!("corrupted rounds: {}/5", corrupted);
    }

    /// Closed-loop render harness: ConPTY bytes → session grid → WmRenderer
    /// output → a simulated host terminal. Compares the host terminal's final
    /// cells against the session grid to catch corruption introduced by the
    /// render path (dirty-row skipping, per-glyph re-anchoring, tail clearing).
    /// Run with: cargo test conpty_render_closed_loop_repro -- --nocapture --ignored
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn conpty_render_closed_loop_repro() {
        const PANE_W: u16 = 60; // outer; Single border → inner 58x20
        const PANE_H: u16 = 22;
        const INNER_W: u16 = 58;
        const INNER_H: u16 = 20;
        const HOST_COLS: u16 = 120;
        const HOST_ROWS: u16 = 40;
        const WRAP: usize = 54;
        const Y_OFFSET: u16 = 1; // tab bar height

        let dir = std::env::temp_dir().join("wtmux_conpty_repro");
        std::fs::create_dir_all(&dir).expect("create repro dir");

        // Filler pushes the cursor near the bottom so the streamed block
        // scrolls the pane while it grows — the scroll + dirty-row path is
        // exactly what a live Claude Code session exercises.
        let mut filler = Vec::new();
        for i in 0..(INNER_H - 4) {
            filler.extend_from_slice(format!("filler {:02} 埋め草テキストです\r\n", i).as_bytes());
        }

        let chars: Vec<char> = REPRO_TEXT.chars().collect();
        let step = 3usize;
        let mut frames = vec![filler];
        let mut prev_lines = 0usize;
        let mut end = step;
        loop {
            let end_c = end.min(chars.len());
            let prefix: String = chars[..end_c].iter().collect();
            let lines = repro_wrap_text(&prefix, WRAP);
            frames.push(repro_ink_frame(&lines, prev_lines, true));
            prev_lines = lines.len();
            if end_c == chars.len() {
                break;
            }
            end += step;
        }

        let full_lines = repro_wrap_text(REPRO_TEXT, WRAP);

        for (variant, render_every) in [(0usize, 1usize), (1, 3)] {
            let chunks = repro_run_conpty(
                &dir,
                &format!("loop{}", variant),
                &frames,
                false,
                INNER_W,
                INNER_H,
            );

            let mut pane = crate::wm::pane::Pane::new(1, PANE_W, PANE_H);
            pane.focused = true;
            let renderer = crate::ui::wm_renderer::WmRenderer::new();
            // The simulated host terminal is a Session so the renderer's
            // output goes through the same UTF-8 decode path as real PTY data.
            let mut host = Session::new(9, HOST_COLS, HOST_ROWS);

            let mut render_count = 0usize;
            let render_dir = dir.clone();
            let mut render = |pane: &mut crate::wm::pane::Pane, host: &mut Session| {
                let screen = pane.session.state.active_screen();
                if screen.full_redraw || screen.has_dirty_lines() {
                    let mut out: Vec<u8> = Vec::new();
                    renderer
                        .render_pane(&mut out, pane, Y_OFFSET, false)
                        .expect("render_pane");
                    pane.session.state.active_screen_mut().clear_dirty();
                    host.feed_bytes(&out);
                    std::fs::write(
                        render_dir.join(format!("render_v{}_{:03}.bin", variant, render_count)),
                        &out,
                    )
                    .expect("write render dump");
                    render_count += 1;
                }
            };

            for (ci, chunk) in chunks.iter().enumerate() {
                pane.session.feed_bytes(chunk);
                if (ci + 1) % render_every == 0 {
                    render(&mut pane, &mut host);
                }
            }
            // Final render pass, like the event loop draining pending output.
            render(&mut pane, &mut host);

            // Compare the host terminal's pane region against the session grid.
            let (inner_x, inner_y) = pane.inner_pos();
            let sess_rows = &pane.session.state.active_screen().rows;
            let mut mismatches = 0usize;
            for row_idx in 0..INNER_H as usize {
                let mut sess_text = String::new();
                for cell in &sess_rows[row_idx].cells {
                    if cell.is_continuation() {
                        continue;
                    }
                    sess_text.push_str(cell.display_char());
                }
                let host_row = &host.state.active_screen().rows
                    [(Y_OFFSET + inner_y) as usize + row_idx];
                let mut host_text = String::new();
                let start = inner_x as usize;
                let end = start + INNER_W as usize;
                for (col, cell) in host_row.cells.iter().enumerate() {
                    if cell.is_continuation() {
                        continue;
                    }
                    let w = cell.width.max(1) as usize;
                    if col >= start && col + w <= end {
                        host_text.push_str(cell.display_char());
                    }
                }
                if sess_text.trim_end() != host_text.trim_end() {
                    mismatches += 1;
                    println!("[variant {}] row {:2} MISMATCH", variant, row_idx);
                    println!("  session |{}|", sess_text.trim_end());
                    println!("  host    |{}|", host_text.trim_end());
                }
            }
            let sess_all: String = (0..INNER_H as usize)
                .map(|r| {
                    sess_rows[r]
                        .cells
                        .iter()
                        .filter(|c| !c.is_continuation())
                        .map(|c| c.display_char())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n");
            let missing: Vec<&String> = full_lines
                .iter()
                .filter(|l| !sess_all.contains(l.as_str()))
                .collect();
            println!(
                "[variant {}] mismatched rows: {}, session grid missing lines: {}",
                variant,
                mismatches,
                missing.len()
            );
        }
    }

    /// Replay wtmux's own render output (the render_v0_*.bin dumps produced by
    /// conpty_render_closed_loop_repro) through a REAL ConPTY/conhost hop and
    /// parse what conhost re-encodes — this is exactly what a host terminal
    /// like WezTerm receives when wtmux runs inside it. Verifies conhost's
    /// wide-char handling doesn't reintroduce stray spaces.
    /// Run after conpty_render_closed_loop_repro with:
    /// cargo test conpty_host_hop_replay -- --nocapture --ignored
    #[cfg(windows)]
    #[test]
    #[ignore]
    fn conpty_host_hop_replay() {
        const HOST_COLS: u16 = 120;
        const HOST_ROWS: u16 = 40;

        let dir = std::env::temp_dir().join("wtmux_conpty_repro");
        let mut frames = Vec::new();
        for i in 0..64 {
            let p = dir.join(format!("render_v0_{:03}.bin", i));
            match std::fs::read(&p) {
                Ok(b) => frames.push(b),
                Err(_) => break,
            }
        }
        assert!(
            !frames.is_empty(),
            "run conpty_render_closed_loop_repro first to produce render dumps"
        );

        let chunks = repro_run_conpty(&dir, "hosthop", &frames, true, HOST_COLS, HOST_ROWS);

        let mut host = Session::new(0, HOST_COLS, HOST_ROWS);
        for c in &chunks {
            host.feed_bytes(c);
        }

        println!("--- host-terminal grid after conhost hop ---");
        for (i, row) in host.state.active_screen().rows.iter().enumerate().take(24) {
            let mut s = String::new();
            for cell in &row.cells {
                if cell.is_continuation() {
                    continue;
                }
                if cell.grapheme.is_empty() {
                    s.push('·');
                } else {
                    s.push_str(&cell.grapheme);
                }
            }
            println!("{:2}|{}|", i, s.trim_end());
        }

        let all: String = host
            .state
            .active_screen()
            .rows
            .iter()
            .map(|r| {
                r.cells
                    .iter()
                    .filter(|c| !c.is_continuation())
                    .map(|c| c.display_char())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        for needle in [
            "じゅげむ じゅげむ ごこうのすりきれ かいじゃりすいぎょ",
            "グーリンダイのポンポコピーのポンポコナーの ちょうきゅ",
        ] {
            assert!(
                all.contains(needle),
                "text corrupted across the conhost hop: {needle} not found"
            );
        }
        println!("host hop OK — no stray spaces after conhost re-encoding");
    }
}
