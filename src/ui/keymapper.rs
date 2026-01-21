//! Key mapping for terminal input
//!
//! Converts key events to VT sequences for PTY input.

use bitflags::bitflags;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind, MouseButton};

use crate::core::term::TerminalModes;

bitflags! {
    /// Modifier keys
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct Modifiers: u8 {
        const SHIFT = 0b0001;
        const CTRL  = 0b0010;
        const ALT   = 0b0100;
    }
}

impl From<KeyModifiers> for Modifiers {
    fn from(mods: KeyModifiers) -> Self {
        let mut result = Modifiers::empty();
        if mods.contains(KeyModifiers::SHIFT) {
            result |= Modifiers::SHIFT;
        }
        if mods.contains(KeyModifiers::CONTROL) {
            result |= Modifiers::CTRL;
        }
        if mods.contains(KeyModifiers::ALT) {
            result |= Modifiers::ALT;
        }
        result
    }
}

/// Key mapper for converting key events to bytes
pub struct KeyMapper;

impl KeyMapper {
    /// Map a crossterm KeyEvent to bytes for PTY (simplified, without modes)
    pub fn map_key(event: &KeyEvent) -> Vec<u8> {
        let modes = TerminalModes::default();
        Self::map(event, &modes).unwrap_or_default()
    }

    /// Map a crossterm KeyEvent to bytes for PTY
    pub fn map(event: &KeyEvent, modes: &TerminalModes) -> Option<Vec<u8>> {
        let mods = Modifiers::from(event.modifiers);

        match event.code {
            // Character keys
            KeyCode::Char(ch) => Some(Self::map_char(ch, mods)),

            // Enter
            KeyCode::Enter => {
                if modes.linefeed_newline {
                    Some(vec![0x0D, 0x0A])
                } else {
                    Some(vec![0x0D])
                }
            }

            // Backspace
            KeyCode::Backspace => {
                if mods.contains(Modifiers::ALT) {
                    Some(vec![0x1B, 0x7F])
                } else {
                    Some(vec![0x7F])
                }
            }

            // Tab
            KeyCode::Tab => {
                if mods.contains(Modifiers::SHIFT) {
                    Some(b"\x1b[Z".to_vec())
                } else {
                    Some(vec![0x09])
                }
            }

            // Escape
            KeyCode::Esc => Some(vec![0x1B]),

            // Arrow keys
            KeyCode::Up => Some(Self::arrow_key(b'A', mods, modes)),
            KeyCode::Down => Some(Self::arrow_key(b'B', mods, modes)),
            KeyCode::Right => Some(Self::arrow_key(b'C', mods, modes)),
            KeyCode::Left => Some(Self::arrow_key(b'D', mods, modes)),

            // Navigation keys
            KeyCode::Home => Some(Self::special_key(b'H', mods)),
            KeyCode::End => Some(Self::special_key(b'F', mods)),
            KeyCode::PageUp => Some(Self::tilde_key(5, mods)),
            KeyCode::PageDown => Some(Self::tilde_key(6, mods)),
            KeyCode::Insert => Some(Self::tilde_key(2, mods)),
            KeyCode::Delete => Some(Self::tilde_key(3, mods)),

            // Function keys
            KeyCode::F(n) => Some(Self::function_key(n, mods)),

            _ => None,
        }
    }

    /// Map a character with modifiers
    fn map_char(ch: char, mods: Modifiers) -> Vec<u8> {
        // Ctrl + letter = control character
        if mods.contains(Modifiers::CTRL) && !mods.contains(Modifiers::ALT) {
            if ch.is_ascii_lowercase() {
                let ctrl_code = (ch as u8) - b'a' + 1;
                return vec![ctrl_code];
            } else if ch.is_ascii_uppercase() {
                let ctrl_code = (ch as u8) - b'A' + 1;
                return vec![ctrl_code];
            } else {
                // Special Ctrl combinations
                match ch {
                    '@' | '`' | ' ' => return vec![0x00], // Ctrl+@ = NUL
                    '[' => return vec![0x1B],             // Ctrl+[ = ESC
                    '\\' => return vec![0x1C],            // Ctrl+\ = FS
                    ']' => return vec![0x1D],             // Ctrl+] = GS
                    '^' | '~' => return vec![0x1E],       // Ctrl+^ = RS
                    '_' | '?' => return vec![0x1F],       // Ctrl+_ = US
                    _ => {}
                }
            }
        }

        // Ctrl + Alt + letter
        if mods.contains(Modifiers::CTRL) && mods.contains(Modifiers::ALT) {
            if ch.is_ascii_alphabetic() {
                let ctrl_code = (ch.to_ascii_lowercase() as u8) - b'a' + 1;
                return vec![0x1B, ctrl_code];
            }
        }

        // Alt + key = ESC + key
        if mods.contains(Modifiers::ALT) && !mods.contains(Modifiers::CTRL) {
            let mut bytes = vec![0x1B];
            bytes.extend(ch.to_string().as_bytes());
            return bytes;
        }

        // Normal character
        ch.to_string().into_bytes()
    }

