//! VT sequence parser
//!
//! Parses ANSI/VT100/VT220 escape sequences and updates [`TerminalState`].
//!
//! ## Supported sequences
//!
//! - CSI sequences (cursor movement, SGR attributes, erase, scroll, …)
//! - OSC sequences (window title, shell integration OSC 133/633, …)
//! - DCS / APC / SOS / PM strings (consumed silently)
//! - SGR attributes including SGR 7 (Reverse Video / INVERSE)
//! - Private modes (mouse tracking, alternate screen, bracketed paste, …)
//!
//! ## Multi-byte character handling
//!
//! Single-byte input is processed via [`VtParser::feed`].
//! Decoded Unicode characters from multi-byte UTF-8 sequences must be routed
//! through [`VtParser::feed_char`] so the state machine can consume them
//! silently when inside a DCS/APC/SOS/PM string body.

use super::state::{AttrFlags, Color, TerminalState};

/// Response that needs to be sent back to the PTY
#[derive(Debug, Clone)]
pub enum Response {
    /// Cursor position report: ESC [ row ; col R
    CursorPosition(u16, u16),
    /// Device attributes response
    DeviceAttributes,
    /// Secondary device attributes response
    SecondaryDeviceAttributes,
}

impl Response {
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            Response::CursorPosition(row, col) => {
                format!("\x1b[{};{}R", row, col).into_bytes()
            }
            Response::DeviceAttributes => {
                // VT220 response
                b"\x1b[?62;c".to_vec()
            }
            Response::SecondaryDeviceAttributes => {
                // VT220 response
                b"\x1b[>1;10;0c".to_vec()
            }
        }
    }
}

/// Parser state machine
pub struct VtParser {
    state: ParserState,
    params: Vec<u16>,
    intermediates: Vec<u8>,
    current_param: Option<u16>,
    osc_string: String,
}

#[derive(Clone, Copy, Default, PartialEq)]
enum ParserState {
    #[default]
    Ground,
    Escape,
    EscapeIntermediate,
    CsiEntry,
    CsiParam,
    CsiIntermediate,
    OscString,
    EscapeInOsc,  // ESC received within OSC, waiting for backslash
    /// DCS  (ESC P … ST) — Device Control String; ignored but consumed
    DcsString,
    EscapeInDcs,
    /// APC  (ESC _ … ST) — Application Program Command; ignored but consumed
    ApcString,
    EscapeInApc,
    /// SOS  (ESC X … ST) — Start Of String; ignored but consumed
    SosString,
    EscapeInSos,
    /// PM   (ESC ^ … ST) — Privacy Message; ignored but consumed
    PmString,
    EscapeInPm,
}

impl Default for VtParser {
    fn default() -> Self {
        Self::new()
    }
}

impl VtParser {
    pub fn new() -> Self {
        Self {
            state: ParserState::Ground,
            params: Vec::with_capacity(16),
            intermediates: Vec::with_capacity(4),
            current_param: None,
            osc_string: String::new(),
        }
    }

    /// Feed a decoded Unicode character to the parser.
    ///
    /// Used by `session::feed_bytes` for multi-byte UTF-8 sequences.  Routes
    /// the character through the state machine so that string-body states
    /// (DCS, APC, SOS, PM) can consume it silently rather than printing it.
    ///
    /// For non-BMP / multi-byte characters the parser must be in Ground (or a
    /// string-body) state because escape sequences are always pure ASCII.
    pub fn feed_char(&mut self, ch: char, state: &mut TerminalState) -> Option<Response> {
        // If we are inside a string-body state, consume the character silently.
        match self.state {
            ParserState::DcsString
            | ParserState::ApcString
            | ParserState::SosString
            | ParserState::PmString
            | ParserState::OscString => {
                // Append to OSC string so that titles/paths with non-ASCII
                // characters (e.g. Japanese directory names) are captured correctly.
                if matches!(self.state, ParserState::OscString) {
                    self.osc_string.push(ch);
                }
                return None;
            }
            // EscapeIn* states are entered on ESC within a string; since ESC
            // is single-byte, we should never get a multi-byte char here, but
            // handle gracefully by staying in current state.
            ParserState::EscapeInDcs
            | ParserState::EscapeInApc
            | ParserState::EscapeInSos
            | ParserState::EscapeInPm
            | ParserState::EscapeInOsc => return None,
            // Any other state: must be Ground (multi-byte chars can't appear
            // inside CSI/Escape sequences).  Write directly to the screen.
            _ => {}
        }

        // Ground state — write the character to the screen buffer.
        state.put_char(ch);
        None
    }

