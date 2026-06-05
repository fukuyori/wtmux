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
//! begin_frame()  → Disable autowrap, start sync
//!     ↓
//! render content → Tab bar, panes, status bar
//!     ↓
//! end_frame()    → Enable autowrap, end sync, flush
//! ```
//!
//! # Performance Optimizations
//!
//! - Generation-based dirty tracking to minimize redraws
//! - Partial updates for cursor movement and selection
//! - Separate overlay rendering for context menus (avoids full redraw)

use std::io::{self, Write};
use crossterm::{
    cursor::{MoveTo, Show},
    execute,
    style::{
        Attribute, Color as CtColor, ResetColor, SetAttribute,
        SetBackgroundColor, SetForegroundColor,
    },
    terminal::{self, Clear, ClearType},
};
use unicode_width::UnicodeWidthChar;

/// Display width for a character, with Nerd Font / Powerline PUA fix.
/// Mirrors the same function in `core::term::state`.
#[inline]
fn char_width(ch: char) -> usize {
    let cp = ch as u32;
    if (0xE000..=0xF8FF).contains(&cp)
        || (0xF0000..=0xFFFFF).contains(&cp)
        || (0x100000..=0x10FFFF).contains(&cp)
    {
        return 1;
    }
    ch.width().unwrap_or(1)
}

#[inline]
fn str_display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

fn truncate_to_display_width(s: &str, max_width: usize) -> String {
    let mut out = String::new();
    let mut width = 0;
    for ch in s.chars() {
        let ch_width = char_width(ch);
        if width + ch_width > max_width {
            break;
        }
        out.push(ch);
        width += ch_width;
    }
    out
}

use crate::wm::{WindowManager, Pane, BorderStyle};
use crate::core::term::{AttrFlags, CellAttrs, Color};
use crate::config::{ColorScheme, ParsedKeyBindings};
use crate::copymode::CopyMode;
use super::row_stream::{render_row_stream, RenderRow};
use super::context_menu::ContextMenu;
use super::cursor::CursorPresenter;
use super::frame::{with_cursor_hidden, with_frame, with_hidden_cursor_frame};

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
    history_selector_shortcut: String,
    /// Last rendered layout generation (for detecting changes)
    last_generation: u64,
    cursor: CursorPresenter,
    /// When true, SGR 1 (Bold) is suppressed.
    /// Use this when the Nerd Font installed lacks a Bold face and the OS
    /// falls back to a non-Nerd-Font bold, causing PUA glyphs to render
    /// with wrong cell widths.
    pub suppress_bold: bool,
}

#[derive(Clone, PartialEq)]
enum RenderRowStyle {
    Cell {
        attrs: CellAttrs,
        selected: bool,
    },
    SearchMatch,
    CurrentMatch,
}

