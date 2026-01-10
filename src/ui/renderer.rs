//! Terminal renderer using crossterm
//!
//! Renders the terminal state to the console.

use std::io::{self, Write};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    execute,
    style::{
        Attribute, ResetColor, SetAttribute,
        SetBackgroundColor, SetForegroundColor,
    },
    terminal::{
        self, Clear, ClearType, DisableLineWrap, EnableLineWrap,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};

use crate::core::term::{AttrFlags, CellAttrs, TerminalState};

/// Debug log file (global for simplicity)
static DEBUG_LOG: std::sync::OnceLock<std::sync::Mutex<std::fs::File>> = std::sync::OnceLock::new();

fn debug_log(msg: &str) {
    if let Some(mutex) = DEBUG_LOG.get() {
        if let Ok(mut file) = mutex.lock() {
            let _ = writeln!(file, "{}", msg);
            let _ = file.flush();
        }
    }
}

fn init_debug_log() {
    let _ = DEBUG_LOG.get_or_init(|| {
        std::sync::Mutex::new(
            std::fs::File::create("rustterm_debug.log").expect("Failed to create debug log")
        )
    });
}

/// A cell for the render buffer (for diff rendering, experimental)
#[allow(dead_code)]
#[derive(Clone, PartialEq)]
struct RenderCell {
    ch: String,
    attrs: CellAttrs,
    selected: bool,
}

#[allow(dead_code)]
impl Default for RenderCell {
    fn default() -> Self {
        Self {
            ch: " ".to_string(),
            attrs: CellAttrs::default(),
            selected: false,
        }
    }
}

/// Terminal renderer
pub struct Renderer {
    /// Last rendered state hash (for optimization)
    last_cursor: (u16, u16),
    /// Whether the terminal has been initialized
    initialized: bool,
    /// Previous frame buffer for diff rendering (experimental)
    #[allow(dead_code)]
    prev_buffer: Vec<Vec<RenderCell>>,
    /// Current terminal size
    #[allow(dead_code)]
    size: (u16, u16),
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

impl Renderer {
    pub fn new() -> Self {
        Self {
            last_cursor: (0, 0),
            initialized: false,
            prev_buffer: Vec::new(),
            size: (0, 0),
        }
    }

    /// Initialize the terminal for rendering
    pub fn init(&mut self) -> io::Result<()> {
        init_debug_log();
        debug_log("=== RustTerm init ===");
        
        // Disable Windows console Quick Edit mode to receive mouse events
        #[cfg(windows)]
        {
            use windows::Win32::System::Console::{
                GetConsoleMode, SetConsoleMode, GetStdHandle,
                CONSOLE_MODE, ENABLE_QUICK_EDIT_MODE, ENABLE_EXTENDED_FLAGS,
                ENABLE_MOUSE_INPUT, ENABLE_WINDOW_INPUT,
                STD_INPUT_HANDLE,
            };
            
            unsafe {
                let handle = GetStdHandle(STD_INPUT_HANDLE).unwrap_or_default();
                debug_log(&format!("STD_INPUT_HANDLE: {:?}", handle));
                
                let mut mode = CONSOLE_MODE(0);
                if GetConsoleMode(handle, &mut mode).is_ok() {
                    debug_log(&format!("Original console mode: 0x{:08X}", mode.0));
                    
                    // Disable Quick Edit mode, enable mouse input
                    let new_mode = CONSOLE_MODE(
                        (mode.0 & !ENABLE_QUICK_EDIT_MODE.0) 
                        | ENABLE_EXTENDED_FLAGS.0
                        | ENABLE_MOUSE_INPUT.0
                        | ENABLE_WINDOW_INPUT.0
                    );
                    debug_log(&format!("New console mode: 0x{:08X}", new_mode.0));
                    
                    match SetConsoleMode(handle, new_mode) {
                        Ok(_) => debug_log("SetConsoleMode succeeded"),
                        Err(e) => debug_log(&format!("SetConsoleMode failed: {:?}", e)),
                    }
                } else {
                    debug_log("GetConsoleMode failed");
                }
            }
        }
        
        debug_log("Calling enable_raw_mode...");
        terminal::enable_raw_mode()?;
        debug_log("enable_raw_mode succeeded");
        
        let mut stdout = io::stdout();
        
        // Enable mouse capture
        debug_log("Enabling mouse capture...");
        execute!(
            stdout,
            EnterAlternateScreen,
            crossterm::event::EnableMouseCapture,
            DisableLineWrap,
            Clear(ClearType::All),
            MoveTo(0, 0)
        )?;
        
        // Enable SGR extended mouse mode for better compatibility
        write!(stdout, "\x1b[?1000h")?; // Enable mouse click tracking
        write!(stdout, "\x1b[?1002h")?; // Enable mouse drag tracking  
        write!(stdout, "\x1b[?1006h")?; // Enable SGR extended mouse mode
        
        // Enable synchronized output mode (reduces flicker)
        write!(stdout, "\x1b[?2026h")?;
        
        stdout.flush()?;
        self.initialized = true;
        
        // Verify console mode after crossterm init
        #[cfg(windows)]
        {
            use windows::Win32::System::Console::{
                GetConsoleMode, GetStdHandle,
                CONSOLE_MODE, STD_INPUT_HANDLE,
            };
            
            unsafe {
                let handle = GetStdHandle(STD_INPUT_HANDLE).unwrap_or_default();
                let mut mode = CONSOLE_MODE(0);
                if GetConsoleMode(handle, &mut mode).is_ok() {
                    debug_log(&format!("Console mode after init: 0x{:08X}", mode.0));
                    debug_log(&format!("  ENABLE_MOUSE_INPUT (0x10): {}", (mode.0 & 0x10) != 0));
                    debug_log(&format!("  ENABLE_QUICK_EDIT_MODE (0x40): {}", (mode.0 & 0x40) != 0));
                    debug_log(&format!("  ENABLE_EXTENDED_FLAGS (0x80): {}", (mode.0 & 0x80) != 0));
                    debug_log(&format!("  ENABLE_WINDOW_INPUT (0x08): {}", (mode.0 & 0x08) != 0));
                    debug_log(&format!("  ENABLE_VIRTUAL_TERMINAL_INPUT (0x200): {}", (mode.0 & 0x200) != 0));
                }
            }
        }
        
        debug_log("init completed successfully");
        Ok(())
    }