    /// Feed a single byte to the parser
    pub fn feed(&mut self, byte: u8, state: &mut TerminalState) -> Option<Response> {
        // Handle C0 controls anywhere (except in OSC-related states)
        if byte < 0x20
            && self.state != ParserState::OscString
            && self.state != ParserState::EscapeInOsc
            && self.state != ParserState::DcsString
            && self.state != ParserState::EscapeInDcs
            && self.state != ParserState::ApcString
            && self.state != ParserState::EscapeInApc
            && self.state != ParserState::SosString
            && self.state != ParserState::EscapeInSos
            && self.state != ParserState::PmString
            && self.state != ParserState::EscapeInPm
        {
            match byte {
                0x1B => {
                    self.enter_escape();
                    return None;
                }
                0x07 => return None, // BEL - ignore
                0x08 => {
                    state.backspace();
                    return None;
                }
                0x09 => {
                    state.horizontal_tab();
                    return None;
                }
                0x0A | 0x0B | 0x0C => {
                    state.linefeed();
                    return None;
                }
                0x0D => {
                    state.carriage_return();
                    return None;
                }
                _ => return None,
            }
        }

        match self.state {
            ParserState::Ground => self.ground(byte, state),
            ParserState::Escape => self.escape(byte, state),
            ParserState::EscapeIntermediate => self.escape_intermediate(byte, state),
            ParserState::CsiEntry => self.csi_entry(byte, state),
            ParserState::CsiParam => self.csi_param(byte, state),
            ParserState::CsiIntermediate => self.csi_intermediate(byte, state),
            ParserState::OscString => self.osc_string_state(byte, state),
            ParserState::EscapeInOsc => self.escape_in_osc(byte, state),
            // Ignored string sequences — consume until ST
            ParserState::DcsString  => { self.ignore_string_byte(byte, ParserState::DcsString,  ParserState::EscapeInDcs);  None }
            ParserState::EscapeInDcs => { self.escape_in_ignored(byte, ParserState::DcsString);  None }
            ParserState::ApcString  => { self.ignore_string_byte(byte, ParserState::ApcString,  ParserState::EscapeInApc);  None }
            ParserState::EscapeInApc => { self.escape_in_ignored(byte, ParserState::ApcString);  None }
            ParserState::SosString  => { self.ignore_string_byte(byte, ParserState::SosString,  ParserState::EscapeInSos);  None }
            ParserState::EscapeInSos => { self.escape_in_ignored(byte, ParserState::SosString);  None }
            ParserState::PmString   => { self.ignore_string_byte(byte, ParserState::PmString,   ParserState::EscapeInPm);   None }
            ParserState::EscapeInPm  => { self.escape_in_ignored(byte, ParserState::PmString);   None }
        }
    }

    /// Handle ESC received within OSC sequence
    fn escape_in_osc(&mut self, byte: u8, state: &mut TerminalState) -> Option<Response> {
        if byte == 0x5C {
            // ST (ESC \ = 0x5C) - String Terminator
            self.execute_osc(state);
            self.state = ParserState::Ground;
        } else {
            // Not ST, execute OSC and process this byte as new escape sequence
            self.execute_osc(state);
            // Re-enter escape mode and process this byte
            self.enter_escape();
            return self.escape(byte, state);
        }
        None
    }

    fn enter_escape(&mut self) {
        self.state = ParserState::Escape;
        self.params.clear();
        self.intermediates.clear();
        self.current_param = None;
    }

    fn ground(&mut self, byte: u8, state: &mut TerminalState) -> Option<Response> {
        if byte >= 0x20 && byte < 0x7F {
            state.put_char(byte as char);
        } else if byte >= 0x80 {
            // UTF-8 or extended ASCII - pass through for now
            // The decoder should handle this before reaching here
            state.put_char(byte as char);
        }
        None
    }