impl WmRenderer {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            initialized: false,
            color_scheme: ColorScheme::default(),
            history_selector_shortcut: "Ctrl+R".to_string(),
            last_generation: 0,
            cursor: CursorPresenter::default(),
            suppress_bold: false,
        }
    }

    pub fn with_color_scheme(color_scheme: ColorScheme) -> Self {
        Self {
            initialized: false,
            color_scheme,
            history_selector_shortcut: "Ctrl+R".to_string(),
            last_generation: 0,
            cursor: CursorPresenter::default(),
            suppress_bold: false,
        }
    }

    /// Set keyboard shortcut labels used by the status bar.
    pub fn set_keybindings(&mut self, keybindings: &ParsedKeyBindings) {
        self.history_selector_shortcut = keybindings.history_selector.display_name();
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
            crossterm::event::EnableBracketedPaste,
            Clear(ClearType::All)
        )?;
        stdout.flush()?;
        
        self.initialized = true;
        self.invalidate_cursor_cache();
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
            crossterm::event::DisableBracketedPaste,
            crossterm::event::DisableMouseCapture,
            crossterm::terminal::LeaveAlternateScreen
        )?;
        terminal::disable_raw_mode()?;
        self.initialized = false;
        self.invalidate_cursor_cache();
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

    fn invalidate_cursor_cache(&mut self) {
        self.cursor.invalidate();
    }

    /// Render with optional snippet selector
    pub fn render_with_selector(&mut self, wm: &WindowManager, selector: Option<&crate::history::HistorySelector>) -> io::Result<()> {
        let stdout = io::stdout();
        let mut out = io::BufWriter::with_capacity(65536, stdout.lock());
        let snippet_visible = selector.map(|s| s.visible).unwrap_or(false);

        let result = with_frame(&mut out, |out| {
            self.render_tab_bar(out, wm)?;
            self.render_panes(out, wm)?;
            self.render_status_bar(out, wm)?;

            if let Some(sel) = selector {
                if sel.visible {
                    self.render_selector(out, wm, sel)?;
                }
            }

            // Show cursor at focused pane's cursor position (unless snippet selector is visible)
            if !snippet_visible {
                self.cursor.show_focused_pane_cursor(out, wm)?;
            }
            Ok(())
        });

        if snippet_visible {
            self.invalidate_cursor_cache();
        }

        result
    }

    /// Render with pane numbers overlay
    pub fn render_with_pane_numbers(&mut self, wm: &WindowManager) -> io::Result<()> {
        let stdout = io::stdout();
        let mut out = stdout.lock();

        if !self.initialized {
            self.init()?;
        }

        let result = with_hidden_cursor_frame(&mut out, |out| {
            self.render_tab_bar(out, wm)?;
            self.render_panes(out, wm)?;
            self.render_status_bar(out, wm)?;
            self.render_pane_numbers(out, wm)?;
            Ok(())
        });
        self.invalidate_cursor_cache();
        result
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

        let result = with_hidden_cursor_frame(&mut out, |out| {
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
        });
        self.invalidate_cursor_cache();
        result
    }

    /// Fast update for copy mode - only update cursor and status
    pub fn render_copy_mode_cursor_only(&mut self, wm: &WindowManager, copy_mode: &CopyMode) -> io::Result<()> {
        let stdout = io::stdout();
        let mut out = stdout.lock();

        let result = with_cursor_hidden(&mut out, |out| {
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
        });
        self.invalidate_cursor_cache();
        result
    }

    /// Render pane content in copy mode with selection/search highlighting
    fn render_pane_with_copy_mode<W: Write>(&self, stdout: &mut W, wm: &WindowManager, copy_mode: &CopyMode) -> io::Result<()> {
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
                let render_row = RenderRow::new(line, render_w);
                self.render_cells(stdout, render_row, render_w, |cell_idx, cell| {
                    let cell_col = cell_idx as u16;
                    if copy_mode.is_current_match(abs_row, cell_col) {
                        RenderRowStyle::CurrentMatch
                    } else if copy_mode.is_search_match(abs_row, cell_col) {
                        RenderRowStyle::SearchMatch
                    } else {
                        RenderRowStyle::Cell {
                            attrs: cell.attrs.clone(),
                            selected: copy_mode.is_selected(abs_row, cell_col),
                        }
                    }
                })?;
            } else {
                self.clear_row(stdout, inner_w as usize)?;
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

        let result = with_hidden_cursor_frame(&mut out, |out| {
            self.render_tab_bar(out, wm)?;
            self.render_panes(out, wm)?;
            self.render_status_bar(out, wm)?;
            self.render_rename_popup(out, wm, rename_buffer)?;
            Ok(())
        });
        self.invalidate_cursor_cache();
        result
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

        let result = with_hidden_cursor_frame(&mut out, |out| {
            self.render_tab_bar(out, wm)?;
            self.render_panes(out, wm)?;
            self.render_status_bar(out, wm)?;
            self.render_theme_selector(out, wm, themes, selected)?;
            Ok(())
        });
        self.invalidate_cursor_cache();
        result
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

        // Top border: ┌─ Theme [Ctrl+B, t] ───...───┐
        let title = format!("Theme [{}, t]", wm.prefix_key.display_name());
        let title_display_width = title.chars().count();
        execute!(stdout, MoveTo(start_x as u16, start_y as u16))?;
        write!(stdout, "┌─ {} ", title)?;
        // "┌─ " = 3 display chars, " " after title = 1 char, "┐" = 1 char
        // Total fixed chars = 3 + 1 + 1 = 5
        let remaining = box_width.saturating_sub(title_display_width + 5);
        for _ in 0..remaining {
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
            let prefix_display_width = prefix.chars().count();  // Use char count
            write!(stdout, "{}", prefix)?;
            write!(stdout, "{}", theme)?;
            
            let theme_display_width = theme.chars().count();
            let used = prefix_display_width + theme_display_width;
            let padding = box_width.saturating_sub(used + 1);  // +1 for closing │
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
        let help_display_width = help.chars().count();  // Use char count
        write!(stdout, "│ {}", help)?;
        let padding = box_width.saturating_sub(help_display_width + 3);  // "│ " = 2 chars, "│" = 1 char
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

        // Top border: "┌─ History [<shortcut>] ───┐"
        let title = format!("History [{}]", self.history_selector_shortcut);
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
                let w = char_width(*c);
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
                    let w = char_width(*c);
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
        let help_text = "Enter:Run Del:Delete S-Enter:&& Esc:Close";
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

        // Full redraw if generation changed (layout change: split, close, resize)
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
                    self.render_pane(stdout, pane, wm.tab_bar_height, needs_full_redraw)?;
                }
            }
        } else {
            for pane in tab.panes.values() {
                let screen = pane.session.state.active_screen();
                // Skip panes with no new content unless forced by layout change
                let pane_needs_render = needs_full_redraw
                    || screen.full_redraw
                    || screen.has_dirty_lines();

                if pane_needs_render {
                    self.render_pane(stdout, pane, wm.tab_bar_height, needs_full_redraw)?;
                }
            }
        }

        Ok(())
    }

    /// Render a single pane
    fn render_pane<W: Write>(&self, stdout: &mut W, pane: &Pane, y_offset: u16, force_full: bool) -> io::Result<()> {
        let screen = pane.session.state.active_screen();
        let (inner_x, inner_y) = pane.inner_pos();
        let (inner_w, inner_h) = pane.inner_size();
        let has_selection = pane.session.state.selection.is_some();
        let session_cols = pane.session.state.cols as usize;

        // Whether to render all rows or only dirty ones
        let full_redraw = force_full || screen.full_redraw;

        // Draw border if needed
        if pane.border != BorderStyle::None {
            self.render_border(stdout, pane, y_offset)?;
        }

        // Render content as a regular left-to-right line stream and let the host
        // terminal handle glyph shaping / width. This avoids bespoke per-cell
        // cursor anchoring, which tended to cause visible flicker and dropped
        // characters during redraws.
        let render_width = session_cols.min(inner_w as usize);
        
        for row_idx in 0..inner_h as usize {
            // --- Dirty line skip ---
            if !full_redraw && !screen.is_line_dirty(row_idx) {
                continue;
            }

            let row_y = y_offset + inner_y + row_idx as u16;
            execute!(stdout, MoveTo(inner_x, row_y))?;

            let row = match screen.get_row_at(row_idx) {
                Some(r) => r,
                None => {
                    self.clear_row(stdout, inner_w as usize)?;
                    continue;
                }
            };

            let render_row = RenderRow::new(&row.cells, render_width);
            self.render_cells(stdout, render_row, inner_w as usize, |col_idx, cell| RenderRowStyle::Cell {
                attrs: cell.attrs.clone(),
                selected: has_selection
                    && pane.session.state.is_selected(col_idx as u16, row_idx as u16),
            })?;
        }

        execute!(stdout, ResetColor, SetAttribute(Attribute::Reset))?;
        Ok(())
    }

    fn render_cells<W, F>(
        &self,
        stdout: &mut W,
        row: RenderRow<'_>,
        clear_width: usize,
        mut style_for: F,
    ) -> io::Result<()>
    where
        W: Write,
        F: FnMut(usize, &crate::core::term::Cell) -> RenderRowStyle,
    {
        let rendered_width = render_row_stream(stdout, row, |col_idx, cell| style_for(col_idx, cell), |stdout, style| {
            self.apply_render_row_style(stdout, style)
        })?;

        if rendered_width < clear_width {
            self.clear_row_tail(stdout, clear_width - rendered_width)?;
        }

        Ok(())
    }

    fn apply_render_row_style<W: Write>(
        &self,
        stdout: &mut W,
        style: &RenderRowStyle,
    ) -> io::Result<()> {
        match style {
            RenderRowStyle::Cell { attrs, selected } => {
                self.apply_attrs_with_selection(stdout, attrs, *selected)
            }
            RenderRowStyle::SearchMatch => execute!(
                stdout,
                SetBackgroundColor(CtColor::DarkYellow),
                SetForegroundColor(CtColor::Black)
            ),
            RenderRowStyle::CurrentMatch => execute!(
                stdout,
                SetBackgroundColor(CtColor::Yellow),
                SetForegroundColor(CtColor::Black)
            ),
        }
    }

    fn clear_row<W: Write>(&self, stdout: &mut W, width: usize) -> io::Result<()> {
        self.clear_row_tail(stdout, width)
    }

    fn clear_row_tail<W: Write>(&self, stdout: &mut W, width: usize) -> io::Result<()> {
        execute!(stdout, ResetColor, SetAttribute(Attribute::Reset))?;
        if width > 0 {
            write!(stdout, "{:width$}", "", width = width)?;
        }
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
        let display_title = truncate_to_display_width(&title, title_space);
        
        let remaining = pane.width.saturating_sub(2 + str_display_width(&display_title) as u16);
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
        let prefix_name = wm.prefix_key.display_name();
        let shortcuts = if wm.prefix_mode {
            r#"c:new x:kill ":split %:vsplit n/p:win o:pane z:zoom t:theme"#.to_string()
        } else {
            format!("{}: prefix | {}: history", prefix_name, self.history_selector_shortcut)
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

        // Batch all SGR codes into a single escape sequence for efficiency.
        // Instead of multiple execute!() calls (each a separate write), we build
        // one \x1b[...m sequence with semicolon-separated parameters.
        let mut sgr = String::with_capacity(48);
        sgr.push_str("\x1b[0"); // Always start with reset

        // Text attributes
        if attrs.flags.contains(AttrFlags::BOLD) && !self.suppress_bold { sgr.push_str(";1"); }
        if attrs.flags.contains(AttrFlags::DIM)       { sgr.push_str(";2"); }
        if attrs.flags.contains(AttrFlags::ITALIC)    { sgr.push_str(";3"); }
        if attrs.flags.contains(AttrFlags::UNDERLINE) { sgr.push_str(";4"); }
        if attrs.flags.contains(AttrFlags::BLINK)     { sgr.push_str(";5"); }
        if attrs.flags.contains(AttrFlags::HIDDEN)    { sgr.push_str(";8"); }
        if attrs.flags.contains(AttrFlags::STRIKETHROUGH) { sgr.push_str(";9"); }

        // Colors
        //
        // INVERSE (SGR 7) strategy: pass SGR 7 through to the host terminal
        // and emit the ORIGINAL (non-swapped) FG/BG colors from the cell.
        // The host terminal then performs the swap with full knowledge of its
        // own default background color.
        //
        // Why NOT swap ourselves: when attrs.bg=Default and INVERSE=on, the
        // "swapped fg" should be the HOST terminal's background color (e.g.
        // #1e1e1e in a dark theme).  We don't know that color — emitting
        // Color::Default as FG causes the host terminal to use its DEFAULT
        // FOREGROUND (white), not its background, producing wrong colors.
        //
        // Passing SGR 7 + original FG/BG lets the host terminal do:
        //   "swap FG=blue with BG=terminal-default-bg" → correct dark arrow.
        if attrs.flags.contains(AttrFlags::INVERSE) { sgr.push_str(";7"); }

        if selected {
            sgr.push_str(&format!(";38;2;{};{};{}", cs.selection_fg.r, cs.selection_fg.g, cs.selection_fg.b));
            sgr.push_str(&format!(";48;2;{};{};{}", cs.selection_bg.r, cs.selection_bg.g, cs.selection_bg.b));
        } else {
            match attrs.fg {
                Color::Default => {}
                Color::Indexed(idx) => {
                    if idx < 8        { sgr.push_str(&format!(";{}", 30 + idx)); }
                    else if idx < 16  { sgr.push_str(&format!(";{}", 90 + (idx - 8))); }
                    else              { sgr.push_str(&format!(";38;5;{}", idx)); }
                }
                Color::Rgb(r, g, b) => { sgr.push_str(&format!(";38;2;{};{};{}", r, g, b)); }
            }
            match attrs.bg {
                Color::Default => {}
                Color::Indexed(idx) => {
                    if idx < 8        { sgr.push_str(&format!(";{}", 40 + idx)); }
                    else if idx < 16  { sgr.push_str(&format!(";{}", 100 + (idx - 8))); }
                    else              { sgr.push_str(&format!(";48;5;{}", idx)); }
                }
                Color::Rgb(r, g, b) => { sgr.push_str(&format!(";48;2;{};{};{}", r, g, b)); }
            }
        }

        sgr.push('m');
        write!(stdout, "{}", sgr)
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
            self.invalidate_cursor_cache();
        }
        
        Ok(())
    }

    /// Render only the context menu (for hover updates without full redraw)
    pub fn render_context_menu_only(&mut self, menu: &ContextMenu) -> io::Result<()> {
        if menu.visible {
            let mut stdout = io::stdout().lock();
            with_cursor_hidden(&mut stdout, |out| {
                self.render_context_menu(out, menu)
            })?;
            self.invalidate_cursor_cache();
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

#[cfg(test)]
mod tests {
    use super::{char_width, str_display_width, truncate_to_display_width};

    #[test]
    fn display_width_counts_cjk_as_two_cells() {
        assert_eq!(char_width('a'), 1);
        assert_eq!(char_width('日'), 2);
        assert_eq!(str_display_width("abc日本語"), 9);
    }

    #[test]
    fn truncate_to_display_width_respects_wide_chars() {
        assert_eq!(truncate_to_display_width("abc日本語", 5), "abc日");
        assert_eq!(truncate_to_display_width("日本語abc", 4), "日本");
        assert_eq!(truncate_to_display_width("abc", 2), "ab");
    }

}