    /// Arrow key sequence
    fn arrow_key(key: u8, mods: Modifiers, modes: &TerminalModes) -> Vec<u8> {
        let has_mods = !mods.is_empty();

        if has_mods {
            // With modifiers: ESC [ 1 ; <mod> <key>
            let mod_code = Self::modifier_code(mods);
            format!("\x1b[1;{}{}", mod_code, key as char).into_bytes()
        } else if modes.application_cursor {
            // Application mode: ESC O <key>
            vec![0x1B, b'O', key]
        } else {
            // Normal mode: ESC [ <key>
            vec![0x1B, b'[', key]
        }
    }

    /// Special key (Home, End) sequence
    fn special_key(key: u8, mods: Modifiers) -> Vec<u8> {
        if mods.is_empty() {
            vec![0x1B, b'[', key]
        } else {
            let mod_code = Self::modifier_code(mods);
            format!("\x1b[1;{}{}", mod_code, key as char).into_bytes()
        }
    }

    /// Tilde key sequence (PageUp, PageDown, Insert, Delete)
    fn tilde_key(code: u8, mods: Modifiers) -> Vec<u8> {
        if mods.is_empty() {
            format!("\x1b[{}~", code).into_bytes()
        } else {
            let mod_code = Self::modifier_code(mods);
            format!("\x1b[{};{}~", code, mod_code).into_bytes()
        }
    }

    /// Function key sequence
    fn function_key(n: u8, mods: Modifiers) -> Vec<u8> {
        let base = match n {
            1 => b"\x1bOP".to_vec(),
            2 => b"\x1bOQ".to_vec(),
            3 => b"\x1bOR".to_vec(),
            4 => b"\x1bOS".to_vec(),
            5 => b"\x1b[15~".to_vec(),
            6 => b"\x1b[17~".to_vec(),
            7 => b"\x1b[18~".to_vec(),
            8 => b"\x1b[19~".to_vec(),
            9 => b"\x1b[20~".to_vec(),
            10 => b"\x1b[21~".to_vec(),
            11 => b"\x1b[23~".to_vec(),
            12 => b"\x1b[24~".to_vec(),
            _ => return vec![],
        };

        if mods.is_empty() {
            base
        } else {
            // Convert to modified form
            let mod_code = Self::modifier_code(mods);
            match n {
                1..=4 => {
                    // ESC O X -> ESC [ 1 ; mod X
                    let key = base[2];
                    format!("\x1b[1;{}{}", mod_code, key as char).into_bytes()
                }
                _ => {
                    // ESC [ n ~ -> ESC [ n ; mod ~
                    let code_str = String::from_utf8_lossy(&base[2..base.len() - 1]);
                    format!("\x1b[{};{}~", code_str, mod_code).into_bytes()
                }
            }
        }
    }

    /// Calculate xterm modifier code
    fn modifier_code(mods: Modifiers) -> u8 {
        1 + if mods.contains(Modifiers::SHIFT) { 1 } else { 0 }
            + if mods.contains(Modifiers::ALT) { 2 } else { 0 }
            + if mods.contains(Modifiers::CTRL) { 4 } else { 0 }
    }
    
    /// Encode mouse event to terminal escape sequence for passthrough to child applications.
    ///
    /// # Arguments
    /// * `event` - The mouse event with pane-relative coordinates
    /// * `sgr_mode` - Whether SGR extended mouse mode (1006) is enabled
    /// * `urxvt_mode` - Whether URXVT mouse mode (1015) is enabled
    ///
    /// # Returns
    /// The encoded escape sequence bytes, or empty if event cannot be encoded
    pub fn encode_mouse_event(
        event: &MouseEvent,
        sgr_mode: bool,
        urxvt_mode: bool,
    ) -> Vec<u8> {
        let (button, pressed) = match event.kind {
            MouseEventKind::Down(btn) => (Self::mouse_button_code(btn), true),
            MouseEventKind::Up(btn) => (Self::mouse_button_code(btn), false),
            MouseEventKind::Drag(btn) => (Self::mouse_button_code(btn) + 32, true),
            MouseEventKind::Moved => (35, true), // No button, movement only
            MouseEventKind::ScrollUp => (64, true),
            MouseEventKind::ScrollDown => (65, true),
            MouseEventKind::ScrollLeft => (66, true),
            MouseEventKind::ScrollRight => (67, true),
        };
        
        // Add modifier keys to button code
        let mut cb = button;
        if event.modifiers.contains(KeyModifiers::SHIFT) {
            cb += 4;
        }
        if event.modifiers.contains(KeyModifiers::ALT) {
            cb += 8;
        }
        if event.modifiers.contains(KeyModifiers::CONTROL) {
            cb += 16;
        }
        
        // 1-based coordinates for terminal protocol
        let x = event.column.saturating_add(1);
        let y = event.row.saturating_add(1);
        
        if sgr_mode {
            // SGR mode: \x1b[<Cb;Cx;CyM (press) or \x1b[<Cb;Cx;Cym (release)
            let suffix = if pressed { 'M' } else { 'm' };
            format!("\x1b[<{};{};{}{}", cb, x, y, suffix).into_bytes()
        } else if urxvt_mode {
            // URXVT mode: \x1b[Cb;Cx;CyM
            format!("\x1b[{};{};{}M", cb + 32, x, y).into_bytes()
        } else {
            // X10 mode: \x1b[MCbCxCy (encoded as bytes + 32)
            // Only works for coordinates <= 223
            if x <= 223 && y <= 223 {
                vec![0x1b, b'[', b'M', (cb + 32) as u8, (x as u8 + 32), (y as u8 + 32)]
            } else {
                // Coordinates out of range for X10 mode
                vec![]
            }
        }
    }
    