    fn escape(&mut self, byte: u8, state: &mut TerminalState) -> Option<Response> {
        match byte {
            b'[' => {
                self.state = ParserState::CsiEntry;
                self.params.clear();
                self.intermediates.clear();
                self.current_param = None;
            }
            b']' => {
                self.state = ParserState::OscString;
                self.osc_string.clear();
            }
            b'7' => {
                // DECSC - Save cursor
                state.save_cursor();
                self.state = ParserState::Ground;
            }
            b'8' => {
                // DECRC - Restore cursor
                state.restore_cursor();
                self.state = ParserState::Ground;
            }
            b'D' => {
                // IND - Index
                state.index();
                self.state = ParserState::Ground;
            }
            b'E' => {
                // NEL - Next line
                state.carriage_return();
                state.linefeed();
                self.state = ParserState::Ground;
            }
            b'M' => {
                // RI - Reverse index
                state.reverse_index();
                self.state = ParserState::Ground;
            }
            b'c' => {
                // RIS - Full reset
                *state = TerminalState::new(state.cols, state.rows);
                self.state = ParserState::Ground;
            }
            // String-body sequences — enter ignore state, consume until ST
            b'P' => {
                // DCS — Device Control String (e.g. tmux passthrough, Sixel, iTerm2)
                self.state = ParserState::DcsString;
            }
            b'_' => {
                // APC — Application Program Command (e.g. Kitty, oh-my-posh window title)
                self.state = ParserState::ApcString;
            }
            b'X' => {
                // SOS — Start Of String
                self.state = ParserState::SosString;
            }
            b'^' => {
                // PM — Privacy Message
                self.state = ParserState::PmString;
            }
            0x20..=0x2F => {
                // Intermediate bytes
                self.intermediates.push(byte);
                self.state = ParserState::EscapeIntermediate;
            }
            _ => {
                self.state = ParserState::Ground;
            }
        }
        None
    }

    fn escape_intermediate(&mut self, byte: u8, _state: &mut TerminalState) -> Option<Response> {
        match byte {
            0x20..=0x2F => {
                self.intermediates.push(byte);
            }
            0x30..=0x7E => {
                // Final byte - execute and return to ground
                // Most of these are charset selections which we ignore for now
                self.state = ParserState::Ground;
            }
            _ => {
                self.state = ParserState::Ground;
            }
        }
        None
    }

    fn csi_entry(&mut self, byte: u8, state: &mut TerminalState) -> Option<Response> {
        match byte {
            b'0'..=b'9' => {
                self.current_param = Some((byte - b'0') as u16);
                self.state = ParserState::CsiParam;
            }
            b';' => {
                self.params.push(0);
                self.state = ParserState::CsiParam;
            }
            b'?' | b'>' | b'!' | b'=' => {
                self.intermediates.push(byte);
            }
            0x20..=0x2F => {
                self.intermediates.push(byte);
                self.state = ParserState::CsiIntermediate;
            }
            0x40..=0x7E => {
                // Final byte
                return self.execute_csi(byte, state);
            }
            _ => {
                self.state = ParserState::Ground;
            }
        }
        None
    }

    fn csi_param(&mut self, byte: u8, state: &mut TerminalState) -> Option<Response> {
        match byte {
            b'0'..=b'9' => {
                let digit = (byte - b'0') as u16;
                self.current_param = Some(
                    self.current_param.unwrap_or(0).saturating_mul(10).saturating_add(digit)
                );
            }
            b';' => {
                self.params.push(self.current_param.unwrap_or(0));
                self.current_param = None;
            }
            b':' => {
                // Subparameter separator (used in SGR)
                // For simplicity, treat as regular separator
                self.params.push(self.current_param.unwrap_or(0));
                self.current_param = None;
            }
            0x20..=0x2F => {
                if let Some(p) = self.current_param.take() {
                    self.params.push(p);
                }
                self.intermediates.push(byte);
                self.state = ParserState::CsiIntermediate;
            }
            0x40..=0x7E => {
                if let Some(p) = self.current_param.take() {
                    self.params.push(p);
                }
                return self.execute_csi(byte, state);
            }
            _ => {
                self.state = ParserState::Ground;
            }
        }
        None
    }

