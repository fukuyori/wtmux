//! Input event wrapper.
//!
//! On Windows, crossterm's legacy console input path can drop non-BMP IME
//! characters when surrogate key-up events interleave with key-down events.
//! This reader keeps the rest of the app on crossterm event types while
//! handling UTF-16 surrogate pairs locally.

#[cfg(not(windows))]
pub fn poll(timeout: std::time::Duration) -> std::io::Result<bool> {
    crossterm::event::poll(timeout)
}

#[cfg(not(windows))]
pub fn read() -> std::io::Result<crossterm::event::Event> {
    crossterm::event::read()
}

#[cfg(windows)]
mod windows_input {
    use std::cell::RefCell;
    use std::io;
    use std::time::{Duration, Instant};

    use crossterm::event::{
        Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent,
        MouseEventKind,
    };
    use windows::Win32::Foundation::{WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows::Win32::System::Console::{
        GetConsoleScreenBufferInfo, GetNumberOfConsoleInputEvents, GetStdHandle,
        ReadConsoleInputW, COORD, DOUBLE_CLICK, FOCUS_EVENT, FROM_LEFT_1ST_BUTTON_PRESSED,
        FROM_LEFT_2ND_BUTTON_PRESSED, INPUT_RECORD, KEY_EVENT, KEY_EVENT_RECORD,
        LEFT_ALT_PRESSED, LEFT_CTRL_PRESSED, MOUSE_EVENT, MOUSE_HWHEELED, MOUSE_MOVED,
        MOUSE_WHEELED, RIGHTMOST_BUTTON_PRESSED, RIGHT_ALT_PRESSED, RIGHT_CTRL_PRESSED,
        SHIFT_PRESSED, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, WINDOW_BUFFER_SIZE_EVENT,
    };
    use windows::Win32::System::Threading::WaitForSingleObject;
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        VK_BACK, VK_CONTROL, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F24, VK_HOME,
        VK_INSERT, VK_LEFT, VK_MENU, VK_NEXT, VK_NUMPAD0, VK_NUMPAD9, VK_PRIOR, VK_RETURN,
        VK_RIGHT, VK_SHIFT, VK_TAB, VK_UP,
    };

    thread_local! {
        static READER: RefCell<Option<WindowsInputReader>> = RefCell::new(None);
    }

    pub fn poll(timeout: Duration) -> io::Result<bool> {
        with_reader(|reader| reader.poll(timeout))
    }

    pub fn read() -> io::Result<Event> {
        with_reader(|reader| reader.read())
    }

    fn with_reader<T>(f: impl FnOnce(&mut WindowsInputReader) -> io::Result<T>) -> io::Result<T> {
        READER.with(|cell| {
            if cell.borrow().is_none() {
                *cell.borrow_mut() = Some(WindowsInputReader::new()?);
            }

            let mut reader = cell.borrow_mut();
            f(reader.as_mut().expect("reader initialized"))
        })
    }

    struct WindowsInputReader {
        input: windows::Win32::Foundation::HANDLE,
        surrogate: Option<u16>,
        mouse_left: bool,
        mouse_right: bool,
        mouse_middle: bool,
    }

    impl WindowsInputReader {
        fn new() -> io::Result<Self> {
            let input = unsafe { GetStdHandle(STD_INPUT_HANDLE) }
                .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;

            Ok(Self {
                input,
                surrogate: None,
                mouse_left: false,
                mouse_right: false,
                mouse_middle: false,
            })
        }

        fn poll(&self, timeout: Duration) -> io::Result<bool> {
            let deadline = Instant::now() + timeout;

            loop {
                if self.available_events()? > 0 {
                    return Ok(true);
                }

                let remaining = deadline.saturating_duration_since(Instant::now());
                let wait_ms = remaining.as_millis().min(10) as u32;
                let wait_result = unsafe { WaitForSingleObject(self.input, wait_ms) };

                if wait_result == WAIT_OBJECT_0 && self.available_events()? > 0 {
                    return Ok(true);
                }
                if wait_result == WAIT_TIMEOUT && Instant::now() >= deadline {
                    return Ok(false);
                }
                if Instant::now() >= deadline {
                    return Ok(false);
                }
            }
        }

        fn read(&mut self) -> io::Result<Event> {
            loop {
                let mut records = [INPUT_RECORD::default(); 1];
                let mut read = 0;
                unsafe {
                    ReadConsoleInputW(self.input, &mut records, &mut read)
                        .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
                }

                if read == 0 {
                    continue;
                }

                if let Some(event) = self.parse_record(records[0]) {
                    return Ok(event);
                }
            }
        }

        fn available_events(&self) -> io::Result<u32> {
            let mut count = 0;
            unsafe {
                GetNumberOfConsoleInputEvents(self.input, &mut count)
                    .map_err(|e| io::Error::from_raw_os_error(e.code().0))?;
            }
            Ok(count)
        }