    /// Cleanup the terminal
    pub fn cleanup(&mut self) -> io::Result<()> {
        if !self.initialized {
            return Ok(());
        }
        self.initialized = false;
        
        let mut stdout = io::stdout();
        
        // Disable mouse modes
        write!(stdout, "\x1b[?1006l")?; // Disable SGR extended mouse mode
        write!(stdout, "\x1b[?1002l")?; // Disable mouse drag tracking
        write!(stdout, "\x1b[?1000l")?; // Disable mouse click tracking
        
        // Reset all attributes first
        let _ = execute!(stdout, ResetColor, SetAttribute(Attribute::Reset));
        
        // Show cursor
        let _ = execute!(stdout, Show);
        
        // Enable line wrap
        let _ = execute!(stdout, EnableLineWrap);
        
        // Disable mouse capture
        let _ = execute!(stdout, crossterm::event::DisableMouseCapture);
        
        // Leave alternate screen
        let _ = execute!(stdout, LeaveAlternateScreen);
        
        // Flush output
        let _ = stdout.flush();
        
        // Disable raw mode - this is the most important part
        terminal::disable_raw_mode()?;
        
        // Print a newline to ensure we're on a fresh line
        println!();
        
        Ok(())
    }

    /// Log mouse event
    pub fn log_mouse_event(&self, msg: &str) {
        debug_log(msg);
    }

    /// Ensure buffer is properly sized (for diff rendering)
    #[allow(dead_code)]
    fn ensure_buffer_size(&mut self, cols: u16, rows: u16) {
        if self.size != (cols, rows) {
            self.prev_buffer = vec![vec![RenderCell::default(); cols as usize]; rows as usize];
            self.size = (cols, rows);
        }
    }

    /// Clear the render buffer (forces full redraw on next render)
    #[allow(dead_code)]
    pub fn clear_buffer(&mut self) {
        for row in &mut self.prev_buffer {
            for cell in row {
                *cell = RenderCell {
                    ch: "\x00".to_string(), // Use null to force redraw
                    attrs: CellAttrs::default(),
                    selected: false,
                };
            }
        }
    }

