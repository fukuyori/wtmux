//! Session management
//!
//! Manages shell sessions, handling I/O between PTY and terminal state.

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
            #[cfg(windows)]
            pty: None,
            running: Arc::new(AtomicBool::new(false)),
            #[cfg(windows)]
            reader_thread: None,
            #[cfg(windows)]
            output_rx: None,
        }
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

    /// Feed raw bytes into the terminal
    pub fn feed_bytes(&mut self, bytes: &[u8]) {
        // ConPTY always outputs UTF-8
        // Process byte by byte, handling UTF-8 sequences
        let mut i = 0;
        while i < bytes.len() {
            let b = bytes[i];
            
            // Control characters and escape sequences
            if b < 0x20 || b == 0x7f {
                if let Some(response) = self.parser.feed(b, &mut self.state) {
                    self.send_response(response);
                }
                i += 1;
                continue;
            }
            
            // ASCII printable
            if b < 0x80 {
                if let Some(response) = self.parser.feed(b, &mut self.state) {
                    self.send_response(response);
                }
                i += 1;
                continue;
            }
            
            // UTF-8 multi-byte sequence
            let seq_len = if b & 0xE0 == 0xC0 { 2 }
                else if b & 0xF0 == 0xE0 { 3 }
                else if b & 0xF8 == 0xF0 { 4 }
                else { 1 }; // Invalid, skip
            
            if i + seq_len <= bytes.len() {
                if let Ok(s) = std::str::from_utf8(&bytes[i..i+seq_len]) {
                    for ch in s.chars() {
                        self.state.put_char(ch);
                    }
                    i += seq_len;
                    continue;
                }
            }
            
            // Invalid or incomplete sequence, skip byte
            i += 1;
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