    /// Convert crossterm MouseButton to protocol button code
    fn mouse_button_code(button: MouseButton) -> u8 {
        match button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            MouseButton::Right => 2,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_event(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn test_char_keys() {
        let modes = TerminalModes::default();

        // Normal character
        let event = key_event(KeyCode::Char('a'), KeyModifiers::NONE);
        assert_eq!(KeyMapper::map(&event, &modes), Some(b"a".to_vec()));

        // Ctrl+C
        let event = key_event(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(KeyMapper::map(&event, &modes), Some(vec![0x03]));

        // Alt+x
        let event = key_event(KeyCode::Char('x'), KeyModifiers::ALT);
        assert_eq!(KeyMapper::map(&event, &modes), Some(vec![0x1B, b'x']));
    }

    #[test]
    fn test_arrow_keys() {
        let modes = TerminalModes::default();

        // Normal mode
        let event = key_event(KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(KeyMapper::map(&event, &modes), Some(b"\x1b[A".to_vec()));

        // With Ctrl
        let event = key_event(KeyCode::Up, KeyModifiers::CONTROL);
        assert_eq!(KeyMapper::map(&event, &modes), Some(b"\x1b[1;5A".to_vec()));
    }

    #[test]
    fn test_function_keys() {
        let modes = TerminalModes::default();

        let event = key_event(KeyCode::F(1), KeyModifiers::NONE);
        assert_eq!(KeyMapper::map(&event, &modes), Some(b"\x1bOP".to_vec()));

        let event = key_event(KeyCode::F(5), KeyModifiers::NONE);
        assert_eq!(KeyMapper::map(&event, &modes), Some(b"\x1b[15~".to_vec()));
    }
    
    #[test]
    fn test_mouse_encoding_x10() {
        // X10 mode: \x1b[MCbCxCy (cb + 32, x + 32, y + 32)
        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        
        // Button 0 (left) at (0,0) -> cb=0+32=32, x=1+32=33, y=1+32=33
        assert_eq!(
            KeyMapper::encode_mouse_event(&event, false, false),
            vec![0x1b, b'[', b'M', 32, 33, 33]
        );
        
        // Right click at (10, 5)
        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Right),
            column: 10,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        // Button 2 (right) at (10,5) -> cb=2+32=34, x=11+32=43, y=6+32=38
        assert_eq!(
            KeyMapper::encode_mouse_event(&event, false, false),
            vec![0x1b, b'[', b'M', 34, 43, 38]
        );
    }
    
    #[test]
    fn test_mouse_encoding_sgr() {
        // SGR mode: \x1b[<Cb;Cx;CyM or m
        let event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        };
        
        assert_eq!(
            KeyMapper::encode_mouse_event(&event, true, false),
            b"\x1b[<0;1;1M".to_vec()
        );
        
        // Mouse up (release)
        let event = MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: 10,
            row: 20,
            modifiers: KeyModifiers::NONE,
        };
        
        assert_eq!(
            KeyMapper::encode_mouse_event(&event, true, false),
            b"\x1b[<0;11;21m".to_vec()
        );
    }
    
    #[test]
    fn test_mouse_encoding_scroll() {
        // Scroll up = button code 64
        let event = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        
        assert_eq!(
            KeyMapper::encode_mouse_event(&event, true, false),
            b"\x1b[<64;6;6M".to_vec()
        );
        
        // Scroll down = button code 65
        let event = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 5,
            row: 5,
            modifiers: KeyModifiers::NONE,
        };
        
        assert_eq!(
            KeyMapper::encode_mouse_event(&event, true, false),
            b"\x1b[<65;6;6M".to_vec()
        );
    }
}