        fn parse_record(&mut self, record: INPUT_RECORD) -> Option<Event> {
            match record.EventType as u32 {
                KEY_EVENT => unsafe { self.parse_key(record.Event.KeyEvent) },
                MOUSE_EVENT => unsafe { self.parse_mouse(record.Event.MouseEvent) },
                WINDOW_BUFFER_SIZE_EVENT => {
                    let size = unsafe { record.Event.WindowBufferSizeEvent.dwSize };
                    Some(resize_event_from_buffer_size(size))
                }
                FOCUS_EVENT => {
                    let focus = unsafe { record.Event.FocusEvent.bSetFocus };
                    Some(if focus.as_bool() {
                        Event::FocusGained
                    } else {
                        Event::FocusLost
                    })
                }
                _ => None,
            }
        }

        fn parse_key(&mut self, key: KEY_EVENT_RECORD) -> Option<Event> {
            let key_down = key.bKeyDown.as_bool();
            let modifiers = key_modifiers(key.dwControlKeyState);
            let virtual_key = key.wVirtualKeyCode;
            let unicode = unsafe { key.uChar.UnicodeChar };

            if (0xD800..=0xDFFF).contains(&unicode) {
                let ch = self.handle_surrogate(unicode, key_down)?;
                return Some(Event::Key(KeyEvent::new(KeyCode::Char(ch), modifiers)));
            }

            if key_down {
                self.surrogate = None;
            }

            let code = match virtual_key {
                v if v == VK_SHIFT.0 || v == VK_CONTROL.0 || v == VK_MENU.0 => None,
                v if v == VK_BACK.0 => Some(KeyCode::Backspace),
                v if v == VK_ESCAPE.0 => Some(KeyCode::Esc),
                v if v == VK_RETURN.0 => Some(KeyCode::Enter),
                v if (VK_F1.0..=VK_F24.0).contains(&v) => {
                    Some(KeyCode::F((v - VK_F1.0 + 1) as u8))
                }
                v if v == VK_LEFT.0 => Some(KeyCode::Left),
                v if v == VK_UP.0 => Some(KeyCode::Up),
                v if v == VK_RIGHT.0 => Some(KeyCode::Right),
                v if v == VK_DOWN.0 => Some(KeyCode::Down),
                v if v == VK_PRIOR.0 => Some(KeyCode::PageUp),
                v if v == VK_NEXT.0 => Some(KeyCode::PageDown),
                v if v == VK_HOME.0 => Some(KeyCode::Home),
                v if v == VK_END.0 => Some(KeyCode::End),
                v if v == VK_DELETE.0 => Some(KeyCode::Delete),
                v if v == VK_INSERT.0 => Some(KeyCode::Insert),
                v if v == VK_TAB.0 && modifiers.contains(KeyModifiers::SHIFT) => {
                    Some(KeyCode::BackTab)
                }
                v if v == VK_TAB.0 => Some(KeyCode::Tab),
                _ if unicode == 0 => char_for_control_key(virtual_key).map(KeyCode::Char),
                _ if unicode <= 0x1F => char_for_control_key(virtual_key).map(KeyCode::Char),
                _ => char::from_u32(unicode as u32).map(KeyCode::Char),
            }?;

            let kind = if key_down {
                KeyEventKind::Press
            } else {
                KeyEventKind::Release
            };
            Some(Event::Key(KeyEvent::new_with_kind(code, modifiers, kind)))
        }

        fn handle_surrogate(&mut self, surrogate: u16, key_down: bool) -> Option<char> {
            if !key_down {
                return None;
            }

            if (0xD800..=0xDBFF).contains(&surrogate) {
                self.surrogate = Some(surrogate);
                return None;
            }

            let high = self.surrogate.take()?;
            std::char::decode_utf16([high, surrogate])
                .next()
                .and_then(Result::ok)
        }