    /// Render the terminal state
    pub fn render(&mut self, state: &TerminalState) -> io::Result<()> {
        let screen = state.active_screen();
        let cursor = state.active_cursor();

        // Use a buffered writer for better performance
        let stdout = io::stdout();
        let mut stdout = io::BufWriter::with_capacity(65536, stdout.lock());

        // Begin synchronized update (reduces flicker)
        write!(stdout, "\x1b[?2026h")?;
        execute!(stdout, Hide)?;

        // Use line-based rendering (more reliable for wide characters)
        if screen.full_redraw {
            self.render_full(&mut stdout, state)?;
        } else if !screen.dirty_lines.is_empty() {
            self.render_dirty(&mut stdout, state)?;
        }

        // Update cursor position
        if cursor.visible {
            execute!(
                stdout,
                MoveTo(cursor.col, cursor.row),
                Show
            )?;
        }

        // End synchronized update
        write!(stdout, "\x1b[?2026l")?;

        stdout.flush()?;
        self.last_cursor = (cursor.col, cursor.row);

        Ok(())
    }

    /// Diff-based rendering - only update changed cells (experimental)
    #[allow(dead_code)]
    fn render_diff<W: Write>(&mut self, stdout: &mut W, state: &TerminalState) -> io::Result<()> {
        let screen = state.active_screen();
        let num_rows = state.rows as usize;
        let num_cols = state.cols as usize;
        let has_selection = state.selection.is_some();

        let mut last_attrs = CellAttrs::default();
        let mut last_selected = false;
        let mut last_pos: Option<(u16, u16)> = None;

        for row_idx in 0..num_rows {
            let row = match screen.get_row_at(row_idx) {
                Some(r) => r,
                None => continue,
            };

            let mut col_idx: u16 = 0;
            for cell in &row.cells {
                if col_idx >= num_cols as u16 {
                    break;
                }

                // Skip continuation cells - they are handled with their parent
                if cell.is_continuation() {
                    col_idx += 1;
                    continue;
                }

                let cell_width = cell.width.max(1) as u16;
                
                // Skip if this cell would overflow the screen width
                if col_idx + cell_width > num_cols as u16 {
                    break;
                }

                let is_selected = has_selection && state.is_selected(col_idx, row_idx as u16);
                let ch = cell.display_char().to_string();

                // Build current cell
                let current = RenderCell {
                    ch: ch.clone(),
                    attrs: cell.attrs.clone(),
                    selected: is_selected,
                };

                // Check if cell changed
                let prev = &self.prev_buffer[row_idx][col_idx as usize];
                if *prev != current {
                    // Cell changed, need to update

                    // Move cursor if not consecutive
                    let need_move = match last_pos {
                        Some((lc, lr)) => !(lr == row_idx as u16 && lc == col_idx),
                        None => true,
                    };
                    if need_move {
                        execute!(stdout, MoveTo(col_idx, row_idx as u16))?;
                    }

                    // Apply attributes if changed
                    if cell.attrs != last_attrs || is_selected != last_selected {
                        self.apply_attrs(stdout, &cell.attrs, is_selected)?;
                        last_attrs = cell.attrs.clone();
                        last_selected = is_selected;
                    }

                    // Write character
                    write!(stdout, "{}", ch)?;
                    
                    // Update last position (cursor moves by display width)
                    last_pos = Some((col_idx + cell_width, row_idx as u16));

                    // Update buffer for this cell
                    self.prev_buffer[row_idx][col_idx as usize] = current;
                    
                    // For wide characters, also mark continuation cells in buffer
                    for w in 1..cell_width {
                        let cont_col = col_idx + w;
                        if (cont_col as usize) < num_cols {
                            self.prev_buffer[row_idx][cont_col as usize] = RenderCell {
                                ch: String::new(), // continuation marker
                                attrs: cell.attrs.clone(),
                                selected: is_selected,
                            };
                        }
                    }
                }

                col_idx += cell_width;
            }

            // Clear remaining cells in the row if needed
            while (col_idx as usize) < num_cols {
                let current = RenderCell::default();
                let prev = &self.prev_buffer[row_idx][col_idx as usize];
                if *prev != current {
                    let need_move = match last_pos {
                        Some((lc, lr)) => !(lr == row_idx as u16 && lc == col_idx),
                        None => true,
                    };
                    if need_move {
                        execute!(stdout, MoveTo(col_idx, row_idx as u16))?;
                    }
                    
                    if last_attrs != CellAttrs::default() || last_selected {
                        self.apply_attrs(stdout, &CellAttrs::default(), false)?;
                        last_attrs = CellAttrs::default();
                        last_selected = false;
                    }
                    
                    write!(stdout, " ")?;
                    last_pos = Some((col_idx + 1, row_idx as u16));
                    self.prev_buffer[row_idx][col_idx as usize] = current;
                }
                col_idx += 1;
            }
        }

        // Show scroll indicator if scrolled
        if screen.is_scrolled() {
            execute!(stdout, MoveTo(0, 0))?;
            self.apply_attrs(stdout, &CellAttrs::default(), false)?;
            let indicator = format!("[↑ {} lines]", screen.scroll_offset);
            write!(stdout, "{}", indicator)?;
            // Mark these cells as changed in buffer
            for (i, ch) in indicator.chars().enumerate() {
                if i < num_cols {
                    self.prev_buffer[0][i] = RenderCell {
                        ch: ch.to_string(),
                        attrs: CellAttrs::default(),
                        selected: false,
                    };
                }
            }
        }

        execute!(stdout, ResetColor, SetAttribute(Attribute::Reset))?;

        Ok(())
    }