    fn csi_intermediate(&mut self, byte: u8, state: &mut TerminalState) -> Option<Response> {
        match byte {
            0x20..=0x2F => {
                self.intermediates.push(byte);
            }
            0x40..=0x7E => {
                return self.execute_csi(byte, state);
            }
            _ => {
                self.state = ParserState::Ground;
            }
        }
        None
    }

    fn osc_string_state(&mut self, byte: u8, state: &mut TerminalState) -> Option<Response> {
        match byte {
            0x07 => {
                // BEL terminates OSC
                self.execute_osc(state);
                self.state = ParserState::Ground;
            }
            0x1B => {
                // Could be ST (ESC \)
                // Move to EscapeInOsc state to check for backslash
                self.state = ParserState::EscapeInOsc;
            }
            0x9C => {
                // ST (String Terminator)
                self.execute_osc(state);
                self.state = ParserState::Ground;
            }
            _ => {
                self.osc_string.push(byte as char);
            }
        }
        None
    }

    fn execute_csi(&mut self, final_byte: u8, state: &mut TerminalState) -> Option<Response> {
        let is_private = self.intermediates.contains(&b'?');
        let is_gt = self.intermediates.contains(&b'>');
        let params = &self.params;

        let response = match (is_private, is_gt, final_byte) {
            // Cursor movement
            (false, false, b'A') => {
                state.cursor_up(params.first().copied().unwrap_or(1).max(1));
                None
            }
            (false, false, b'B') => {
                state.cursor_down(params.first().copied().unwrap_or(1).max(1));
                None
            }
            (false, false, b'C') => {
                state.cursor_forward(params.first().copied().unwrap_or(1).max(1));
                None
            }
            (false, false, b'D') => {
                state.cursor_backward(params.first().copied().unwrap_or(1).max(1));
                None
            }
            (false, false, b'E') => {
                // CNL - Cursor Next Line
                let n = params.first().copied().unwrap_or(1).max(1);
                state.cursor_down(n);
                state.carriage_return();
                None
            }
            (false, false, b'F') => {
                // CPL - Cursor Previous Line
                let n = params.first().copied().unwrap_or(1).max(1);
                state.cursor_up(n);
                state.carriage_return();
                None
            }
            (false, false, b'G') => {
                // CHA - Cursor Character Absolute
                let col = params.first().copied().unwrap_or(1);
                state.active_cursor_mut().col = col.saturating_sub(1).min(state.cols - 1);
                None
            }
            (false, false, b'H') | (false, false, b'f') => {
                // CUP - Cursor Position
                let row = params.first().copied().unwrap_or(1);
                let col = params.get(1).copied().unwrap_or(1);
                state.cursor_position(row, col);
                None
            }
            (false, false, b'd') => {
                // VPA - Line Position Absolute
                let row = params.first().copied().unwrap_or(1);
                state.active_cursor_mut().row = row.saturating_sub(1).min(state.rows - 1);
                None
            }

            // Erase
            (false, false, b'J') => {
                state.erase_in_display(params.first().copied().unwrap_or(0));
                None
            }
            (false, false, b'K') => {
                state.erase_in_line(params.first().copied().unwrap_or(0));
                None
            }

            // Line operations
            (false, false, b'L') => {
                state.insert_lines(params.first().copied().unwrap_or(1).max(1));
                None
            }
            (false, false, b'M') => {
                state.delete_lines(params.first().copied().unwrap_or(1).max(1));
                None
            }

            // Character operations
            (false, false, b'@') => {
                // ICH - Insert Characters
                let n = params.first().copied().unwrap_or(1).max(1) as usize;
                let cursor = state.active_cursor().clone();
                let screen = state.active_screen_mut();
                let row = cursor.row as usize;
                let col = cursor.col as usize;

                // Shift cells right
                for _ in 0..n {
                    if col < screen.rows[row].cells.len() {
                        screen.rows[row].cells.pop();
                        screen.rows[row].cells.insert(col, super::state::Cell::default());
                    }
                }
                screen.mark_dirty(row);
                None
            }
            (false, false, b'P') => {
                // DCH - Delete Characters
                let n = params.first().copied().unwrap_or(1).max(1) as usize;
                let cursor = state.active_cursor().clone();
                let screen = state.active_screen_mut();
                let row = cursor.row as usize;
                let col = cursor.col as usize;

                for _ in 0..n {
                    if col < screen.rows[row].cells.len() {
                        screen.rows[row].cells.remove(col);
                        screen.rows[row].cells.push(super::state::Cell::default());
                    }
                }
                screen.mark_dirty(row);
                None
            }
            (false, false, b'X') => {
                // ECH - Erase Characters
                let n = params.first().copied().unwrap_or(1).max(1) as usize;
                let cursor = state.active_cursor().clone();
                let row = cursor.row as usize;
                let col = cursor.col as usize;
                let attrs = state.current_attrs.clone();
                
                let screen = state.active_screen_mut();

                for i in 0..n {
                    if col + i < screen.rows[row].cells.len() {
                        screen.rows[row].cells[col + i].clear(&attrs);
                    }
                }
                screen.mark_dirty(row);
                None
            }

            // Scroll
            (false, false, b'S') => {
                state.scroll_up(params.first().copied().unwrap_or(1).max(1));
                None
            }
            (false, false, b'T') => {
                state.scroll_down(params.first().copied().unwrap_or(1).max(1));
                None
            }

            // Scroll region
            (false, false, b'r') => {
                let top = params.first().copied().unwrap_or(1);
                let bottom = params.get(1).copied().unwrap_or(state.rows);
                state.set_scroll_region(top, bottom);
                state.cursor_position(1, 1);
                None
            }

            // SGR - Select Graphic Rendition
            (false, false, b'm') => {
                self.execute_sgr(params, state);
                None
            }

            // Save/restore cursor
            (false, false, b's') => {
                state.save_cursor();
                None
            }
            (false, false, b'u') => {
                state.restore_cursor();
                None
            }

            // Device Status Report
            (false, false, b'n') => {
                match params.first() {
                    Some(5) => {
                        // Status report - we're OK
                        // Would send back CSI 0 n
                        None
                    }
                    Some(6) => {
                        // Cursor position report
                        let cursor = state.active_cursor();
                        Some(Response::CursorPosition(cursor.row + 1, cursor.col + 1))
                    }
                    _ => None,
                }
            }

            // Device Attributes
            (false, false, b'c') => {
                Some(Response::DeviceAttributes)
            }
            (false, true, b'c') => {
                Some(Response::SecondaryDeviceAttributes)
            }

            // Private modes (DEC)
            (true, false, b'h') => {
                for &p in params {
                    state.set_private_mode(p, true);
                }
                None
            }
            (true, false, b'l') => {
                for &p in params {
                    state.set_private_mode(p, false);
                }
                None
            }

            // Standard modes
            (false, false, b'h') => {
                for &p in params {
                    match p {
                        4 => state.modes.insert_mode = true,
                        20 => state.modes.linefeed_newline = true,
                        _ => {}
                    }
                }
                None
            }
            (false, false, b'l') => {
                for &p in params {
                    match p {
                        4 => state.modes.insert_mode = false,
                        20 => state.modes.linefeed_newline = false,
                        _ => {}
                    }
                }
                None
            }

            _ => {
                // Check for DECSCUSR (CSI Ps SP q) - Set cursor style
                if final_byte == b'q' && self.intermediates.contains(&b' ') {
                    let shape = params.first().copied().unwrap_or(0) as u8;
                    state.active_cursor_mut().shape = super::state::CursorShape::from_decscusr(shape);
                    return None;
                }
                
                // Unknown sequence
                tracing::debug!(
                    "Unknown CSI: intermediates={:?}, params={:?}, final={:?}",
                    self.intermediates,
                    params,
                    final_byte as char
                );
                None
            }
        };

        self.state = ParserState::Ground;
        response
    }