        fn parse_mouse(
            &mut self,
            mouse: windows::Win32::System::Console::MOUSE_EVENT_RECORD,
        ) -> Option<Event> {
            let modifiers = key_modifiers(mouse.dwControlKeyState);
            let column = mouse.dwMousePosition.X.max(0) as u16;
            let row = relative_mouse_row(mouse.dwMousePosition.Y);
            let button_state = mouse.dwButtonState;

            let kind = match mouse.dwEventFlags {
                0 | DOUBLE_CLICK => {
                    if button_state & FROM_LEFT_1ST_BUTTON_PRESSED != 0 && !self.mouse_left {
                        Some(MouseEventKind::Down(MouseButton::Left))
                    } else if button_state & FROM_LEFT_1ST_BUTTON_PRESSED == 0 && self.mouse_left {
                        Some(MouseEventKind::Up(MouseButton::Left))
                    } else if button_state & RIGHTMOST_BUTTON_PRESSED != 0 && !self.mouse_right {
                        Some(MouseEventKind::Down(MouseButton::Right))
                    } else if button_state & RIGHTMOST_BUTTON_PRESSED == 0 && self.mouse_right {
                        Some(MouseEventKind::Up(MouseButton::Right))
                    } else if button_state & FROM_LEFT_2ND_BUTTON_PRESSED != 0 && !self.mouse_middle
                    {
                        Some(MouseEventKind::Down(MouseButton::Middle))
                    } else if button_state & FROM_LEFT_2ND_BUTTON_PRESSED == 0 && self.mouse_middle
                    {
                        Some(MouseEventKind::Up(MouseButton::Middle))
                    } else {
                        None
                    }
                }
                MOUSE_MOVED => {
                    if button_state & RIGHTMOST_BUTTON_PRESSED != 0 {
                        Some(MouseEventKind::Drag(MouseButton::Right))
                    } else if button_state & FROM_LEFT_2ND_BUTTON_PRESSED != 0 {
                        Some(MouseEventKind::Drag(MouseButton::Middle))
                    } else if button_state & FROM_LEFT_1ST_BUTTON_PRESSED != 0 {
                        Some(MouseEventKind::Drag(MouseButton::Left))
                    } else {
                        Some(MouseEventKind::Moved)
                    }
                }
                MOUSE_WHEELED => {
                    if (button_state as i32) < 0 {
                        Some(MouseEventKind::ScrollDown)
                    } else {
                        Some(MouseEventKind::ScrollUp)
                    }
                }
                MOUSE_HWHEELED => {
                    if (button_state as i32) < 0 {
                        Some(MouseEventKind::ScrollLeft)
                    } else {
                        Some(MouseEventKind::ScrollRight)
                    }
                }
                _ => None,
            };

            self.mouse_left = button_state & FROM_LEFT_1ST_BUTTON_PRESSED != 0;
            self.mouse_right = button_state & RIGHTMOST_BUTTON_PRESSED != 0;
            self.mouse_middle = button_state & FROM_LEFT_2ND_BUTTON_PRESSED != 0;

            kind.map(|kind| Event::Mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers,
            }))
        }
    }

    fn key_modifiers(state: u32) -> KeyModifiers {
        let mut modifiers = KeyModifiers::empty();
        if state & SHIFT_PRESSED != 0 {
            modifiers |= KeyModifiers::SHIFT;
        }
        if state & (LEFT_CTRL_PRESSED | RIGHT_CTRL_PRESSED) != 0 {
            modifiers |= KeyModifiers::CONTROL;
        }
        if state & (LEFT_ALT_PRESSED | RIGHT_ALT_PRESSED) != 0 {
            modifiers |= KeyModifiers::ALT;
        }
        modifiers
    }

    fn char_for_control_key(virtual_key: u16) -> Option<char> {
        match virtual_key {
            v if (b'A' as u16..=b'Z' as u16).contains(&v) => {
                Some((v as u8 as char).to_ascii_lowercase())
            }
            v if (b'0' as u16..=b'9' as u16).contains(&v) => Some(v as u8 as char),
            v if (VK_NUMPAD0.0..=VK_NUMPAD9.0).contains(&v) => {
                Some((b'0' + (v - VK_NUMPAD0.0) as u8) as char)
            }
            _ => None,
        }
    }

    fn relative_mouse_row(buffer_row: i16) -> u16 {
        let output = unsafe { GetStdHandle(STD_OUTPUT_HANDLE) };
        if let Ok(output) = output {
            let mut info = Default::default();
            if unsafe { GetConsoleScreenBufferInfo(output, &mut info) }.is_ok() {
                return buffer_row.saturating_sub(info.srWindow.Top).max(0) as u16;
            }
        }
        buffer_row.max(0) as u16
    }

    fn resize_event_from_buffer_size(size: COORD) -> Event {
        Event::Resize(size.X.max(0) as u16, size.Y.max(0) as u16)
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use windows::Win32::Foundation::BOOL;
        use windows::Win32::System::Console::KEY_EVENT_RECORD_0;

        fn key_event(unicode: u16, key_down: bool) -> KEY_EVENT_RECORD {
            KEY_EVENT_RECORD {
                bKeyDown: BOOL(key_down as i32),
                wRepeatCount: 1,
                wVirtualKeyCode: 0,
                wVirtualScanCode: 0,
                uChar: KEY_EVENT_RECORD_0 {
                    UnicodeChar: unicode,
                },
                dwControlKeyState: 0,
            }
        }

        #[test]
        fn surrogate_key_up_events_do_not_clear_pending_pair() {
            let mut reader = WindowsInputReader {
                input: Default::default(),
                surrogate: None,
                mouse_left: false,
                mouse_right: false,
                mouse_middle: false,
            };

            assert!(reader.parse_key(key_event(0xD83D, true)).is_none());
            assert!(reader.parse_key(key_event(0xD83D, false)).is_none());

            let event = reader.parse_key(key_event(0xDEB6, true));
            assert_eq!(
                event,
                Some(Event::Key(KeyEvent::new(
                    KeyCode::Char('🚶'),
                    KeyModifiers::empty()
                )))
            );
        }

        #[test]
        fn window_buffer_size_event_uses_cell_count_without_extra_row() {
            assert_eq!(
                resize_event_from_buffer_size(COORD { X: 120, Y: 30 }),
                Event::Resize(120, 30)
            );
        }
    }
}

#[cfg(windows)]
pub use windows_input::{poll, read};