    /// Full screen render with optimizations
    fn render_full<W: Write>(&self, stdout: &mut W, state: &TerminalState) -> io::Result<()> {
        let screen = state.active_screen();
        let num_rows = state.rows as usize;
        let num_cols = state.cols as u16;
        let has_selection = state.selection.is_some();

        // Hide cursor during rendering
        execute!(stdout, Hide)?;

        let mut current_attrs = CellAttrs::default();
        let mut current_selected = false;
        let mut line_buffer = String::with_capacity(256);

        for row_idx in 0..num_rows {
            // Move to line start and clear line
            execute!(stdout, MoveTo(0, row_idx as u16))?;
            write!(stdout, "\x1b[K")?; // Clear to end of line
            line_buffer.clear();

            // Get row accounting for scroll offset
            let row = match screen.get_row_at(row_idx) {
                Some(r) => r,
                None => continue,
            };

            let mut col_idx: u16 = 0;
            for cell in &row.cells {
                if col_idx >= num_cols {
                    break;
                }
                
                // Skip continuation cells (placeholders for wide characters)
                if cell.is_continuation() {
                    col_idx += 1;
                    continue;
                }

                // Check if this cell is selected
                let is_selected = has_selection && state.is_selected(col_idx, row_idx as u16);
                
                // Check if we need to flush and change attributes
                let attrs_changed = cell.attrs != current_attrs || is_selected != current_selected;
                
                if attrs_changed && !line_buffer.is_empty() {
                    // Apply current attributes and flush buffer
                    self.apply_attrs(stdout, &current_attrs, current_selected)?;
                    write!(stdout, "{}", line_buffer)?;
                    line_buffer.clear();
                }
                
                if attrs_changed {
                    current_attrs = cell.attrs.clone();
                    current_selected = is_selected;
                }

                // Add character to buffer
                line_buffer.push_str(cell.display_char());
                
                // Advance column by actual cell width
                col_idx += cell.width.max(1) as u16;
            }

            // Flush remaining text for this line
            if !line_buffer.is_empty() {
                self.apply_attrs(stdout, &current_attrs, current_selected)?;
                write!(stdout, "{}", line_buffer)?;
                line_buffer.clear();
            }
        }

        // Show scroll indicator if scrolled
        if screen.is_scrolled() {
            execute!(stdout, MoveTo(0, 0))?;
            self.apply_attrs(stdout, &CellAttrs::default(), false)?;
            write!(stdout, "[↑ {} lines]", screen.scroll_offset)?;
        }

        // Reset attributes
        execute!(stdout, ResetColor, SetAttribute(Attribute::Reset))?;

        Ok(())
    }

    /// Render only dirty lines with optimizations
    fn render_dirty<W: Write>(&self, stdout: &mut W, state: &TerminalState) -> io::Result<()> {
        let screen = state.active_screen();
        let has_selection = state.selection.is_some();
        let num_cols = state.cols as u16;

        execute!(stdout, Hide)?;

        let mut current_attrs;
        let mut current_selected;
        let mut line_buffer = String::with_capacity(256);

        // Sort dirty lines for sequential access
        let mut dirty: Vec<_> = screen.dirty_lines.iter().copied().collect();
        dirty.sort_unstable();

        for row_idx in dirty {
            // Get row accounting for scroll offset
            let row = match screen.get_row_at(row_idx) {
                Some(r) => r,
                None => continue,
            };

            // Move to line start and clear to end of line
            execute!(stdout, MoveTo(0, row_idx as u16))?;
            write!(stdout, "\x1b[K")?; // Clear to end of line
            line_buffer.clear();
            current_attrs = CellAttrs::default();
            current_selected = false;

            let mut col_idx: u16 = 0;
            for cell in &row.cells {
                if col_idx >= num_cols {
                    break;
                }
                
                if cell.is_continuation() {
                    col_idx += 1;
                    continue;
                }

                let is_selected = has_selection && state.is_selected(col_idx, row_idx as u16);
                let attrs_changed = cell.attrs != current_attrs || is_selected != current_selected;
                
                if attrs_changed && !line_buffer.is_empty() {
                    self.apply_attrs(stdout, &current_attrs, current_selected)?;
                    write!(stdout, "{}", line_buffer)?;
                    line_buffer.clear();
                }
                
                if attrs_changed {
                    current_attrs = cell.attrs.clone();
                    current_selected = is_selected;
                }

                line_buffer.push_str(cell.display_char());
                col_idx += cell.width.max(1) as u16;
            }

            if !line_buffer.is_empty() {
                self.apply_attrs(stdout, &current_attrs, current_selected)?;
                write!(stdout, "{}", line_buffer)?;
                line_buffer.clear();
            }
        }

        execute!(stdout, ResetColor, SetAttribute(Attribute::Reset))?;

        Ok(())
    }