    fn execute_sgr(&self, params: &[u16], state: &mut TerminalState) {
        if params.is_empty() {
            state.current_attrs.reset();
            return;
        }

        let mut iter = params.iter().peekable();

        while let Some(&param) = iter.next() {
            match param {
                0 => state.current_attrs.reset(),
                1 => state.current_attrs.flags |= AttrFlags::BOLD,
                2 => state.current_attrs.flags |= AttrFlags::DIM,
                3 => state.current_attrs.flags |= AttrFlags::ITALIC,
                4 => state.current_attrs.flags |= AttrFlags::UNDERLINE,
                5 => state.current_attrs.flags |= AttrFlags::BLINK,
                7 => state.current_attrs.flags |= AttrFlags::INVERSE,
                8 => state.current_attrs.flags |= AttrFlags::HIDDEN,
                9 => state.current_attrs.flags |= AttrFlags::STRIKETHROUGH,

                22 => state.current_attrs.flags &= !(AttrFlags::BOLD | AttrFlags::DIM),
                23 => state.current_attrs.flags &= !AttrFlags::ITALIC,
                24 => state.current_attrs.flags &= !AttrFlags::UNDERLINE,
                25 => state.current_attrs.flags &= !AttrFlags::BLINK,
                27 => state.current_attrs.flags &= !AttrFlags::INVERSE,
                28 => state.current_attrs.flags &= !AttrFlags::HIDDEN,
                29 => state.current_attrs.flags &= !AttrFlags::STRIKETHROUGH,

                // Foreground colors (standard)
                30..=37 => {
                    state.current_attrs.fg = Color::Indexed((param - 30) as u8);
                }
                38 => {
                    // Extended foreground
                    if let Some(&mode) = iter.next() {
                        match mode {
                            5 => {
                                // 256 color
                                if let Some(&n) = iter.next() {
                                    state.current_attrs.fg = Color::Indexed(n as u8);
                                }
                            }
                            2 => {
                                // RGB
                                let r = iter.next().copied().unwrap_or(0) as u8;
                                let g = iter.next().copied().unwrap_or(0) as u8;
                                let b = iter.next().copied().unwrap_or(0) as u8;
                                state.current_attrs.fg = Color::Rgb(r, g, b);
                            }
                            _ => {}
                        }
                    }
                }
                39 => state.current_attrs.fg = Color::Default,

                // Background colors (standard)
                40..=47 => {
                    state.current_attrs.bg = Color::Indexed((param - 40) as u8);
                }
                48 => {
                    // Extended background
                    if let Some(&mode) = iter.next() {
                        match mode {
                            5 => {
                                if let Some(&n) = iter.next() {
                                    state.current_attrs.bg = Color::Indexed(n as u8);
                                }
                            }
                            2 => {
                                let r = iter.next().copied().unwrap_or(0) as u8;
                                let g = iter.next().copied().unwrap_or(0) as u8;
                                let b = iter.next().copied().unwrap_or(0) as u8;
                                state.current_attrs.bg = Color::Rgb(r, g, b);
                            }
                            _ => {}
                        }
                    }
                }
                49 => state.current_attrs.bg = Color::Default,

                // Bright foreground
                90..=97 => {
                    state.current_attrs.fg = Color::Indexed((param - 90 + 8) as u8);
                }
                // Bright background
                100..=107 => {
                    state.current_attrs.bg = Color::Indexed((param - 100 + 8) as u8);
                }

                _ => {}
            }
        }
    }

