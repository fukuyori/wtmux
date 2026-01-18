//! Multi-pane renderer for the window manager.
//!
//! This module handles all visual rendering for wtmux, including:
//! - Tab bar with clickable tabs
//! - Pane borders and content
//! - Status bar with session information
//! - Context menus, theme selectors, and other overlays
//!
//! # Rendering Architecture
//!
//! The renderer uses synchronized updates to prevent screen tearing:
//!
//! ```text
//! begin_frame()  → Hide cursor, disable autowrap, start sync
//!     ↓
//! render content → Tab bar, panes, status bar
//!     ↓
//! end_frame()    → Show cursor, enable autowrap, end sync, flush
//! ```
//!
//! # Performance Optimizations
//!
//! - Generation-based dirty tracking to minimize redraws
//! - Partial updates for cursor movement and selection
//! - Separate overlay rendering for context menus (avoids full redraw)

use std::io::{self, Write};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    execute,
    style::{
        Attribute, Color as CtColor, ResetColor, SetAttribute,
        SetBackgroundColor, SetForegroundColor,
    },
    terminal::{self, Clear, ClearType},
};
use unicode_width::UnicodeWidthChar;

use crate::wm::{WindowManager, Pane, BorderStyle};
use crate::core::term::{AttrFlags, CellAttrs, Color};
use crate::config::ColorScheme;
use crate::copymode::CopyMode;
use super::context_menu::ContextMenu;

/// Begin a render frame (synchronized update, hide cursor, disable autowrap)
fn begin_frame<W: Write>(out: &mut W) -> io::Result<()> {
    write!(out, "\x1b[?2026h")?;  // Begin synchronized update
    write!(out, "\x1b[?7l")?;      // Disable autowrap
    execute!(out, Hide)?;
    Ok(())
}

/// End a render frame (show cursor, enable autowrap, end synchronized update, flush)
fn end_frame<W: Write>(out: &mut W) -> io::Result<()> {
    execute!(out, Show)?;          // Show cursor
    write!(out, "\x1b[?7h")?;      // Enable autowrap
    write!(out, "\x1b[?2026l")?;   // End synchronized update
    out.flush()?;
    Ok(())
}

/// Execute a render operation with frame guards, ensuring cleanup on error
fn with_frame<W: Write, F, R>(out: &mut W, f: F) -> io::Result<R>
where
    F: FnOnce(&mut W) -> io::Result<R>,
{
    begin_frame(out)?;
    let result = f(out);
    // Always end frame, even on error
    let _ = end_frame(out);
    result
}

/// Execute an operation with cursor hidden, ensuring Show on exit
fn with_cursor_hidden<W: Write, F, R>(out: &mut W, f: F) -> io::Result<R>
where
    F: FnOnce(&mut W) -> io::Result<R>,
{
    execute!(out, Hide)?;
    let result = f(out);
    let _ = execute!(out, Show);
    let _ = out.flush();
    result
}

/// Border characters
#[allow(dead_code)]
struct BorderChars {
    top_left: char,
    top_right: char,
    bottom_left: char,
    bottom_right: char,
    horizontal: char,
    vertical: char,
    t_down: char,
    t_up: char,
    t_left: char,
    t_right: char,
    cross: char,
}

impl BorderChars {
    fn single() -> Self {
        Self {
            top_left: '┌',
            top_right: '┐',
            bottom_left: '└',
            bottom_right: '┘',
            horizontal: '─',
            vertical: '│',
            t_down: '┬',
            t_up: '┴',
            t_left: '┤',
            t_right: '├',
            cross: '┼',
        }
    }
}

/// Multi-pane renderer
pub struct WmRenderer {
    initialized: bool,
    pub color_scheme: ColorScheme,
    /// Last rendered layout generation (for detecting changes)
    last_generation: u64,
}