    /// Apply cell attributes
    fn apply_attrs<W: Write>(&self, stdout: &mut W, attrs: &CellAttrs, is_selected: bool) -> io::Result<()> {
        // Reset first
        execute!(stdout, SetAttribute(Attribute::Reset))?;

        // Apply style attributes
        if attrs.flags.contains(AttrFlags::BOLD) {
            execute!(stdout, SetAttribute(Attribute::Bold))?;
        }
        if attrs.flags.contains(AttrFlags::ITALIC) {
            execute!(stdout, SetAttribute(Attribute::Italic))?;
        }
        if attrs.flags.contains(AttrFlags::UNDERLINE) {
            execute!(stdout, SetAttribute(Attribute::Underlined))?;
        }
        if attrs.flags.contains(AttrFlags::BLINK) {
            execute!(stdout, SetAttribute(Attribute::SlowBlink))?;
        }
        // Apply INVERSE if cell has it OR if selected
        if attrs.flags.contains(AttrFlags::INVERSE) != is_selected {
            // XOR: show reverse if either is true but not both
            execute!(stdout, SetAttribute(Attribute::Reverse))?;
        }
        if attrs.flags.contains(AttrFlags::STRIKETHROUGH) {
            execute!(stdout, SetAttribute(Attribute::CrossedOut))?;
        }

        // Apply colors
        let fg_color = attrs.fg.to_crossterm(true);
        if fg_color != crossterm::style::Color::Reset {
            execute!(stdout, SetForegroundColor(fg_color))?;
        }
        let bg_color = attrs.bg.to_crossterm(false);
        if bg_color != crossterm::style::Color::Reset {
            execute!(stdout, SetBackgroundColor(bg_color))?;
        }

        Ok(())
    }

    /// Get terminal size
    pub fn size() -> io::Result<(u16, u16)> {
        terminal::size()
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

/// Simple debug renderer that outputs to a string
#[allow(dead_code)]
pub struct DebugRenderer;

#[allow(dead_code)]
impl DebugRenderer {
    /// Render state to string (for debugging)
    pub fn render(state: &TerminalState) -> String {
        let screen = state.active_screen();
        let cursor = state.active_cursor();
        let mut output = String::new();

        output.push_str(&format!(
            "=== Terminal {}x{} ===\n",
            state.cols, state.rows
        ));
        output.push_str(&format!(
            "Cursor: ({}, {}) visible={}\n",
            cursor.col, cursor.row, cursor.visible
        ));
        output.push_str(&format!("Title: {}\n", state.title));
        output.push_str(&format!("Alternate: {}\n", state.using_alternate));
        output.push_str("─".repeat(state.cols as usize).as_str());
        output.push('\n');

        for (row_idx, row) in screen.rows.iter().enumerate() {
            // Row indicator
            let indicator = if row_idx == cursor.row as usize {
                '>'
            } else {
                ' '
            };
            output.push(indicator);

            for (col_idx, cell) in row.cells.iter().enumerate() {
                if cell.is_continuation() {
                    continue;
                }

                // Cursor indicator
                let ch = if row_idx == cursor.row as usize && col_idx == cursor.col as usize {
                    if cursor.visible {
                        '█'
                    } else {
                        cell.display_char().chars().next().unwrap_or(' ')
                    }
                } else {
                    cell.display_char().chars().next().unwrap_or(' ')
                };

                output.push(ch);
            }

            output.push('\n');
        }

        output.push_str("─".repeat(state.cols as usize).as_str());
        output.push('\n');

        output
    }
}