    /// Consume a byte inside an ignored string (DCS / APC / SOS / PM).
    /// Transitions to `esc_state` on ESC, returns to Ground on BEL / ST (0x9C).
    fn ignore_string_byte(&mut self, byte: u8, _str_state: ParserState, esc_state: ParserState) {
        match byte {
            0x07 => { self.state = ParserState::Ground; } // BEL = ST for OSC compat
            0x1B => { self.state = esc_state; }
            0x9C => { self.state = ParserState::Ground; } // 8-bit ST
            _ => {} // consume
        }
    }

    /// Called after ESC inside an ignored string; back-slash (0x5C) = ST → Ground.
    fn escape_in_ignored(&mut self, byte: u8, str_state: ParserState) {
        if byte == 0x5C {
            // ESC \ = String Terminator
            self.state = ParserState::Ground;
        } else {
            // Not ST — stay in the string and process this byte normally
            self.state = str_state;
        }
    }

    fn execute_osc(&mut self, state: &mut TerminalState) {
        // Parse OSC: "code;text"
        if let Some(pos) = self.osc_string.find(';') {
            let code = &self.osc_string[..pos];
            let text = &self.osc_string[pos + 1..];

            match code {
                "0" | "1" | "2" => {
                    // Set title
                    state.title = text.to_string();
                }
                // ── Shell Integration ─────────────────────────────────────
                // OSC 133 : FinalTerm / standard shell integration
                // OSC 633 : VS Code extension (superset of OSC 133)
                //
                // Both use the same A/B/C/D marker scheme.
                // OSC 633 additionally has E (set custom property) which we ignore.
                "133" | "633" => {
                    self.handle_shell_integration(text, state);
                }
                _ => {}
            }
        } else {
            // OSC with no semicolon (e.g. OSC 133 ; A with empty text part)
            // Shouldn't normally occur but guard against it.
        }
    }