impl WmRenderer {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            initialized: false,
            color_scheme: ColorScheme::default(),
            last_generation: 0,
        }
    }

    pub fn with_color_scheme(color_scheme: ColorScheme) -> Self {
        Self {
            initialized: false,
            color_scheme,
            last_generation: 0,
        }
    }

    /// Set color scheme
    pub fn set_color_scheme(&mut self, scheme: ColorScheme) {
        self.color_scheme = scheme;
    }

    /// Initialize the terminal
    pub fn init(&mut self) -> io::Result<()> {
        terminal::enable_raw_mode()?;
        
        let mut stdout = io::stdout();
        execute!(
            stdout,
            crossterm::terminal::EnterAlternateScreen,
            crossterm::event::EnableMouseCapture,
            Clear(ClearType::All)
        )?;
        stdout.flush()?;
        
        self.initialized = true;
        Ok(())
    }

    /// Cleanup
    pub fn cleanup(&mut self) -> io::Result<()> {
        if !self.initialized {
            return Ok(());
        }
        
        let mut stdout = io::stdout();
        
        // Restore terminal state (in case of abnormal exit)
        write!(stdout, "\x1b[?7h")?;      // Enable autowrap
        write!(stdout, "\x1b[?2026l")?;   // End synchronized update (if active)
        stdout.flush()?;
        
        execute!(
            stdout,
            Show,
            crossterm::event::DisableMouseCapture,
            crossterm::terminal::LeaveAlternateScreen
        )?;
        terminal::disable_raw_mode()?;
        self.initialized = false;
        Ok(())
    }

    /// Get terminal size
    #[allow(dead_code)]
    pub fn size() -> io::Result<(u16, u16)> {
        terminal::size()
    }

    /// Render the window manager state
    pub fn render(&mut self, wm: &WindowManager) -> io::Result<()> {
        self.render_with_selector(wm, None)
    }

    /// Render with optional snippet selector
    pub fn render_with_selector(&mut self, wm: &WindowManager, selector: Option<&crate::history::HistorySelector>) -> io::Result<()> {
        let stdout = io::stdout();
        let mut out = io::BufWriter::with_capacity(65536, stdout.lock());

        with_frame(&mut out, |out| {
            self.render_tab_bar(out, wm)?;
            self.render_panes(out, wm)?;
            self.render_status_bar(out, wm)?;

            if let Some(sel) = selector {
                if sel.visible {
                    self.render_selector(out, wm, sel)?;
                }
            }

            // Show cursor at focused pane's cursor position (unless snippet selector is visible)
            let snippet_visible = selector.map(|s| s.visible).unwrap_or(false);
            if !snippet_visible {
                if let Some(tab) = wm.active_tab() {
                    if let Some(pane) = tab.focused_pane() {
                        let cursor = pane.session.state.active_cursor();
                        let (inner_x, inner_y) = pane.inner_pos();
                        if cursor.visible {
                            let shape_code = cursor.shape.to_decscusr();
                            write!(out, "\x1b[{} q", shape_code)?;
                            execute!(
                                out,
                                MoveTo(inner_x + cursor.col, wm.tab_bar_height + inner_y + cursor.row)
                            )?;
                        }
                    }
                }
            }
            Ok(())
        })
    }

    /// Render with pane numbers overlay
    pub fn render_with_pane_numbers(&mut self, wm: &WindowManager) -> io::Result<()> {
        let stdout = io::stdout();
        let mut out = stdout.lock();

        if !self.initialized {
            self.init()?;
        }

        with_frame(&mut out, |out| {
            self.render_tab_bar(out, wm)?;
            self.render_panes(out, wm)?;
            self.render_status_bar(out, wm)?;
            self.render_pane_numbers(out, wm)?;
            Ok(())
        })
    }

    /// Render pane numbers overlay
    fn render_pane_numbers<W: Write>(&self, stdout: &mut W, wm: &WindowManager) -> io::Result<()> {
        let cs = &self.color_scheme;
        
        let pane_info = wm.get_pane_numbers();
        
        for (idx, (_pane_id, x, y, width, height)) in pane_info.iter().enumerate() {
            // Calculate center of pane
            let center_x = x + width / 2;
            let center_y = wm.tab_bar_height + y + height / 2;
            
            // Draw number
            execute!(stdout, MoveTo(center_x.saturating_sub(1), center_y))?;
            execute!(stdout, 
                SetBackgroundColor(cs.selector_selected_bg.to_crossterm()), 
                SetForegroundColor(cs.selector_selected_fg.to_crossterm())
            )?;
            write!(stdout, " {} ", idx)?;
            execute!(stdout, ResetColor)?;
        }

        Ok(())
    }

    /// Render with copy mode overlay
    pub fn render_with_copy_mode(&mut self, wm: &WindowManager, copy_mode: &CopyMode) -> io::Result<()> {
        let stdout = io::stdout();
        let mut out = stdout.lock();

        if !self.initialized {
            self.init()?;
        }

        with_frame(&mut out, |out| {
            self.render_pane_with_copy_mode(out, wm, copy_mode)?;
            self.render_copy_mode_status(out, wm, copy_mode)?;

            // Position cursor at copy mode location
            if let Some(tab) = wm.active_tab() {
                if let Some(pane) = tab.focused_pane() {
                    if let Some(visible_row) = copy_mode.absolute_to_visible(copy_mode.cursor_row, wm) {
                        let (inner_x, inner_y) = pane.inner_pos();
                        let cursor_x = inner_x + copy_mode.cursor_col.min(pane.session.state.cols.saturating_sub(1));
                        execute!(out, MoveTo(cursor_x, wm.tab_bar_height + inner_y + visible_row))?;
                    }
                }
            }
            Ok(())
        })
    }

    /// Fast update for copy mode - only update cursor and status
    pub fn render_copy_mode_cursor_only(&mut self, wm: &WindowManager, copy_mode: &CopyMode) -> io::Result<()> {
        let stdout = io::stdout();
        let mut out = stdout.lock();

        with_cursor_hidden(&mut out, |out| {
            self.render_copy_mode_status(out, wm, copy_mode)?;

            // Position cursor at copy mode location
            if let Some(tab) = wm.active_tab() {
                if let Some(pane) = tab.focused_pane() {
                    if let Some(visible_row) = copy_mode.absolute_to_visible(copy_mode.cursor_row, wm) {
                        let (inner_x, inner_y) = pane.inner_pos();
                        let cursor_x = inner_x + copy_mode.cursor_col.min(pane.session.state.cols.saturating_sub(1));
                        execute!(out, MoveTo(cursor_x, wm.tab_bar_height + inner_y + visible_row))?;
                    }
                }
            }
            Ok(())
        })
    }

    /// Render pane content in copy mode with selection/search highlighting
    fn render_pane_with_copy_mode<W: Write>(&self, stdout: &mut W, wm: &WindowManager, copy_mode: &CopyMode) -> io::Result<()> {
        let cs = &self.color_scheme;
        let tab = match wm.active_tab() {
            Some(t) => t,
            None => return Ok(()),
        };

        // For now, render focused pane only in copy mode
        let pane = match tab.focused_pane() {
            Some(p) => p,
            None => return Ok(()),
        };

        let y_offset = wm.tab_bar_height;
        let (inner_x, inner_y) = pane.inner_pos();
        let (inner_w, inner_h) = pane.inner_size();
        
        let screen = pane.session.state.active_screen();
        let total_lines = screen.total_lines();
        let visible_rows = pane.session.state.rows as usize;
        
        // Calculate which rows to display based on copy mode scroll
        let bottom_row = total_lines.saturating_sub(1);
        let visible_start = bottom_row.saturating_sub(copy_mode.scroll_offset + visible_rows - 1);
        let render_w = inner_w as usize;

        for row_idx in 0..inner_h as usize {
            let abs_row = visible_start + row_idx;
            let screen_y = y_offset + inner_y + row_idx as u16;
            
            execute!(stdout, MoveTo(inner_x, screen_y))?;
            
            if let Some(line) = screen.get_line_at_absolute(abs_row) {
                let mut line_buffer = String::with_capacity(256);
                let mut last_style: Option<(bool, bool, bool)> = None; // (selected, current_match, search_match)
                
                for (cell_idx, cell) in line.iter().enumerate() {
                    if cell_idx >= render_w {
                        break;
                    }
                    
                    // Skip continuation cells
                    if cell.width == 0 {
                        continue;
                    }
                    
                    let cell_col = cell_idx as u16;
                    
                    // Check highlighting
                    let is_selected = copy_mode.is_selected(abs_row, cell_col);
                    let is_current_match = copy_mode.is_current_match(abs_row, cell_col);
                    let is_search_match = copy_mode.is_search_match(abs_row, cell_col);
                    
                    let current_style = (is_selected, is_current_match, is_search_match);
                    
                    // Check if style changed
                    if last_style != Some(current_style) {
                        // Flush buffer
                        if !line_buffer.is_empty() {
                            write!(stdout, "{}", line_buffer)?;
                            line_buffer.clear();
                        }
                        
                        // Apply new style
                        if is_current_match {
                            execute!(stdout, 
                                SetBackgroundColor(CtColor::Yellow),
                                SetForegroundColor(CtColor::Black)
                            )?;
                        } else if is_search_match {
                            execute!(stdout,
                                SetBackgroundColor(CtColor::DarkYellow),
                                SetForegroundColor(CtColor::Black)
                            )?;
                        } else if is_selected {
                            execute!(stdout,
                                SetBackgroundColor(cs.selection_bg.to_crossterm()),
                                SetForegroundColor(cs.selection_fg.to_crossterm())
                            )?;
                        } else {
                            self.apply_attrs_with_selection(stdout, &cell.attrs, false)?;
                        }
                        
                        last_style = Some(current_style);
                    }
                    
                    line_buffer.push_str(cell.display_char());
                }
                
                // Flush remaining
                if !line_buffer.is_empty() {
                    write!(stdout, "{}", line_buffer)?;
                }
                
                // Clear rest of line
                execute!(stdout, ResetColor)?;
            } else {
                // Empty line
                execute!(stdout, ResetColor)?;
                write!(stdout, "{:width$}", "", width = inner_w as usize)?;
            }
        }

        // Render border if needed
        if pane.border != BorderStyle::None {
            self.render_border(stdout, pane, y_offset)?;
        }

        execute!(stdout, ResetColor)?;
        Ok(())
    }

    /// Render copy mode status bar
    fn render_copy_mode_status<W: Write>(&self, stdout: &mut W, wm: &WindowManager, copy_mode: &CopyMode) -> io::Result<()> {
        let status_y = wm.height - 1;
        execute!(stdout, MoveTo(0, status_y))?;
        
        // Yellow background for copy mode
        execute!(stdout, 
            SetBackgroundColor(CtColor::DarkYellow),
            SetForegroundColor(CtColor::Black)
        )?;
        
        let mode_str = if copy_mode.search_mode {
            format!("[SEARCH] {}", copy_mode.search_query)
        } else if copy_mode.selection_start.is_some() {
            format!("[COPY] Selection active | {}", copy_mode.search_status())
        } else {
            format!("[COPY] {}", if copy_mode.search_status().is_empty() {
                "q:quit Space:select /:search".to_string()
            } else {
                copy_mode.search_status()
            })
        };
        
        let padding = (wm.width as usize).saturating_sub(mode_str.len() + 2);
        write!(stdout, " {}{:padding$} ", mode_str, "", padding = padding)?;
        
        execute!(stdout, ResetColor)?;
        Ok(())
    }

    /// Render with rename input overlay
    pub fn render_with_rename(&mut self, wm: &WindowManager, rename_buffer: &str) -> io::Result<()> {
        let stdout = io::stdout();
        let mut out = stdout.lock();

        if !self.initialized {
            self.init()?;
        }

        with_frame(&mut out, |out| {
            self.render_tab_bar(out, wm)?;
            self.render_panes(out, wm)?;
            self.render_status_bar(out, wm)?;
            self.render_rename_popup(out, wm, rename_buffer)?;
            Ok(())
        })
    }

    /// Render rename popup in center of screen
    fn render_rename_popup<W: Write>(&self, stdout: &mut W, wm: &WindowManager, rename_buffer: &str) -> io::Result<()> {
        let box_width = 40.min(wm.width.saturating_sub(4)) as usize;
        let box_height = 5;
        let start_x = ((wm.width as usize).saturating_sub(box_width)) / 2;
        let start_y = ((wm.height as usize).saturating_sub(box_height)) / 2;

        // Draw box
        execute!(stdout, 
            SetBackgroundColor(CtColor::DarkBlue),
            SetForegroundColor(CtColor::White)
        )?;

        // Top border
        execute!(stdout, MoveTo(start_x as u16, start_y as u16))?;
        write!(stdout, "┌─ Rename Window ")?;
        let title_len = 16;
        for _ in 0..(box_width.saturating_sub(title_len + 2)) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┐")?;

        // Empty line
        execute!(stdout, MoveTo(start_x as u16, (start_y + 1) as u16))?;
        write!(stdout, "│{:width$}│", "", width = box_width - 2)?;

        // Input line
        execute!(stdout, MoveTo(start_x as u16, (start_y + 2) as u16))?;
        let input_display = if rename_buffer.len() > box_width - 6 {
            &rename_buffer[rename_buffer.len() - (box_width - 6)..]
        } else {
            rename_buffer
        };
        write!(stdout, "│ {:<width$} │", format!("{}█", input_display), width = box_width - 4)?;

        // Empty line
        execute!(stdout, MoveTo(start_x as u16, (start_y + 3) as u16))?;
        write!(stdout, "│{:width$}│", "", width = box_width - 2)?;

        // Bottom border with help
        execute!(stdout, MoveTo(start_x as u16, (start_y + 4) as u16))?;
        let help = "Enter:OK  Esc:Cancel";
        let help_padding = (box_width.saturating_sub(help.len() + 4)) / 2;
        write!(stdout, "└")?;
        for _ in 0..help_padding {
            write!(stdout, "─")?;
        }
        write!(stdout, " {} ", help)?;
        for _ in 0..(box_width.saturating_sub(help.len() + 4 + help_padding + 2)) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┘")?;

        execute!(stdout, ResetColor)?;
        Ok(())
    }

    /// Render with theme selector overlay
    pub fn render_with_theme_selector(&mut self, wm: &WindowManager, themes: &[&str], selected: usize) -> io::Result<()> {
        let stdout = io::stdout();
        let mut out = stdout.lock();

        if !self.initialized {
            self.init()?;
        }

        with_frame(&mut out, |out| {
            self.render_tab_bar(out, wm)?;
            self.render_panes(out, wm)?;
            self.render_status_bar(out, wm)?;
            self.render_theme_selector(out, wm, themes, selected)?;
            Ok(())
        })
    }

    /// Render theme selector overlay
    fn render_theme_selector<W: Write>(&self, stdout: &mut W, wm: &WindowManager, themes: &[&str], selected: usize) -> io::Result<()> {
        let cs = &self.color_scheme;
        
        let box_width = 40.min(wm.width.saturating_sub(4)) as usize;
        let box_height = (themes.len() + 4).min(wm.height.saturating_sub(4) as usize);
        let start_x = (wm.width as usize - box_width) / 2;
        let start_y = (wm.height as usize - box_height) / 2;

        // Draw box background
        execute!(stdout, 
            SetBackgroundColor(cs.selector_bg.to_crossterm()), 
            SetForegroundColor(cs.selector_fg.to_crossterm())
        )?;

        // Top border
        let title = "Theme [Ctrl+B, t]";
        execute!(stdout, MoveTo(start_x as u16, start_y as u16))?;
        write!(stdout, "┌─ {} ", title)?;
        for _ in 0..(box_width.saturating_sub(title.len() + 5)) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┐")?;

        // Theme items
        for (i, theme) in themes.iter().enumerate() {
            let y = start_y + 1 + i;
            if y >= start_y + box_height - 1 {
                break;
            }
            
            execute!(stdout, MoveTo(start_x as u16, y as u16))?;
            
            if i == selected {
                execute!(stdout, 
                    SetBackgroundColor(cs.selector_selected_bg.to_crossterm()), 
                    SetForegroundColor(cs.selector_selected_fg.to_crossterm())
                )?;
            } else {
                execute!(stdout, 
                    SetBackgroundColor(cs.selector_bg.to_crossterm()), 
                    SetForegroundColor(cs.selector_fg.to_crossterm())
                )?;
            }
            
            let num = i + 1;
            let prefix = format!("│ {}. ", num);
            write!(stdout, "{}", prefix)?;
            write!(stdout, "{}", theme)?;
            
            let used = prefix.len() + theme.len();
            let padding = box_width.saturating_sub(used + 1);
            write!(stdout, "{:padding$}", "", padding = padding)?;
            
            execute!(stdout, 
                SetBackgroundColor(cs.selector_bg.to_crossterm()), 
                SetForegroundColor(cs.selector_fg.to_crossterm())
            )?;
            write!(stdout, "│")?;
        }

        // Bottom border with help
        let help_y = start_y + themes.len() + 1;
        execute!(stdout, MoveTo(start_x as u16, help_y as u16))?;
        write!(stdout, "├")?;
        for _ in 0..(box_width - 2) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┤")?;

        execute!(stdout, MoveTo(start_x as u16, (help_y + 1) as u16))?;
        let help = "Up/Down:Select Enter:Apply Esc:Cancel";
        write!(stdout, "│ {}", help)?;
        let padding = box_width.saturating_sub(help.len() + 3);
        write!(stdout, "{:padding$}│", "", padding = padding)?;

        execute!(stdout, MoveTo(start_x as u16, (help_y + 2) as u16))?;
        write!(stdout, "└")?;
        for _ in 0..(box_width - 2) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┘")?;

        execute!(stdout, ResetColor)?;

        Ok(())
    }

    /// Render history selector overlay
    fn render_selector<W: Write>(&self, stdout: &mut W, wm: &WindowManager, selector: &crate::history::HistorySelector) -> io::Result<()> {
        let cs = &self.color_scheme;
        let box_width = 60.min(wm.width.saturating_sub(4)) as usize;
        let box_height = (selector.max_visible + 4).min(wm.height.saturating_sub(4) as usize);
        let start_x = (wm.width as usize - box_width) / 2;
        let start_y = (wm.height as usize - box_height) / 2;

        // Draw box background
        execute!(stdout, 
            SetBackgroundColor(cs.selector_bg.to_crossterm()), 
            SetForegroundColor(cs.selector_fg.to_crossterm())
        )?;

        // Top border: "┌─ History [Ctrl+R] ───┐"
        let title = "History [Ctrl+R]";
        let title_section_width = 3 + title.len() + 1; // "┌─ " + title + " "
        execute!(stdout, MoveTo(start_x as u16, start_y as u16))?;
        write!(stdout, "┌─ {} ", title)?;
        for _ in 0..(box_width.saturating_sub(title_section_width + 1)) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┐")?;

        // Search line: "│ > query                           │"
        let prompt = "> ";
        let prompt_len = 2; // "> "
        let prefix_len = 2; // "│ "
        execute!(stdout, MoveTo(start_x as u16, (start_y + 1) as u16))?;
        write!(stdout, "│ {}", prompt)?;
        execute!(stdout, SetForegroundColor(cs.status_prefix_bg.to_crossterm()))?;
        
        // Calculate query display width
        let max_query_width = box_width.saturating_sub(prefix_len + prompt_len + 1); // "│ " + "> " + "│"
        let mut query_width = 0;
        let query_display: String = selector.query.chars()
            .take_while(|c| {
                let w = c.width().unwrap_or(1);
                if query_width + w <= max_query_width {
                    query_width += w;
                    true
                } else {
                    false
                }
            })
            .collect();
        write!(stdout, "{}", query_display)?;
        
        execute!(stdout, SetForegroundColor(cs.selector_fg.to_crossterm()))?;
        let padding = box_width.saturating_sub(prefix_len + prompt_len + query_width + 1);
        write!(stdout, "{:padding$}│", "", padding = padding)?;

        // Separator
        execute!(stdout, MoveTo(start_x as u16, (start_y + 2) as u16))?;
        write!(stdout, "├")?;
        for _ in 0..(box_width - 2) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┤")?;

        // History items
        let items = selector.visible_items();
        for (display_idx, command, is_selected) in items.iter() {
            let y = start_y + 3 + display_idx;
            if y >= start_y + box_height - 1 {
                break;
            }
            
            execute!(stdout, MoveTo(start_x as u16, y as u16))?;
            
            if *is_selected {
                execute!(stdout, 
                    SetBackgroundColor(cs.selector_selected_bg.to_crossterm()), 
                    SetForegroundColor(cs.selector_selected_fg.to_crossterm())
                )?;
            } else {
                execute!(stdout, 
                    SetBackgroundColor(cs.selector_bg.to_crossterm()), 
                    SetForegroundColor(cs.selector_fg.to_crossterm())
                )?;
            }
            
            // Fixed width number format: "│ XX. " (always 2 digits for alignment)
            let num = display_idx + 1;
            let prefix = format!("│{:2}. ", num);
            let prefix_len = 5; // "│" + 2digit + ". " = 5 chars
            write!(stdout, "{}", prefix)?;
            
            // Truncate command to fit: box_width - prefix_len - 1 (for trailing "│")
            let max_cmd_width = box_width.saturating_sub(prefix_len + 1);
            let mut cmd_width = 0;
            let cmd: String = command.chars()
                .take_while(|c| {
                    let w = c.width().unwrap_or(1);
                    if cmd_width + w <= max_cmd_width {
                        cmd_width += w;
                        true
                    } else {
                        false
                    }
                })
                .collect();
            write!(stdout, "{}", cmd)?;
            
            // Padding to fill the rest
            let padding = box_width.saturating_sub(prefix_len + cmd_width + 1);
            if padding > 0 {
                write!(stdout, "{:padding$}", "", padding = padding)?;
            }
            
            execute!(stdout, 
                SetBackgroundColor(cs.selector_bg.to_crossterm()), 
                SetForegroundColor(cs.selector_fg.to_crossterm())
            )?;
            write!(stdout, "│")?;
        }

        // Fill empty rows
        for i in items.len()..(selector.max_visible) {
            let y = start_y + 3 + i;
            if y >= start_y + box_height - 1 {
                break;
            }
            execute!(stdout, MoveTo(start_x as u16, y as u16))?;
            write!(stdout, "│{:width$}│", "", width = box_width - 2)?;
        }

        // Bottom border with help (English)
        let help_text = "Enter:Run S-Enter:&& C-Enter:& Esc:Close";
        let help_width = help_text.len();
        execute!(stdout, MoveTo(start_x as u16, (start_y + box_height - 1) as u16))?;
        write!(stdout, "└ {} ", help_text)?;
        for _ in 0..(box_width.saturating_sub(help_width + 4)) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┘")?;

        execute!(stdout, ResetColor)?;
        
        // Position cursor in search box (after "│ > ")
        let cursor_x = start_x + prefix_len + prompt_len + query_width;
        execute!(stdout, MoveTo(cursor_x as u16, (start_y + 1) as u16), Show)?;

        Ok(())
    }

    /// Render the tab bar
    fn render_tab_bar<W: Write>(&self, stdout: &mut W, wm: &WindowManager) -> io::Result<()> {
        let cs = &self.color_scheme;
        
        execute!(stdout, MoveTo(0, 0))?;
        
        // Background
        execute!(stdout, 
            SetBackgroundColor(cs.tab_bar_bg.to_crossterm()), 
            SetForegroundColor(cs.tab_bar_fg.to_crossterm())
        )?;
        
        // Clear tab bar
        write!(stdout, "{:width$}", "", width = wm.width as usize)?;
        execute!(stdout, MoveTo(0, 0))?;

        // Render tabs
        let tabs = wm.tab_info();
        for (i, (_id, name, active)) in tabs.iter().enumerate() {
            if *active {
                execute!(stdout, 
                    SetBackgroundColor(cs.tab_active_bg.to_crossterm()), 
                    SetForegroundColor(cs.tab_active_fg.to_crossterm())
                )?;
            } else {
                execute!(stdout, 
                    SetBackgroundColor(cs.tab_inactive_bg.to_crossterm()), 
                    SetForegroundColor(cs.tab_inactive_fg.to_crossterm())
                )?;
            }
            write!(stdout, " {} ", name)?;
            
            if i < tabs.len() - 1 {
                execute!(stdout, 
                    SetBackgroundColor(cs.tab_bar_bg.to_crossterm()), 
                    SetForegroundColor(cs.tab_bar_fg.to_crossterm())
                )?;
                write!(stdout, "│")?;
            }
        }

        // Show prefix mode indicator
        if wm.prefix_mode {
            execute!(stdout, MoveTo(wm.width - 10, 0))?;
            execute!(stdout, 
                SetBackgroundColor(cs.status_prefix_bg.to_crossterm()), 
                SetForegroundColor(cs.status_prefix_fg.to_crossterm())
            )?;
            write!(stdout, " PREFIX ")?;
        }

        execute!(stdout, ResetColor)?;
        Ok(())
    }

    /// Render all panes
    fn render_panes<W: Write>(&mut self, stdout: &mut W, wm: &WindowManager) -> io::Result<()> {
        let tab = match wm.active_tab() {
            Some(t) => t,
            None => return Ok(()),
        };

        // Full redraw if generation changed
        let needs_full_redraw = tab.layout_generation != self.last_generation;
        if needs_full_redraw {
            execute!(stdout, ResetColor)?;
            for row in wm.tab_bar_height..(wm.height.saturating_sub(1)) {
                execute!(stdout, MoveTo(0, row), Clear(ClearType::CurrentLine))?;
            }
            self.last_generation = tab.layout_generation;
        }

        // If zoomed, only render the zoomed pane
        if tab.is_zoomed() {
            if let Some(zoomed_id) = tab.zoomed_pane_id() {
                if let Some(pane) = tab.panes.get(&zoomed_id) {
                    self.render_pane(stdout, pane, wm.tab_bar_height)?;
                }
            }
        } else {
            for pane in tab.panes.values() {
                self.render_pane(stdout, pane, wm.tab_bar_height)?;
            }
        }

        Ok(())
    }

    /// Render a single pane
    fn render_pane<W: Write>(&self, stdout: &mut W, pane: &Pane, y_offset: u16) -> io::Result<()> {
        let screen = pane.session.state.active_screen();
        let (inner_x, inner_y) = pane.inner_pos();
        let (inner_w, inner_h) = pane.inner_size();
        let has_selection = pane.session.state.selection.is_some();
        
        // Always render full inner_w to prevent remnants
        // If session.cols is smaller, we'll pad with spaces
        let session_cols = pane.session.state.cols as usize;

        // Draw border if needed
        if pane.border != BorderStyle::None {
            self.render_border(stdout, pane, y_offset)?;
        }

        // Render content
        let mut current_attrs = CellAttrs::default();
        let mut current_selected = false;
        let mut line_buffer = String::with_capacity(256);
        
        for row_idx in 0..inner_h as usize {
            execute!(stdout, MoveTo(inner_x, y_offset + inner_y + row_idx as u16))?;
            line_buffer.clear();
            
            let mut rendered_width: usize = 0;
            
            let row = match screen.get_row_at(row_idx) {
                Some(r) => r,
                None => {
                    // Clear empty row - fill with spaces
                    execute!(stdout, ResetColor)?;
                    write!(stdout, "{:width$}", "", width = inner_w as usize)?;
                    continue;
                }
            };

            // Output cells sequentially, letting the terminal handle positioning
            for (col_idx, cell) in row.cells.iter().enumerate() {
                if col_idx >= session_cols || rendered_width >= inner_w as usize {
                    break;
                }

                if cell.is_continuation() {
                    continue;
                }

                // Check if this cell is selected
                let is_selected = has_selection && pane.session.state.is_selected(col_idx as u16, row_idx as u16);

                // Check if we need to flush and change attributes
                let attrs_changed = cell.attrs != current_attrs || is_selected != current_selected;
                
                if attrs_changed && !line_buffer.is_empty() {
                    self.apply_attrs_with_selection(stdout, &current_attrs, current_selected)?;
                    write!(stdout, "{}", line_buffer)?;
                    line_buffer.clear();
                }
                
                if attrs_changed {
                    current_attrs = cell.attrs.clone();
                    current_selected = is_selected;
                }

                let ch = cell.display_char();
                line_buffer.push_str(ch);
                rendered_width += unicode_width::UnicodeWidthStr::width(ch);
            }

            // Flush remaining text
            if !line_buffer.is_empty() {
                self.apply_attrs_with_selection(stdout, &current_attrs, current_selected)?;
                write!(stdout, "{}", line_buffer)?;
                line_buffer.clear();
            }

            // Pad the rest of the line with spaces to prevent remnants
            if rendered_width < inner_w as usize {
                execute!(stdout, ResetColor)?;
                write!(stdout, "{:width$}", "", width = inner_w as usize - rendered_width)?;
            }
        }

        execute!(stdout, ResetColor, SetAttribute(Attribute::Reset))?;
        Ok(())
    }

    /// Render pane border
    fn render_border<W: Write>(&self, stdout: &mut W, pane: &Pane, y_offset: u16) -> io::Result<()> {
        let cs = &self.color_scheme;
        let chars = BorderChars::single();
        
        // Border color based on focus
        if pane.focused {
            execute!(stdout, SetForegroundColor(cs.pane_border_active.to_crossterm()))?;
        } else {
            execute!(stdout, SetForegroundColor(cs.pane_border.to_crossterm()))?;
        }

        // Top border
        execute!(stdout, MoveTo(pane.x, y_offset + pane.y))?;
        write!(stdout, "{}", chars.top_left)?;
        
        // Title in top border
        let title = pane.display_title();
        let title_space = (pane.width as usize).saturating_sub(4);
        let display_title: String = if title.len() > title_space {
            title.chars().take(title_space).collect()
        } else {
            title
        };
        
        let remaining = pane.width.saturating_sub(2 + display_title.len() as u16);
        let left_pad = remaining / 2;
        let right_pad = remaining - left_pad;
        
        for _ in 0..left_pad {
            write!(stdout, "{}", chars.horizontal)?;
        }
        
        if pane.focused {
            execute!(stdout, SetForegroundColor(cs.tab_active_fg.to_crossterm()))?;
        }
        write!(stdout, "{}", display_title)?;
        if pane.focused {
            execute!(stdout, SetForegroundColor(cs.pane_border_active.to_crossterm()))?;
        } else {
            execute!(stdout, SetForegroundColor(cs.pane_border.to_crossterm()))?;
        }
        
        for _ in 0..right_pad {
            write!(stdout, "{}", chars.horizontal)?;
        }
        write!(stdout, "{}", chars.top_right)?;

        // Side borders
        for row in 1..pane.height.saturating_sub(1) {
            execute!(stdout, MoveTo(pane.x, y_offset + pane.y + row))?;
            write!(stdout, "{}", chars.vertical)?;
            execute!(stdout, MoveTo(pane.x + pane.width - 1, y_offset + pane.y + row))?;
            write!(stdout, "{}", chars.vertical)?;
        }

        // Bottom border
        if pane.height > 1 {
            execute!(stdout, MoveTo(pane.x, y_offset + pane.y + pane.height - 1))?;
            write!(stdout, "{}", chars.bottom_left)?;
            for _ in 0..pane.width.saturating_sub(2) {
                write!(stdout, "{}", chars.horizontal)?;
            }
            write!(stdout, "{}", chars.bottom_right)?;
        }

        execute!(stdout, ResetColor)?;
        Ok(())
    }

    /// Render the status bar
    fn render_status_bar<W: Write>(&self, stdout: &mut W, wm: &WindowManager) -> io::Result<()> {
        let cs = &self.color_scheme;
        let status_y = wm.height - 1;
        execute!(stdout, MoveTo(0, status_y))?;
        
        execute!(stdout, 
            SetBackgroundColor(cs.status_bar_bg.to_crossterm()), 
            SetForegroundColor(cs.status_bar_fg.to_crossterm())
        )?;
        
        let status = wm.status_info();
        let shortcuts = if wm.prefix_mode {
            r#"c:new x:kill ":split %:vsplit n/p:win o:pane z:zoom t:theme"#
        } else {
            "Ctrl+B: prefix | Ctrl+R: history"
        };
        
        let left_len = status.len();
        let right_len = shortcuts.len();
        let padding = (wm.width as usize).saturating_sub(left_len + right_len + 2);
        
        write!(stdout, " {}{:padding$}{} ", status, "", shortcuts, padding = padding)?;
        
        execute!(stdout, ResetColor)?;
        Ok(())
    }

    /// Apply cell attributes with selection highlighting
    fn apply_attrs_with_selection<W: Write>(&self, stdout: &mut W, attrs: &CellAttrs, selected: bool) -> io::Result<()> {
        let cs = &self.color_scheme;
        execute!(stdout, SetAttribute(Attribute::Reset))?;

        if selected {
            // Selection: use color scheme colors
            execute!(stdout, SetBackgroundColor(cs.selection_bg.to_crossterm()))?;
            execute!(stdout, SetForegroundColor(cs.selection_fg.to_crossterm()))?;
        } else {
            // Foreground
            match attrs.fg {
                Color::Default => {}
                Color::Indexed(idx) => {
                    execute!(stdout, SetForegroundColor(CtColor::AnsiValue(idx)))?;
                }
                Color::Rgb(r, g, b) => {
                    execute!(stdout, SetForegroundColor(CtColor::Rgb { r, g, b }))?;
                }
            }

            // Background
            match attrs.bg {
                Color::Default => {}
                Color::Indexed(idx) => {
                    execute!(stdout, SetBackgroundColor(CtColor::AnsiValue(idx)))?;
                }
                Color::Rgb(r, g, b) => {
                    execute!(stdout, SetBackgroundColor(CtColor::Rgb { r, g, b }))?;
                }
            }
        }

        // Attributes (apply regardless of selection)
        if attrs.flags.contains(AttrFlags::BOLD) {
            execute!(stdout, SetAttribute(Attribute::Bold))?;
        }
        if attrs.flags.contains(AttrFlags::DIM) {
            execute!(stdout, SetAttribute(Attribute::Dim))?;
        }
        if attrs.flags.contains(AttrFlags::ITALIC) {
            execute!(stdout, SetAttribute(Attribute::Italic))?;
        }
        if attrs.flags.contains(AttrFlags::UNDERLINE) {
            execute!(stdout, SetAttribute(Attribute::Underlined))?;
        }

        Ok(())
    }

    /// Apply cell attributes
    #[allow(dead_code)]
    fn apply_attrs<W: Write>(&self, stdout: &mut W, attrs: &CellAttrs) -> io::Result<()> {
        self.apply_attrs_with_selection(stdout, attrs, false)
    }

    /// Render with context menu overlay
    pub fn render_with_context_menu(&mut self, wm: &mut WindowManager, menu: &ContextMenu) -> io::Result<()> {
        // First render normally
        self.render(wm)?;
        
        // Then overlay the menu if visible
        if menu.visible {
            let mut stdout = io::stdout().lock();
            with_cursor_hidden(&mut stdout, |out| {
                self.render_context_menu(out, menu)
            })?;
        }
        
        Ok(())
    }

    /// Render only the context menu (for hover updates without full redraw)
    pub fn render_context_menu_only(&self, menu: &ContextMenu) -> io::Result<()> {
        if menu.visible {
            let mut stdout = io::stdout().lock();
            with_cursor_hidden(&mut stdout, |out| {
                self.render_context_menu(out, menu)
            })?;
        }
        Ok(())
    }

    /// Render the context menu
    fn render_context_menu<W: Write>(&self, stdout: &mut W, menu: &ContextMenu) -> io::Result<()> {
        let cs = &self.color_scheme;
        let content_width = menu.content_width() as usize;
        let (_, height) = menu.dimensions();
        
        // Use pre-adjusted position from menu.show()
        let x = menu.x;
        let y = menu.y;
        
        // Menu colors
        let menu_bg = cs.status_bar_bg.to_crossterm();
        let menu_fg = cs.status_bar_fg.to_crossterm();
        let selected_bg = cs.tab_active_bg.to_crossterm();
        let selected_fg = cs.tab_active_fg.to_crossterm();
        
        // Draw border
        execute!(stdout, SetBackgroundColor(menu_bg), SetForegroundColor(menu_fg))?;
        
        // Top border: ┌────────┐
        execute!(stdout, MoveTo(x, y))?;
        write!(stdout, "┌")?;
        for _ in 0..content_width {
            write!(stdout, "─")?;
        }
        write!(stdout, "┐")?;
        
        // Menu items
        for (i, item) in menu.items.iter().enumerate() {
            let row = y + 1 + i as u16;
            execute!(stdout, MoveTo(x, row))?;
            
            // Left border
            execute!(stdout, SetBackgroundColor(menu_bg), SetForegroundColor(menu_fg))?;
            write!(stdout, "│")?;
            
            // Item content (with selection highlight)
            if i == menu.selected {
                execute!(stdout, SetBackgroundColor(selected_bg), SetForegroundColor(selected_fg))?;
            } else {
                execute!(stdout, SetBackgroundColor(menu_bg), SetForegroundColor(menu_fg))?;
            }
            
            // Format: " label (shortcut)"
            let shortcut_str = item.shortcut.map(|s| format!(" ({})", s)).unwrap_or_default();
            let label_with_shortcut = format!(" {}{}", item.label, shortcut_str);
            let label_len = label_with_shortcut.chars().count();
            let padding = content_width.saturating_sub(label_len);
            write!(stdout, "{}{:padding$}", label_with_shortcut, "", padding = padding)?;
            
            // Right border
            execute!(stdout, SetBackgroundColor(menu_bg), SetForegroundColor(menu_fg))?;
            write!(stdout, "│")?;
        }
        
        // Bottom border: └────────┘
        execute!(stdout, MoveTo(x, y + height - 1))?;
        write!(stdout, "└")?;
        for _ in 0..content_width {
            write!(stdout, "─")?;
        }
        write!(stdout, "┘")?;
        
        execute!(stdout, ResetColor)?;
        stdout.flush()?;
        
        Ok(())
    }
}

impl Drop for WmRenderer {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}