    /// Handle OSC 133 / 633 shell-integration markers.
    ///
    /// Marker grammar:
    ///   A          – prompt start
    ///   B          – prompt end  (cursor is now at input start)
    ///   C          – command executing (Enter was pressed)
    ///   D[;code]   – command finished (optional exit code)
    ///   E;prop;val – VS Code property (ignored)
    fn handle_shell_integration(&self, text: &str, state: &mut TerminalState) {
        // text is everything after the first ';', e.g. "A", "B", "C", "D;0"
        // Split on ';' to get marker and optional arguments
        let (marker, args) = if let Some(p) = text.find(';') {
            (&text[..p], Some(&text[p + 1..]))
        } else {
            (text, None)
        };

        match marker {
            "A" => {
                // Prompt start: clear previous confirmed command
                state.shell_integration.on_prompt_start();
            }
            "B" => {
                // Prompt end: record where the user's input begins
                let col = state.active_cursor().col;
                let row = state.active_cursor().row;
                state.shell_integration.on_prompt_end(col, row);
            }
            "C" => {
                // Command executing: extract command text from screen buffer.
                // The cursor has already moved past the command (or is at the
                // end of the input line).  We use the prompt_end position
                // recorded on marker B to know where the command starts.
                let command = Self::extract_command_from_screen(state);
                state.shell_integration.on_command_start(command);
            }
            "D" => {
                // Command finished: parse optional exit code
                let exit_code = args
                    .and_then(|s| s.parse::<i32>().ok());
                state.shell_integration.on_command_done(exit_code);
            }
            // "E" (VS Code property) and anything else – ignore
            _ => {}
        }
    }

    /// Extract the command text from the screen buffer.
    ///
    /// Uses `shell_integration.prompt_end_col/row` as the start of user input
    /// and reads up to (but not including) the cursor position.
    /// Falls back to the entire cursor row if no prompt_end is recorded.
    fn extract_command_from_screen(state: &TerminalState) -> String {
        let cursor = state.active_cursor();
        let screen = state.active_screen();

        // Determine the row and starting column of the command
        let (cmd_row, start_col) = if let (Some(row), Some(col)) = (
            state.shell_integration.prompt_end_row,
            state.shell_integration.prompt_end_col,
        ) {
            (row as usize, col as usize)
        } else {
            // No prompt_end recorded – use beginning of cursor row
            (cursor.row as usize, 0)
        };

        let Some(row_data) = screen.get_row_at(cmd_row) else {
            return String::new();
        };

        // Collect cells from start_col onward, stop at cursor column
        // (or end of row if cursor has moved to next line)
        let end_col = if cursor.row as usize == cmd_row {
            cursor.col as usize
        } else {
            row_data.cells.len()
        };

        let mut cmd = String::new();
        for cell in row_data.cells.iter().skip(start_col).take(end_col.saturating_sub(start_col)) {
            if !cell.is_continuation() {
                if cell.grapheme.is_empty() {
                    cmd.push(' ');
                } else {
                    cmd.push_str(&cell.grapheme);
                }
            }
        }
        cmd.trim_end().to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_movement() {
        let mut state = TerminalState::new(80, 24);
        let mut parser = VtParser::new();

        // Move to position (5, 10)
        for byte in b"\x1b[5;10H" {
            parser.feed(*byte, &mut state);
        }

        assert_eq!(state.active_cursor().row, 4);
        assert_eq!(state.active_cursor().col, 9);
    }

    #[test]
    fn test_sgr_colors() {
        let mut state = TerminalState::new(80, 24);
        let mut parser = VtParser::new();

        // Set red foreground
        for byte in b"\x1b[31m" {
            parser.feed(*byte, &mut state);
        }

        assert_eq!(state.current_attrs.fg, Color::Indexed(1));
    }
}
