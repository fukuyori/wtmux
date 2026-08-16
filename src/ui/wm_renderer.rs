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
//! - One synchronized scene frame for base content and modal overlays

use std::io::{self, Write};
use crossterm::{
    cursor::{Hide, MoveTo, Show},
    execute, queue,
    style::{
        Attribute, Color as CtColor, ResetColor, SetAttribute,
        SetBackgroundColor, SetForegroundColor,
    },
    terminal::{self, Clear, ClearType},
};

use crate::wm::{WindowManager, Pane, BorderStyle};
use crate::core::term::{AttrFlags, CellAttrs, Color};
use crate::core::term::width::{
    char_width, str_display_width, truncate_tail_to_display_width,
    truncate_to_display_width,
};
use crate::config::{ColorScheme, ParsedKeyBindings};
use crate::copymode::CopyMode;
use crate::history::HistorySelector;
use super::row_stream::{render_row_stream, RenderRow};
use super::context_menu::ContextMenu;
use super::cursor::CursorPresenter;
use super::frame::{with_cursor_hidden, with_frame};
use super::agent_dashboard::AgentDashboard;
use super::message_composer::MessageComposer;
use super::window_selector::{TreeEntry, WindowSelector};

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

/// Geometry of the window selector overlay (tmux `choose-window` style).
///
/// Produced by [`WmRenderer::window_selector_layout`]; rendering and mouse
/// hit-testing both derive from it so they can never disagree.
pub struct WindowSelectorLayout {
    start_x: usize,
    start_y: usize,
    box_width: usize,
    box_height: usize,
    list_h: usize,
    preview_h: usize,
    first_visible: usize,
}

impl WindowSelectorLayout {
    /// Zero-based window index of the list row at a screen position.
    pub fn list_row_at(&self, window_count: usize, column: u16, row: u16) -> Option<usize> {
        let (col, row) = (column as usize, row as usize);
        if col < self.start_x || col >= self.start_x + self.box_width {
            return None;
        }
        let list_top = self.start_y + 1;
        if row < list_top || row >= list_top + self.list_h {
            return None;
        }
        let index = self.first_visible + (row - list_top);
        (index < window_count).then_some(index)
    }

    /// Whether a screen position falls inside the overlay box.
    pub fn contains(&self, column: u16, row: u16) -> bool {
        let (col, row) = (column as usize, row as usize);
        col >= self.start_x
            && col < self.start_x + self.box_width
            && row >= self.start_y
            && row < self.start_y + self.box_height
    }
}

/// Which border of the message composer a resize drag grabbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComposerResizeEdge {
    Right,
    Bottom,
    Corner,
}

/// Geometry of the message composer overlay (`Prefix + m`).
///
/// Produced by [`WmRenderer::message_composer_layout`]; rendering and mouse
/// hit-testing both derive from it so they can never disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MessageComposerLayout {
    pub start_x: usize,
    pub start_y: usize,
    pub box_width: usize,
    /// Editor rows between the top border and the separator
    pub body_h: usize,
    /// Cells available for text on one editor row
    pub inner_width: usize,
    /// Index into the wrapped display rows of the first visible one
    pub first_visible: usize,
}

impl MessageComposerLayout {
    /// Full box height: body plus top border, separator, help line, and
    /// bottom border.
    pub fn box_height(&self) -> usize {
        self.body_h + 4
    }

    /// Map a screen position to `(display row index, cell offset)` in the
    /// editor body, or None outside it.
    pub fn body_pos_at(&self, column: u16, row: u16) -> Option<(usize, usize)> {
        let (col, row) = (column as usize, row as usize);
        let text_left = self.start_x + 2;
        if row < self.start_y + 1 || row >= self.start_y + 1 + self.body_h {
            return None;
        }
        if col < text_left || col >= text_left + self.inner_width {
            return None;
        }
        Some((self.first_visible + (row - self.start_y - 1), col - text_left))
    }

    /// Like [`Self::body_pos_at`] but clamping outside positions onto the
    /// body, so a drag-selection keeps tracking when the pointer leaves
    /// the box.
    pub fn body_pos_clamped(&self, column: u16, row: u16) -> (usize, usize) {
        let text_left = self.start_x + 2;
        let col = (column as usize)
            .clamp(text_left, text_left + self.inner_width.saturating_sub(1));
        let row = (row as usize).clamp(self.start_y + 1, self.start_y + self.body_h.max(1));
        (self.first_visible + (row - self.start_y - 1), col - text_left)
    }

    /// The resize handle at a screen position: the right border, the
    /// bottom border, or the bottom-right corner.
    pub fn resize_handle_at(&self, column: u16, row: u16) -> Option<ComposerResizeEdge> {
        let (col, row) = (column as usize, row as usize);
        let right = self.start_x + self.box_width - 1;
        let bottom = self.start_y + self.box_height() - 1;
        let on_right = col == right && row >= self.start_y && row <= bottom;
        let on_bottom = row == bottom && col >= self.start_x && col <= right;
        match (on_right, on_bottom) {
            (true, true) => Some(ComposerResizeEdge::Corner),
            (true, false) => Some(ComposerResizeEdge::Right),
            (false, true) => Some(ComposerResizeEdge::Bottom),
            (false, false) => None,
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
    /// A run with an OSC 8 hyperlink was emitted and not yet closed;
    /// the next link-less run (or the end of the row) closes it.
    hyperlink_open: std::cell::Cell<bool>,
    /// Transient message shown over the status bar (command prompt results)
    status_message: Option<(String, std::time::Instant)>,
}

/// How long a transient status message stays visible.
const STATUS_MESSAGE_TTL: std::time::Duration = std::time::Duration::from_secs(3);

/// Optional UI layer rendered over the window-manager scene.
///
/// A single overlay value keeps rendering priority explicit and guarantees
/// that the base scene and its overlay are emitted in one synchronized frame.
pub enum WmOverlay<'a> {
    History(&'a HistorySelector),
    PaneNumbers,
    CopyMode(&'a CopyMode),
    Rename {
        buffer: &'a str,
        target: super::app_state::RenameTarget,
    },
    ThemeSelector {
        themes: &'a [&'a str],
        selected: usize,
    },
    WindowSelector(&'a WindowSelector),
    AgentDashboard(&'a AgentDashboard),
    ContextMenu(&'a ContextMenu),
    /// tmux-style command prompt (`Prefix + :`) on the status bar row
    CommandPrompt(&'a str),
    /// Floating popup pane (display-popup)
    Popup(&'a Pane),
    /// Floating message composer (`Prefix + m`)
    MessageComposer(&'a MessageComposer),
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
            status_message: None,
            hyperlink_open: std::cell::Cell::new(false),
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
            status_message: None,
            hyperlink_open: std::cell::Cell::new(false),
        }
    }

    /// Show a transient message over the status bar (auto-hides after a few
    /// seconds).
    pub fn set_status_message(&mut self, message: String) {
        self.status_message = Some((message, std::time::Instant::now()));
    }

    /// Expire a stale status message. Returns true when it was just cleared
    /// (the caller should re-render to erase it).
    pub fn tick_status_message(&mut self) -> bool {
        if self
            .status_message
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed() >= STATUS_MESSAGE_TTL)
        {
            self.status_message = None;
            return true;
        }
        false
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
        self.render_scene(wm, None)
    }

    fn invalidate_cursor_cache(&mut self) {
        self.cursor.invalidate();
    }

    /// Render the base window-manager scene and one optional UI overlay.
    pub fn render_scene(
        &mut self,
        wm: &WindowManager,
        overlay: Option<WmOverlay<'_>>,
    ) -> io::Result<()> {
        if !self.initialized {
            self.init()?;
        }

        let stdout = io::stdout();
        let mut out = io::BufWriter::with_capacity(65536, stdout.lock());

        let result = with_frame(&mut out, |out| {
            // Keep the real cursor hidden while repainting; otherwise hosts
            // that render mid-frame show it hopping through whichever pane is
            // being redrawn. It is re-shown at its final position below, in
            // the same buffered write.
            queue!(out, Hide)?;
            self.cursor.note_hidden();

            // Text-cursor position inside the message composer, if open
            let mut composer_cursor: Option<(u16, u16)> = None;

            if let Some(WmOverlay::CopyMode(copy_mode)) = overlay.as_ref() {
                self.render_pane_with_copy_mode(out, wm, copy_mode)?;
                self.render_copy_mode_status(out, wm, copy_mode)?;
                self.position_copy_mode_cursor(out, wm, copy_mode)?;
            } else {
                self.render_tab_bar(out, wm)?;
                self.render_panes(out, wm)?;
                self.render_status_bar(out, wm)?;

                match overlay.as_ref() {
                    Some(WmOverlay::History(selector)) => {
                        self.render_selector(out, wm, selector)?;
                    }
                    Some(WmOverlay::PaneNumbers) => {
                        self.render_pane_numbers(out, wm)?;
                    }
                    Some(WmOverlay::Rename { buffer, target }) => {
                        let title = match target {
                            super::app_state::RenameTarget::Window => "Rename Window",
                            super::app_state::RenameTarget::Pane => "Rename Pane",
                        };
                        self.render_rename_popup(out, wm, buffer, title)?;
                    }
                    Some(WmOverlay::ThemeSelector { themes, selected }) => {
                        self.render_theme_selector(out, wm, themes, *selected)?;
                    }
                    Some(WmOverlay::WindowSelector(selector)) => {
                        self.render_window_selector(out, wm, selector)?;
                    }
                    Some(WmOverlay::AgentDashboard(dashboard)) => {
                        self.render_agent_dashboard(out, wm, dashboard)?;
                    }
                    Some(WmOverlay::ContextMenu(menu)) => {
                        self.render_context_menu(out, menu)?;
                    }
                    Some(WmOverlay::CommandPrompt(buffer)) => {
                        self.render_command_prompt(out, wm, buffer)?;
                    }
                    Some(WmOverlay::Popup(pane)) => {
                        self.render_popup(out, pane)?;
                    }
                    Some(WmOverlay::MessageComposer(composer)) => {
                        composer_cursor = self.render_message_composer(out, wm, composer)?;
                    }
                    Some(WmOverlay::CopyMode(_)) | None => {}
                }

                if let Some(message) = self.active_status_message() {
                    if !matches!(overlay, Some(WmOverlay::CommandPrompt(_))) {
                        self.render_status_message(out, wm, &message)?;
                    }
                }
            }

            match overlay.as_ref() {
                None => self.cursor.show_focused_pane_cursor(out, wm)?,
                Some(WmOverlay::History(_)) => {}
                // Park the real cursor at the composer's input position so
                // IMEs anchor their inline preedit window there
                Some(WmOverlay::MessageComposer(_)) => {
                    if let Some((x, y)) = composer_cursor {
                        queue!(out, MoveTo(x, y), Show)?;
                    }
                }
                Some(WmOverlay::Popup(pane)) => self.position_popup_cursor(out, pane)?,
                Some(_) => queue!(out, Show)?,
            }
            Ok(())
        });

        if overlay.is_some() {
            self.invalidate_cursor_cache();
        }

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

    /// Fast update for copy mode - only update cursor and status
    pub fn render_copy_mode_cursor_only(&mut self, wm: &WindowManager, copy_mode: &CopyMode) -> io::Result<()> {
        let stdout = io::stdout();
        let mut out = stdout.lock();

        let result = with_cursor_hidden(&mut out, |out| {
            self.render_copy_mode_status(out, wm, copy_mode)?;
            self.position_copy_mode_cursor(out, wm, copy_mode)?;
            Ok(())
        });
        self.invalidate_cursor_cache();
        result
    }

    fn position_copy_mode_cursor<W: Write>(
        &self,
        stdout: &mut W,
        wm: &WindowManager,
        copy_mode: &CopyMode,
    ) -> io::Result<()> {
        if let Some(tab) = wm.active_tab() {
            if let Some(pane) = tab.focused_pane() {
                if let Some(visible_row) =
                    copy_mode.absolute_to_visible(copy_mode.cursor_row, wm)
                {
                    let (inner_x, inner_y) = pane.inner_pos();
                    let cursor_x = inner_x
                        + copy_mode
                            .cursor_col
                            .min(pane.session.state.cols.saturating_sub(1));
                    execute!(
                        stdout,
                        MoveTo(cursor_x, wm.tab_bar_height + inner_y + visible_row)
                    )?;
                }
            }
        }
        Ok(())
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

            // Erase before painting so overpainting shifted wide glyphs can't
            // bisect a double-width char in the host buffer (see render_pane).
            write!(stdout, "\x1b[0m\x1b[{}X", inner_w)?;

            if let Some(line) = screen.get_line_at_absolute(abs_row) {
                let render_row = RenderRow::with_origin(line, render_w, inner_x, screen_y);
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

    /// Render rename popup in center of screen
    fn render_rename_popup<W: Write>(
        &self,
        stdout: &mut W,
        wm: &WindowManager,
        rename_buffer: &str,
        title: &str,
    ) -> io::Result<()> {
        let box_width = 40.min(wm.width.saturating_sub(4)) as usize;
        let box_height = 5;
        if box_width < 8 || wm.height < box_height as u16 {
            return Ok(());
        }
        let start_x = ((wm.width as usize).saturating_sub(box_width)) / 2;
        let start_y = ((wm.height as usize).saturating_sub(box_height)) / 2;

        // Draw box
        execute!(stdout, 
            SetBackgroundColor(CtColor::DarkBlue),
            SetForegroundColor(CtColor::White)
        )?;

        // Top border
        execute!(stdout, MoveTo(start_x as u16, start_y as u16))?;
        write!(stdout, "┌─ {} ", title)?;
        let title_len = title.chars().count() + 3; // "─ title " after the corner
        for _ in 0..(box_width.saturating_sub(title_len + 2)) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┐")?;

        // Empty line
        execute!(stdout, MoveTo(start_x as u16, (start_y + 1) as u16))?;
        write!(stdout, "│{:width$}│", "", width = box_width - 2)?;

        // Input line
        execute!(stdout, MoveTo(start_x as u16, (start_y + 2) as u16))?;
        let input_display =
            truncate_tail_to_display_width(rename_buffer, box_width.saturating_sub(5));
        let padding = box_width
            .saturating_sub(5)
            .saturating_sub(str_display_width(&input_display));
        write!(
            stdout,
            "│ {}█{:padding$} │",
            input_display,
            "",
            padding = padding
        )?;

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

    /// The status message if it has not expired yet (cloned so callers can
    /// keep using `&mut self`).
    fn active_status_message(&self) -> Option<String> {
        self.status_message
            .as_ref()
            .filter(|(_, at)| at.elapsed() < STATUS_MESSAGE_TTL)
            .map(|(message, _)| message.clone())
    }

    /// Render a transient message over the status bar row.
    fn render_status_message<W: Write>(
        &self,
        stdout: &mut W,
        wm: &WindowManager,
        message: &str,
    ) -> io::Result<()> {
        let cs = &self.color_scheme;
        let row = wm.height.saturating_sub(1);
        let width = wm.width as usize;
        execute!(
            stdout,
            MoveTo(0, row),
            SetBackgroundColor(cs.status_prefix_bg.to_crossterm()),
            SetForegroundColor(cs.status_prefix_fg.to_crossterm())
        )?;
        let text = truncate_to_display_width(message, width);
        let padding = width.saturating_sub(str_display_width(&text));
        write!(stdout, "{}{:padding$}", text, "", padding = padding)?;
        execute!(stdout, ResetColor)?;
        Ok(())
    }

    /// Render the command prompt (`Prefix + :`) on the status bar row.
    fn render_command_prompt<W: Write>(
        &self,
        stdout: &mut W,
        wm: &WindowManager,
        buffer: &str,
    ) -> io::Result<()> {
        let cs = &self.color_scheme;
        let row = wm.height.saturating_sub(1);
        let width = wm.width as usize;
        execute!(
            stdout,
            MoveTo(0, row),
            SetBackgroundColor(cs.status_bar_bg.to_crossterm()),
            SetForegroundColor(cs.status_bar_fg.to_crossterm())
        )?;
        // Keep the tail visible while typing long commands
        let input = truncate_tail_to_display_width(buffer, width.saturating_sub(2));
        let text = format!(":{}", input);
        let padding = width.saturating_sub(str_display_width(&text));
        write!(stdout, "{}{:padding$}", text, "", padding = padding)?;
        // Leave the real cursor at the input position; render_scene shows it
        execute!(
            stdout,
            MoveTo(str_display_width(&text) as u16, row),
            ResetColor
        )?;
        Ok(())
    }

    /// Render the floating popup pane over the scene.
    fn render_popup<W: Write>(&self, stdout: &mut W, pane: &Pane) -> io::Result<()> {
        // Popup panes float above other content, so always paint every row
        self.render_pane(stdout, pane, 0, true)
    }

    /// Place the terminal cursor at the popup's cursor position.
    fn position_popup_cursor<W: Write>(&self, stdout: &mut W, pane: &Pane) -> io::Result<()> {
        let cursor = pane.session.state.active_cursor();
        if cursor.visible {
            let (inner_x, inner_y) = pane.inner_pos();
            let (inner_w, inner_h) = pane.inner_size();
            let col = cursor.col.min(inner_w.saturating_sub(1));
            let row = cursor.row.min(inner_h.saturating_sub(1));
            queue!(stdout, MoveTo(inner_x + col, inner_y + row), Show)?;
        }
        Ok(())
    }

    /// Compute the message composer overlay geometry, or None when the
    /// terminal is too small to show it. Shared by rendering and mouse
    /// hit-testing so the two can never disagree.
    pub fn message_composer_layout(
        &self,
        wm: &WindowManager,
        composer: &MessageComposer,
    ) -> Option<MessageComposerLayout> {
        composer.target.as_ref()?;
        if wm.width < 30 || wm.height < 8 {
            return None;
        }
        let content_top = wm.tab_bar_height as usize;
        let content_height = (wm.height as usize).saturating_sub(content_top + 1);
        let max_width = (wm.width as usize).saturating_sub(4);
        // Chrome: top border, separator, help line, bottom border
        let max_body = content_height.saturating_sub(4);
        if max_width < 24 || max_body < 1 {
            return None;
        }
        // Default size reads as multi-line from the start (8 rows); a border
        // drag overrides it, clamped to what fits on screen
        let (box_width, body_h) = match composer.custom_size {
            Some((w, h)) => (w.clamp(24, max_width), h.clamp(1, max_body)),
            None => (max_width.min(70), max_body.min(8)),
        };
        let inner_width = box_width.saturating_sub(4);
        // Long lines soft-wrap onto extra display rows instead of truncating;
        // scroll so the cursor's display row stays visible
        let rows = composer.wrapped_rows(inner_width);
        let (cursor_display_row, _) = composer.cursor_display_pos(&rows);
        let first_visible = cursor_display_row
            .saturating_add(1)
            .saturating_sub(body_h)
            .min(rows.len().saturating_sub(body_h));
        let box_height = body_h + 4;
        let start_x = (wm.width as usize - box_width) / 2;
        let start_y = content_top + (content_height - box_height) / 2;
        Some(MessageComposerLayout {
            start_x,
            start_y,
            box_width,
            body_h,
            inner_width,
            first_visible,
        })
    }

    /// Render the floating message composer (`Prefix + m`): a small
    /// multi-line editor addressed to one pane.
    ///
    /// Returns the screen position of the text cursor so `render_scene` can
    /// park the real terminal cursor there — IMEs anchor their inline
    /// preedit/candidate window to it.
    fn render_message_composer<W: Write>(
        &self,
        stdout: &mut W,
        wm: &WindowManager,
        composer: &MessageComposer,
    ) -> io::Result<Option<(u16, u16)>> {
        let Some(target) = composer.target.as_ref() else {
            return Ok(None);
        };
        let Some(layout) = self.message_composer_layout(wm, composer) else {
            return Ok(None);
        };
        let MessageComposerLayout {
            start_x,
            start_y,
            box_width,
            body_h,
            inner_width,
            first_visible,
        } = layout;

        let cs = &self.color_scheme;
        let rows = composer.wrapped_rows(inner_width);
        let (cursor_display_row, cursor_offset) = composer.cursor_display_pos(&rows);

        execute!(
            stdout,
            SetBackgroundColor(cs.selector_bg.to_crossterm()),
            SetForegroundColor(cs.selector_fg.to_crossterm())
        )?;

        // Top border with the destination pane in the title, plus the
        // history position while browsing with Ctrl+P/N
        let title = match composer.history_position() {
            Some((index, total)) => {
                format!("Send to {} [{}/{}]", target.label, index + 1, total)
            }
            None => format!("Send to {}", target.label),
        };
        let title = truncate_to_display_width(&title, box_width.saturating_sub(5));
        let title_width = str_display_width(&title);
        execute!(stdout, MoveTo(start_x as u16, start_y as u16))?;
        write!(stdout, "┌─ {} ", title)?;
        for _ in 0..box_width.saturating_sub(title_width + 5) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┐")?;

        // Editor body: soft-wrapped display rows. The cursor is the real
        // terminal cursor (positioned by the caller), not a drawn glyph, so
        // IME preedit text appears inline at the right spot.
        for row in 0..body_h {
            let index = first_visible + row;
            execute!(
                stdout,
                MoveTo(start_x as u16, (start_y + 1 + row) as u16),
                SetBackgroundColor(cs.selector_bg.to_crossterm()),
                SetForegroundColor(cs.selector_fg.to_crossterm())
            )?;
            write!(stdout, "│ ")?;

            let mut display_width = 0;
            let mut selection_color = false;
            if let Some(wrow) = rows.get(index) {
                if let Some(line) = composer.lines.get(wrow.line) {
                    for (offset, ch) in line
                        .chars()
                        .skip(wrow.start)
                        .take(wrow.end - wrow.start)
                        .enumerate()
                    {
                        let selected = composer.is_selected(wrow.line, wrow.start + offset);
                        if selected != selection_color {
                            if selected {
                                execute!(
                                    stdout,
                                    SetBackgroundColor(cs.selector_selected_bg.to_crossterm()),
                                    SetForegroundColor(cs.selector_selected_fg.to_crossterm())
                                )?;
                            } else {
                                execute!(
                                    stdout,
                                    SetBackgroundColor(cs.selector_bg.to_crossterm()),
                                    SetForegroundColor(cs.selector_fg.to_crossterm())
                                )?;
                            }
                            selection_color = selected;
                        }
                        write!(stdout, "{ch}")?;
                        display_width += char_width(ch);
                    }

                    // Represent a selected line break with one highlighted
                    // space so Shift+Left/Right across lines is visible.
                    if wrow.end == line.chars().count()
                        && wrow.line + 1 < composer.lines.len()
                        && composer.is_selected(wrow.line, wrow.end)
                        && display_width < inner_width
                    {
                        if !selection_color {
                            execute!(
                                stdout,
                                SetBackgroundColor(cs.selector_selected_bg.to_crossterm()),
                                SetForegroundColor(cs.selector_selected_fg.to_crossterm())
                            )?;
                            selection_color = true;
                        }
                        write!(stdout, " ")?;
                        display_width += 1;
                    }
                }
            }
            if selection_color {
                execute!(
                    stdout,
                    SetBackgroundColor(cs.selector_bg.to_crossterm()),
                    SetForegroundColor(cs.selector_fg.to_crossterm())
                )?;
            }
            let padding = inner_width.saturating_sub(display_width);
            write!(stdout, "{:padding$} │", "", padding = padding)?;
        }

        // Screen position of the text cursor within the box
        let cursor_left: String = rows
            .get(cursor_display_row)
            .and_then(|wrow| {
                composer.lines.get(wrow.line).map(|line| {
                    line.chars().skip(wrow.start).take(cursor_offset).collect()
                })
            })
            .unwrap_or_default();
        let cursor_x = start_x + 2 + str_display_width(&cursor_left).min(inner_width);
        let cursor_y = start_y
            + 1
            + cursor_display_row
                .saturating_sub(first_visible)
                .min(body_h - 1);

        // Separator, help line, bottom border
        let separator_y = start_y + 1 + body_h;
        execute!(stdout, MoveTo(start_x as u16, separator_y as u16))?;
        write!(stdout, "├")?;
        for _ in 0..box_width.saturating_sub(2) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┤")?;

        // Help on the left, line/char counter on the right
        let total_chars: usize = composer
            .lines
            .iter()
            .map(|l| l.chars().count())
            .sum::<usize>()
            + composer.lines.len().saturating_sub(1);
        let counter = format!("{}L {}C", composer.lines.len(), total_chars);
        let counter_width = str_display_width(&counter);
        let inner = box_width.saturating_sub(4);
        let help = "C-Enter:Send Enter:NL C-z:Undo C-c/x/v:Clipboard Esc:Close";
        let help = truncate_to_display_width(help, inner.saturating_sub(counter_width + 1));
        let help_width = str_display_width(&help);
        execute!(stdout, MoveTo(start_x as u16, (separator_y + 1) as u16))?;
        write!(
            stdout,
            "│ {help}{:pad$}{counter} │",
            "",
            pad = inner.saturating_sub(help_width + counter_width)
        )?;

        execute!(stdout, MoveTo(start_x as u16, (separator_y + 2) as u16))?;
        write!(stdout, "└")?;
        for _ in 0..box_width.saturating_sub(2) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┘")?;

        execute!(stdout, ResetColor)?;
        Ok(Some((cursor_x as u16, cursor_y as u16)))
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

    /// Compute the window selector overlay geometry, or None when the
    /// terminal is too small to show it. Shared by rendering and mouse
    /// hit-testing so the two can never disagree.
    pub fn window_selector_layout(
        &self,
        wm: &WindowManager,
        entry_count: usize,
        selected: usize,
    ) -> Option<WindowSelectorLayout> {
        if entry_count == 0 || wm.width < 20 || wm.height < 8 {
            return None;
        }
        let selected = selected.min(entry_count - 1);

        // Overlay fills the content area (below tab bar, above status bar)
        // with a one-cell margin, tmux choose-tree style: window list on
        // top, live preview of the selected window below.
        let content_top = wm.tab_bar_height as usize;
        let content_height = (wm.height as usize).saturating_sub(content_top + 1);
        let box_width = (wm.width as usize).saturating_sub(4);
        let mut box_height = content_height.saturating_sub(2);
        if box_width < 16 || box_height < 5 {
            return None;
        }

        // Chrome rows: top border, separator(s), help line, bottom border.
        // With preview:    top + list + preview-sep + preview + sep + help + bottom
        // Without preview: top + list + sep + help + bottom
        const MIN_PREVIEW: usize = 3;
        let (list_h, preview_h) = if box_height >= 5 + 1 + MIN_PREVIEW {
            let max_list = box_height - 5 - MIN_PREVIEW;
            let list_h = entry_count.min(max_list).max(1);
            (list_h, box_height - 5 - list_h)
        } else {
            box_height = (entry_count + 4).min(box_height);
            (box_height - 4, 0)
        };
        let first_visible = selected
            .saturating_add(1)
            .saturating_sub(list_h)
            .min(entry_count.saturating_sub(list_h));
        let start_x = (wm.width as usize - box_width) / 2;
        let start_y = content_top + (content_height - box_height) / 2;

        Some(WindowSelectorLayout {
            start_x,
            start_y,
            box_width,
            box_height,
            list_h,
            preview_h,
            first_visible,
        })
    }

    fn render_window_selector<W: Write>(
        &self,
        stdout: &mut W,
        wm: &WindowManager,
        selector: &WindowSelector,
    ) -> io::Result<()> {
        let windows = wm.window_info();
        let entries = selector.entries(&windows);
        let Some(layout) =
            self.window_selector_layout(wm, entries.len(), selector.selected)
        else {
            return Ok(());
        };
        let WindowSelectorLayout {
            start_x,
            start_y,
            box_width,
            box_height: _,
            list_h,
            preview_h,
            first_visible,
        } = layout;

        let cs = &self.color_scheme;
        let selected = selector.selected.min(entries.len() - 1);

        let selector_colors = |stdout: &mut W| {
            execute!(
                stdout,
                SetBackgroundColor(cs.selector_bg.to_crossterm()),
                SetForegroundColor(cs.selector_fg.to_crossterm())
            )
        };
        selector_colors(stdout)?;

        // Top border with title
        let title = format!("Windows [{}, w]", wm.prefix_key.display_name());
        let title = truncate_to_display_width(&title, box_width.saturating_sub(5));
        let title_width = str_display_width(&title);
        execute!(stdout, MoveTo(start_x as u16, start_y as u16))?;
        write!(stdout, "┌─ {} ", title)?;
        for _ in 0..box_width.saturating_sub(title_width + 5) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┐")?;

        // Tree rows: windows, with pane children under expanded windows
        for (row, entry) in entries
            .iter()
            .enumerate()
            .skip(first_visible)
            .take(list_h)
        {
            let y = start_y + 1 + row - first_visible;
            execute!(stdout, MoveTo(start_x as u16, y as u16))?;
            write!(stdout, "│")?;

            if row == selected {
                execute!(
                    stdout,
                    SetBackgroundColor(cs.selector_selected_bg.to_crossterm()),
                    SetForegroundColor(cs.selector_selected_fg.to_crossterm())
                )?;
            }

            let (prefix, name, flag, full_suffix, compact_suffix) = match *entry {
                TreeEntry::Window { window } => {
                    let info = &windows[window];
                    let mark = if selector.is_expanded(info.id) { '-' } else { '+' };
                    let flag = if info.is_active {
                        '*'
                    } else if info.is_last {
                        '-'
                    } else {
                        ' '
                    };
                    let pane_count = info.panes.len();
                    (
                        format!(" {} {:>2}: ", mark, info.number),
                        info.name.clone(),
                        flag,
                        format!(
                            " ({} pane{})",
                            pane_count,
                            if pane_count == 1 { "" } else { "s" }
                        ),
                        format!(" ({})", pane_count),
                    )
                }
                TreeEntry::Pane { window, pane } => {
                    let pane_info = &windows[window].panes[pane];
                    let branch = if pane + 1 == windows[window].panes.len() {
                        "└─"
                    } else {
                        "├─"
                    };
                    let flag = if pane_info.is_active { '*' } else { ' ' };
                    (
                        format!("      {} {}: ", branch, pane_info.number),
                        pane_info.title.clone(),
                        flag,
                        String::new(),
                        String::new(),
                    )
                }
            };

            let fixed = str_display_width(&prefix) + 3; // borders + flag
            let suffix = if fixed + str_display_width(&full_suffix) + 1 < box_width {
                full_suffix
            } else if fixed + str_display_width(&compact_suffix) + 1 < box_width {
                compact_suffix
            } else {
                String::new()
            };
            let content_width =
                box_width.saturating_sub(fixed + str_display_width(&suffix));
            let display_name = truncate_to_display_width(&name, content_width);

            write!(stdout, "{}{}{}{}", prefix, display_name, flag, suffix)?;
            let used = str_display_width(&prefix)
                + str_display_width(&display_name)
                + 1
                + str_display_width(&suffix);
            write!(
                stdout,
                "{:padding$}",
                "",
                padding = box_width.saturating_sub(used + 2)
            )?;

            selector_colors(stdout)?;
            write!(stdout, "│")?;
        }

        let mut separator_y = start_y + 1 + list_h;

        // Preview of the selected window or pane
        if preview_h > 0 {
            let preview_title = match entries[selected] {
                TreeEntry::Window { window } => format!(
                    "Preview: {}: {}",
                    windows[window].number, windows[window].name
                ),
                TreeEntry::Pane { window, pane } => format!(
                    "Preview: {}: {} - pane {}",
                    windows[window].number,
                    windows[window].name,
                    windows[window].panes[pane].number
                ),
            };
            let preview_title =
                truncate_to_display_width(&preview_title, box_width.saturating_sub(5));
            let preview_title_width = str_display_width(&preview_title);
            execute!(stdout, MoveTo(start_x as u16, separator_y as u16))?;
            write!(stdout, "├─ {} ", preview_title)?;
            for _ in 0..box_width.saturating_sub(preview_title_width + 5) {
                write!(stdout, "─")?;
            }
            write!(stdout, "┤")?;

            // Clear the preview body first, then paint the content clipped
            // to it.
            for row in 0..preview_h {
                let y = separator_y + 1 + row;
                execute!(stdout, MoveTo(start_x as u16, y as u16))?;
                write!(
                    stdout,
                    "│{:width$}│",
                    "",
                    width = box_width.saturating_sub(2)
                )?;
            }
            let (px, py) = (start_x + 1, separator_y + 1);
            let pw = box_width.saturating_sub(2);
            match entries[selected] {
                TreeEntry::Window { window } => {
                    if let Some(tab) = wm.tab_at(window) {
                        self.render_window_preview(stdout, tab, px, py, pw, preview_h)?;
                    }
                }
                TreeEntry::Pane { window, pane } => {
                    if let Some(pane) = wm
                        .tab_at(window)
                        .and_then(|tab| tab.pane_order.get(pane).map(|id| (tab, id)))
                        .and_then(|(tab, id)| tab.panes.get(id))
                    {
                        self.render_pane_region(stdout, pane, px, py, pw, preview_h)?;
                    }
                }
            }
            execute!(stdout, ResetColor, SetAttribute(Attribute::Reset))?;
            selector_colors(stdout)?;

            separator_y += 1 + preview_h;
        }

        // Bottom separator, help line, bottom border
        execute!(stdout, MoveTo(start_x as u16, separator_y as u16))?;
        write!(stdout, "├")?;
        for _ in 0..box_width.saturating_sub(2) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┤")?;

        let help = if selector.kill_confirm {
            match entries[selected] {
                TreeEntry::Window { window } => format!(
                    "Kill window {}: {}? (y/N)",
                    windows[window].number, windows[window].name
                ),
                TreeEntry::Pane { window, pane } => format!(
                    "Kill pane {} of window {}: {}? (y/N)",
                    windows[window].panes[pane].number,
                    windows[window].number,
                    windows[window].name
                ),
            }
        } else {
            "j/k:Move h/l:Fold 1-9:Jump Enter:Select x:Kill q/Esc:Close".to_string()
        };
        let help = truncate_to_display_width(&help, box_width.saturating_sub(3));
        let help_width = str_display_width(&help);
        execute!(stdout, MoveTo(start_x as u16, (separator_y + 1) as u16))?;
        write!(stdout, "│ {}", help)?;
        write!(
            stdout,
            "{:padding$}│",
            "",
            padding = box_width.saturating_sub(help_width + 3)
        )?;

        execute!(stdout, MoveTo(start_x as u16, (separator_y + 2) as u16))?;
        write!(stdout, "└")?;
        for _ in 0..box_width.saturating_sub(2) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┘")?;

        execute!(stdout, ResetColor)?;
        Ok(())
    }

    /// Render the agent dashboard overlay (herdr-style): every pane across
    /// all windows with its agent state, refreshed live while open.
    fn render_agent_dashboard<W: Write>(
        &self,
        stdout: &mut W,
        wm: &WindowManager,
        dashboard: &AgentDashboard,
    ) -> io::Result<()> {
        let entries = wm.agent_overview();
        if entries.is_empty() || wm.width < 30 || wm.height < 8 {
            return Ok(());
        }
        let selected = dashboard.selected.min(entries.len() - 1);

        let cs = &self.color_scheme;
        let content_top = wm.tab_bar_height as usize;
        let content_height = (wm.height as usize).saturating_sub(content_top + 1);

        let box_width = (wm.width as usize).saturating_sub(4).min(64);
        // Chrome: top border, separator, help line, bottom border
        let max_list = content_height.saturating_sub(2 + 4);
        if box_width < 24 || max_list < 1 {
            return Ok(());
        }
        let list_h = entries.len().min(max_list);
        let box_height = list_h + 4;
        let first_visible = selected
            .saturating_add(1)
            .saturating_sub(list_h)
            .min(entries.len().saturating_sub(list_h));
        let start_x = (wm.width as usize - box_width) / 2;
        let start_y = content_top + (content_height - box_height) / 2;

        let base_colors = |stdout: &mut W| {
            execute!(
                stdout,
                SetBackgroundColor(cs.selector_bg.to_crossterm()),
                SetForegroundColor(cs.selector_fg.to_crossterm())
            )
        };
        base_colors(stdout)?;

        // Top border with title
        let title = format!("Agents [{}, g]", wm.prefix_key.display_name());
        let title = truncate_to_display_width(&title, box_width.saturating_sub(5));
        let title_width = str_display_width(&title);
        execute!(stdout, MoveTo(start_x as u16, start_y as u16))?;
        write!(stdout, "┌─ {} ", title)?;
        for _ in 0..box_width.saturating_sub(title_width + 5) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┐")?;

        // One row per pane: "  1:main · 1: Pane 1        ! WORKING "
        const STATE_COL: usize = 10; // "! BLOCKED " right-aligned block
        for (row, entry) in entries.iter().enumerate().skip(first_visible).take(list_h) {
            let y = start_y + 1 + row - first_visible;
            execute!(stdout, MoveTo(start_x as u16, y as u16))?;
            write!(stdout, "│")?;

            let is_selected = row == selected;
            if is_selected {
                execute!(
                    stdout,
                    SetBackgroundColor(cs.selector_selected_bg.to_crossterm()),
                    SetForegroundColor(cs.selector_selected_fg.to_crossterm())
                )?;
            }

            let focus_flag = if entry.is_focused { '*' } else { ' ' };
            let prefix = format!(" {} {}:{} · {}: ", focus_flag, entry.window_number, entry.window_name, entry.pane_number);
            let name_width = box_width.saturating_sub(str_display_width(&prefix) + STATE_COL + 2);
            let title = truncate_to_display_width(&entry.pane_title, name_width);
            let used = str_display_width(&prefix) + str_display_width(&title);
            write!(stdout, "{}{}", prefix, title)?;
            write!(
                stdout,
                "{:padding$}",
                "",
                padding = box_width.saturating_sub(used + STATE_COL + 2)
            )?;

            // State cell, colored unless the row is selected. WORKING rows
            // animate a Nerd Font spinner in place of the spacer.
            let attn = if entry.attention { '!' } else { ' ' };
            if !is_selected {
                let state_color = match entry.state {
                    crate::wm::AgentState::Working => CtColor::Green,
                    crate::wm::AgentState::Blocked => CtColor::Yellow,
                    crate::wm::AgentState::Done => CtColor::Cyan,
                    crate::wm::AgentState::Idle => CtColor::DarkGrey,
                };
                execute!(stdout, SetForegroundColor(state_color))?;
            }
            let spinner = if entry.state == crate::wm::AgentState::Working {
                crate::wm::pane::working_spinner_frame_now()
            } else {
                ' '
            };
            write!(stdout, "{}{}{:<8}", attn, spinner, entry.state.label())?;

            base_colors(stdout)?;
            write!(stdout, "│")?;
        }

        // Separator, help line, bottom border
        let separator_y = start_y + 1 + list_h;
        execute!(stdout, MoveTo(start_x as u16, separator_y as u16))?;
        write!(stdout, "├")?;
        for _ in 0..box_width.saturating_sub(2) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┤")?;

        let help = "j/k:Move Enter:Focus m:Message a:Next-alert q/Esc:Close";
        let help = truncate_to_display_width(help, box_width.saturating_sub(3));
        let help_width = str_display_width(&help);
        execute!(stdout, MoveTo(start_x as u16, (separator_y + 1) as u16))?;
        write!(stdout, "│ {}", help)?;
        write!(
            stdout,
            "{:padding$}│",
            "",
            padding = box_width.saturating_sub(help_width + 3)
        )?;

        execute!(stdout, MoveTo(start_x as u16, (separator_y + 2) as u16))?;
        write!(stdout, "└")?;
        for _ in 0..box_width.saturating_sub(2) {
            write!(stdout, "─")?;
        }
        write!(stdout, "┘")?;

        execute!(stdout, ResetColor)?;
        Ok(())
    }

    /// Paint a live preview of a window's panes clipped to the given region.
    ///
    /// Pane geometry is laid out for the full content area, so panes are
    /// cropped at the right/bottom edges of the preview; the gaps left by
    /// pane borders show through as the selector background.
    fn render_window_preview<W: Write>(
        &self,
        stdout: &mut W,
        tab: &crate::wm::Tab,
        px: usize,
        py: usize,
        pw: usize,
        ph: usize,
    ) -> io::Result<()> {
        let panes: Vec<&Pane> = if let Some(zoomed_id) = tab.zoomed_pane_id() {
            tab.panes.get(&zoomed_id).into_iter().collect()
        } else {
            tab.panes.values().collect()
        };

        for pane in panes {
            let (inner_x, inner_y) = pane.inner_pos();
            let (inner_x, inner_y) = (inner_x as usize, inner_y as usize);
            if inner_x >= pw || inner_y >= ph {
                continue;
            }
            self.render_pane_region(
                stdout,
                pane,
                px + inner_x,
                py + inner_y,
                pw - inner_x,
                ph - inner_y,
            )?;
        }

        execute!(stdout, ResetColor, SetAttribute(Attribute::Reset))?;
        Ok(())
    }

    /// Paint the top-left of a pane's screen at a destination position,
    /// cropped to the given bounds.
    fn render_pane_region<W: Write>(
        &self,
        stdout: &mut W,
        pane: &Pane,
        dest_x: usize,
        dest_y: usize,
        max_w: usize,
        max_h: usize,
    ) -> io::Result<()> {
        let (inner_w, inner_h) = pane.inner_size();
        let visible_w = (inner_w as usize).min(max_w);
        let visible_h = (inner_h as usize).min(max_h);
        let screen = pane.session.state.active_screen();
        let render_width = (pane.session.state.cols as usize).min(visible_w);

        for row_idx in 0..visible_h {
            let x = dest_x as u16;
            let y = (dest_y + row_idx) as u16;
            let row = match screen.get_row_at(row_idx) {
                Some(r) => r,
                None => continue,
            };
            execute!(stdout, MoveTo(x, y))?;
            let render_row = RenderRow::with_origin(&row.cells, render_width, x, y);
            self.render_cells(stdout, render_row, visible_w, |_, cell| {
                RenderRowStyle::Cell {
                    attrs: cell.attrs.clone(),
                    selected: false,
                }
            })?;
        }
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
        for (i, (_id, name, active, attention)) in tabs.iter().enumerate() {
            if *active {
                execute!(stdout,
                    SetBackgroundColor(cs.tab_active_bg.to_crossterm()),
                    SetForegroundColor(cs.tab_active_fg.to_crossterm())
                )?;
            } else if *attention {
                // A pane in this tab needs attention (bell / agent went quiet)
                execute!(stdout,
                    SetBackgroundColor(cs.tab_inactive_bg.to_crossterm()),
                    SetForegroundColor(CtColor::Yellow)
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

        if let Some(range) = wm.new_tab_button_range() {
            execute!(
                stdout,
                MoveTo(range.start, 0),
                SetBackgroundColor(cs.tab_bar_bg.to_crossterm()),
                SetForegroundColor(cs.tab_bar_fg.to_crossterm())
            )?;
            write!(stdout, "[+]")?;
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
                    if needs_full_redraw || !pane.session.is_settling() {
                        self.render_pane(stdout, pane, wm.tab_bar_height, needs_full_redraw)?;
                    }
                }
            }
        } else {
            for pane in tab.panes.values() {
                let screen = pane.session.state.active_screen();
                // Skip panes with no new content unless forced by layout
                // change. Panes settling after a resize are also skipped:
                // ConPTY is replaying the whole buffer into them and painting
                // the intermediate states would show old content scrolling
                // past; their accumulated dirty lines are painted in one
                // frame once the replay finishes.
                let pane_needs_render = needs_full_redraw
                    || ((screen.full_redraw || screen.has_dirty_lines())
                        && !pane.session.is_settling());

                if pane_needs_render {
                    self.render_pane(stdout, pane, wm.tab_bar_height, needs_full_redraw)?;
                }
            }
        }

        Ok(())
    }

    /// Render a single pane
    pub(crate) fn render_pane<W: Write>(&self, stdout: &mut W, pane: &Pane, y_offset: u16, force_full: bool) -> io::Result<()> {
        let screen = pane.session.state.active_screen();
        let (inner_x, inner_y) = pane.inner_pos();
        let (inner_w, inner_h) = pane.inner_size();
        let has_selection = pane.session.state.selection.is_some();
        let session_cols = pane.session.state.cols as usize;

        // Whether to render all rows or only dirty ones
        let full_redraw = force_full || screen.full_redraw;

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

            // Erase the row span before painting (ECH keeps the cursor put
            // and stays inside the pane, unlike EL). Overpainting wide glyphs
            // whose columns shifted since the last frame would otherwise
            // transiently bisect double-width characters in the host's
            // buffer — and ConPTY/conhost resolves every bisection by
            // padding with a space, which is how stray blanks ended up
            // inside repainted CJK text.
            write!(stdout, "\x1b[0m\x1b[{}X", inner_w)?;

            let row = match screen.get_row_at(row_idx) {
                Some(r) => r,
                None => continue,
            };

            let render_row = RenderRow::with_origin(&row.cells, render_width, inner_x, row_y);
            self.render_cells(stdout, render_row, inner_w as usize, |col_idx, cell| RenderRowStyle::Cell {
                attrs: cell.attrs.clone(),
                selected: has_selection
                    && pane.session.state.is_selected(col_idx as u16, row_idx as u16),
            })?;
        }

        // Draw the border after the content so that any residual overflow
        // from host/our width disagreements gets painted over.
        if pane.border != BorderStyle::None {
            self.render_border(stdout, pane, y_offset)?;
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
        let origin = row.origin();
        let rendered_width = render_row_stream(stdout, row, |col_idx, cell| style_for(col_idx, cell), |stdout, style| {
            self.apply_render_row_style(stdout, style)
        })?;

        // Close a hyperlink left open by the row's last run so borders and
        // the cleared tail never become clickable.
        if self.hyperlink_open.get() {
            write!(stdout, "\x1b]8;;\x1b\\")?;
            self.hyperlink_open.set(false);
        }

        if rendered_width < clear_width {
            // Re-anchor before padding: if the host rendered a grapheme wider
            // or narrower than our accounting, the cursor has drifted and the
            // tail would be cleared at the wrong columns.
            if let Some((x, y)) = origin {
                execute!(stdout, MoveTo(x + rendered_width as u16, y))?;
            }
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
        
        execute!(stdout, ResetColor, SetAttribute(Attribute::Reset))?;

        // Border color: focus wins, then attention (agent waiting), then default
        let border_fg = if pane.focused {
            cs.pane_border_active.to_crossterm()
        } else if pane.activity.attention().is_some() {
            CtColor::Yellow
        } else {
            cs.pane_border.to_crossterm()
        };
        execute!(stdout, SetForegroundColor(border_fg))?;

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
        
        // The title uses the same color as the surrounding border, so the
        // focused pane's title picks up the active border color.
        write!(stdout, "{}", display_title)?;
        
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

        execute!(stdout, ResetColor, SetAttribute(Attribute::Reset))?;
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
            r#"c:new x:kill ":split %:vsplit n/p:win z:zoom e:sync a:alert t:theme"#.to_string()
        } else {
            format!("{}: prefix | {}: history", prefix_name, self.history_selector_shortcut)
        };
        
        let left_len = status.len();
        let right_len = shortcuts.len();
        let padding = (wm.width as usize).saturating_sub(left_len + right_len + 2);

        // Clip to the terminal width — on narrow terminals the combined
        // status + shortcuts would otherwise spill past the last column.
        let line = format!(" {}{:padding$}{} ", status, "", shortcuts, padding = padding);
        let line = truncate_to_display_width(&line, wm.width as usize);
        write!(stdout, "{}", line)?;
        
        execute!(stdout, ResetColor)?;
        Ok(())
    }

    /// Apply cell attributes with selection highlighting
    fn apply_attrs_with_selection<W: Write>(&self, stdout: &mut W, attrs: &CellAttrs, selected: bool) -> io::Result<()> {
        let cs = &self.color_scheme;

        // OSC 8 hyperlink state for this run. Re-opening the same target on
        // consecutive runs is harmless; a link-less run closes any open one.
        match &attrs.hyperlink {
            Some(link) => {
                let id = link.id.as_deref().unwrap_or("");
                let params = if id.is_empty() { String::new() } else { format!("id={id}") };
                write!(stdout, "\x1b]8;{};{}\x1b\\", params, link.uri)?;
                self.hyperlink_open.set(true);
            }
            None if self.hyperlink_open.get() => {
                write!(stdout, "\x1b]8;;\x1b\\")?;
                self.hyperlink_open.set(false);
            }
            None => {}
        }

        // Batch all SGR codes into a single escape sequence for efficiency.
        // Instead of multiple execute!() calls (each a separate write), we build
        // one \x1b[...m sequence with semicolon-separated parameters.
        let mut sgr = String::with_capacity(48);
        sgr.push_str("\x1b[0"); // Always start with reset

        // Text attributes
        if attrs.flags.contains(AttrFlags::BOLD) && !self.suppress_bold { sgr.push_str(";1"); }
        if attrs.flags.contains(AttrFlags::DIM)       { sgr.push_str(";2"); }
        if attrs.flags.contains(AttrFlags::ITALIC)    { sgr.push_str(";3"); }
        if attrs.flags.contains(AttrFlags::UNDERLINE) {
            use crate::core::term::UnderlineStyle;
            match attrs.underline_style {
                UnderlineStyle::Single => sgr.push_str(";4"),
                UnderlineStyle::Double => sgr.push_str(";4:2"),
                UnderlineStyle::Curly => sgr.push_str(";4:3"),
                UnderlineStyle::Dotted => sgr.push_str(";4:4"),
                UnderlineStyle::Dashed => sgr.push_str(";4:5"),
            }
        }
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

        // write! into the String (fmt::Write) formats in place — no per-cell
        // String allocations on this hot path.  Writing to a String is
        // infallible, so the results can be ignored.
        use std::fmt::Write as _;
        if selected {
            let _ = write!(sgr, ";38;2;{};{};{}", cs.selection_fg.r, cs.selection_fg.g, cs.selection_fg.b);
            let _ = write!(sgr, ";48;2;{};{};{}", cs.selection_bg.r, cs.selection_bg.g, cs.selection_bg.b);
        } else {
            match attrs.fg {
                Color::Default => {}
                Color::Indexed(idx) => {
                    if idx < 8        { let _ = write!(sgr, ";{}", 30 + idx); }
                    else if idx < 16  { let _ = write!(sgr, ";{}", 90 + (idx - 8)); }
                    else              { let _ = write!(sgr, ";38;5;{}", idx); }
                }
                Color::Rgb(r, g, b) => { let _ = write!(sgr, ";38;2;{};{};{}", r, g, b); }
            }
            match attrs.bg {
                Color::Default => {}
                Color::Indexed(idx) => {
                    if idx < 8        { let _ = write!(sgr, ";{}", 40 + idx); }
                    else if idx < 16  { let _ = write!(sgr, ";{}", 100 + (idx - 8)); }
                    else              { let _ = write!(sgr, ";48;5;{}", idx); }
                }
                Color::Rgb(r, g, b) => { let _ = write!(sgr, ";48;2;{};{};{}", r, g, b); }
            }
            match attrs.underline_color {
                Color::Default => {}
                Color::Indexed(idx) => { let _ = write!(sgr, ";58:5:{}", idx); }
                Color::Rgb(r, g, b) => { let _ = write!(sgr, ";58:2::{}:{}:{}", r, g, b); }
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
    use super::{char_width, str_display_width, truncate_to_display_width, WmRenderer};
    use crate::wm::Pane;

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

    #[cfg(windows)]
    #[test]
    fn rename_popup_truncates_cjk_at_character_boundaries() {
        use crate::config::PrefixKey;
        use crate::wm::WindowManager;

        let wm = WindowManager::new(24, 10, None, None, PrefixKey { char: 'b' }, true);
        let renderer = WmRenderer::new();
        let mut out = Vec::new();

        renderer
            .render_rename_popup(&mut out, &wm, "とても長いウィンドウ名", "Rename Window")
            .expect("render rename popup");

        let output = String::from_utf8(out).expect("valid UTF-8 output");
        assert!(output.contains("いウィンドウ名█"));
    }

    #[cfg(windows)]
    #[test]
    fn rename_popup_shows_pane_title_variant() {
        use crate::config::PrefixKey;
        use crate::wm::WindowManager;

        let wm = WindowManager::new(60, 10, None, None, PrefixKey { char: 'b' }, true);
        let renderer = WmRenderer::new();
        let mut out = Vec::new();

        renderer
            .render_rename_popup(&mut out, &wm, "agent", "Rename Pane")
            .expect("render rename popup");

        let output = String::from_utf8(out).expect("valid UTF-8 output");
        assert!(output.contains("Rename Pane"));
        assert!(output.contains("agent█"));
    }

    #[cfg(windows)]
    #[test]
    fn window_selector_overlay_lays_out_list_and_preview() {
        use crate::config::PrefixKey;
        use crate::core::session::Session;
        use crate::ui::window_selector::WindowSelector;
        use crate::wm::WindowManager;

        // Replay emitted bytes through a simulated host terminal so the
        // assertions run against what the user's terminal grid would show.
        fn replay(out: &[u8]) -> Vec<String> {
            let mut host = Session::new(9, 80, 24);
            host.feed_bytes(out);
            host.state
                .active_screen()
                .rows
                .iter()
                .map(|row| {
                    row.cells
                        .iter()
                        .filter(|c| !c.is_continuation())
                        .map(|c| if c.grapheme.is_empty() { " " } else { c.grapheme.as_str() })
                        .collect()
                })
                .collect()
        }

        let mut wm = WindowManager::new(80, 24, None, None, PrefixKey { char: 'b' }, true);
        wm.new_tab();
        let renderer = WmRenderer::new();
        let mut selector = WindowSelector::new();
        selector.open(&wm);

        let mut out = Vec::new();
        renderer
            .render_window_selector(&mut out, &wm, &selector)
            .expect("render window selector");
        let grid = replay(&out);

        let has_line = |needle: &str| grid.iter().any(|l| l.contains(needle));
        assert!(has_line("Windows [Ctrl+B, w]"), "title missing:\n{}", grid.join("\n"));
        assert!(has_line("+  1: 1:main- (1 pane)"), "last-window row missing:\n{}", grid.join("\n"));
        assert!(has_line("+  2: 2:shell* (1 pane)"), "active-window row missing:\n{}", grid.join("\n"));
        assert!(has_line("Preview: 2: 2:shell"), "preview separator missing:\n{}", grid.join("\n"));
        assert!(has_line("Enter:Select"), "help line missing:\n{}", grid.join("\n"));

        // Every overlay row must keep the box edges aligned.
        let left = grid
            .iter()
            .find(|l| l.contains('┌'))
            .and_then(|l| l.find('┌'))
            .expect("top border");
        let width = grid.iter().find(|l| l.contains('┌')).unwrap().trim_end().chars().count();
        for line in grid.iter().filter(|l| l.contains('│') || l.contains('├')) {
            assert_eq!(
                line.trim_end().chars().count(),
                width,
                "misaligned overlay row:\n{}",
                grid.join("\n")
            );
        }
        assert!(left > 0);

        // Expanding the selected window lists its panes as child rows, and
        // selecting a pane row previews that pane.
        let windows = wm.window_info();
        selector.expand(&windows);
        selector.selected += 1; // the pane row under window 2
        let mut out = Vec::new();
        renderer
            .render_window_selector(&mut out, &wm, &selector)
            .expect("render expanded selector");
        let tree_grid = replay(&out);
        assert!(
            tree_grid.iter().any(|l| l.contains("-  2: 2:shell* (1 pane)")),
            "expanded-window marker missing:\n{}",
            tree_grid.join("\n")
        );
        assert!(
            tree_grid.iter().any(|l| l.contains("└─ 1: Pane 1*")),
            "pane child row missing:\n{}",
            tree_grid.join("\n")
        );
        assert!(
            tree_grid.iter().any(|l| l.contains("Preview: 2: 2:shell - pane 1")),
            "pane preview title missing:\n{}",
            tree_grid.join("\n")
        );

        // Kill confirmation replaces the help line (window and pane rows).
        selector.kill_confirm = true;
        let mut out = Vec::new();
        renderer
            .render_window_selector(&mut out, &wm, &selector)
            .expect("render pane kill confirm");
        let confirm_grid = replay(&out);
        assert!(
            confirm_grid
                .iter()
                .any(|l| l.contains("Kill pane 1 of window 2: 2:shell? (y/N)")),
            "pane kill confirmation missing:\n{}",
            confirm_grid.join("\n")
        );

        selector.selected = 0;
        let mut out = Vec::new();
        renderer
            .render_window_selector(&mut out, &wm, &selector)
            .expect("render window kill confirm");
        let confirm_grid = replay(&out);
        assert!(
            confirm_grid.iter().any(|l| l.contains("Kill window 1: 1:main? (y/N)")),
            "window kill confirmation missing:\n{}",
            confirm_grid.join("\n")
        );
    }

    #[test]
    fn window_selector_layout_maps_mouse_positions_to_list_rows() {
        use crate::config::PrefixKey;
        use crate::wm::WindowManager;

        let wm = WindowManager::new(80, 24, None, None, PrefixKey { char: 'b' }, true);
        let renderer = WmRenderer::new();
        let layout = renderer
            .window_selector_layout(&wm, 2, 0)
            .expect("layout for 80x24");

        // 80x24, tab bar 1, status bar 1 → box at x 2..78, y 2..22 with the
        // two list rows directly under the top border.
        assert_eq!(layout.list_row_at(2, 2, 3), Some(0));
        assert_eq!(layout.list_row_at(2, 40, 4), Some(1));
        assert_eq!(layout.list_row_at(2, 40, 5), None); // preview separator
        assert_eq!(layout.list_row_at(2, 40, 2), None); // top border
        assert_eq!(layout.list_row_at(2, 1, 3), None); // left of the box

        assert!(layout.contains(2, 2));
        assert!(layout.contains(77, 21));
        assert!(!layout.contains(78, 21));
        assert!(!layout.contains(40, 1));

        // Too-small terminals produce no layout (selector is not drawn)
        let tiny = WindowManager::new(15, 6, None, None, PrefixKey { char: 'b' }, true);
        assert!(renderer.window_selector_layout(&tiny, 2, 0).is_none());
    }

    #[test]
    fn message_composer_layout_maps_mouse_and_clamps_custom_size() {
        use super::ComposerResizeEdge as Edge;
        use crate::config::PrefixKey;
        use crate::ui::message_composer::MessageComposer;
        use crate::wm::WindowManager;

        let wm = WindowManager::new(80, 24, None, None, PrefixKey { char: 'b' }, true);
        let renderer = WmRenderer::new();
        let mut composer = MessageComposer::new();
        // Closed composer has no layout
        assert!(renderer.message_composer_layout(&wm, &composer).is_none());
        composer.open(1, 1, "1:main · 1: Pane 1".to_string());

        // 80x24, tab bar 1, status bar 1 → 70x12 box at x 5..75, y 6..18;
        // body rows 7..15, text columns 7..73
        let layout = renderer
            .message_composer_layout(&wm, &composer)
            .expect("layout for 80x24");
        assert_eq!(layout.body_pos_at(7, 7), Some((0, 0)));
        assert_eq!(layout.body_pos_at(72, 14), Some((7, 65)));
        assert_eq!(layout.body_pos_at(7, 6), None); // top border
        assert_eq!(layout.body_pos_at(6, 7), None); // left border
        assert_eq!(layout.body_pos_at(7, 15), None); // separator

        // Dragging outside the body clamps back onto it
        assert_eq!(layout.body_pos_clamped(0, 0), (0, 0));
        assert_eq!(layout.body_pos_clamped(79, 23), (7, 65));

        assert_eq!(layout.resize_handle_at(74, 10), Some(Edge::Right));
        assert_eq!(layout.resize_handle_at(40, 17), Some(Edge::Bottom));
        assert_eq!(layout.resize_handle_at(74, 17), Some(Edge::Corner));
        assert_eq!(layout.resize_handle_at(5, 10), None); // left border

        // Dragged sizes are clamped to what fits on screen
        composer.custom_size = Some((200, 100));
        let big = renderer.message_composer_layout(&wm, &composer).unwrap();
        assert_eq!((big.box_width, big.body_h), (76, 18));
        composer.custom_size = Some((5, 0));
        let small = renderer.message_composer_layout(&wm, &composer).unwrap();
        assert_eq!((small.box_width, small.body_h), (24, 1));

        // Too-small terminals produce no layout (composer is not drawn)
        let tiny = WindowManager::new(20, 6, None, None, PrefixKey { char: 'b' }, true);
        assert!(renderer.message_composer_layout(&tiny, &composer).is_none());
    }

    #[cfg(windows)]
    #[test]
    fn message_composer_overlay_shows_help_and_counter() {
        use crate::config::PrefixKey;
        use crate::core::session::Session;
        use crate::ui::message_composer::MessageComposer;
        use crate::wm::WindowManager;

        // Replay emitted bytes through a simulated host terminal so the
        // assertions run against what the user's terminal grid would show.
        fn replay(out: &[u8]) -> Vec<String> {
            let mut host = Session::new(9, 80, 24);
            host.feed_bytes(out);
            host.state
                .active_screen()
                .rows
                .iter()
                .map(|row| {
                    row.cells
                        .iter()
                        .filter(|c| !c.is_continuation())
                        .map(|c| if c.grapheme.is_empty() { " " } else { c.grapheme.as_str() })
                        .collect()
                })
                .collect()
        }

        let wm = WindowManager::new(80, 24, None, None, PrefixKey { char: 'b' }, true);
        let renderer = WmRenderer::new();
        let mut composer = MessageComposer::new();
        composer.open(1, 1, "1:main · 1: Pane 1".to_string());
        composer.insert_str("hello\nこんにちは");

        let mut out = Vec::new();
        let cursor = renderer
            .render_message_composer(&mut out, &wm, &composer)
            .expect("render composer");
        assert!(cursor.is_some(), "composer must park the text cursor");
        let grid = replay(&out);

        let has_line = |needle: &str| grid.iter().any(|l| l.contains(needle));
        assert!(has_line("Send to 1:main"), "title missing:\n{}", grid.join("\n"));
        assert!(has_line("hello"), "body missing:\n{}", grid.join("\n"));
        // 5 + 5 chars joined by one newline
        assert!(has_line("2L 11C"), "counter missing:\n{}", grid.join("\n"));
        assert!(has_line("C-z:Undo"), "help missing:\n{}", grid.join("\n"));

        // Every overlay row must keep the box edges aligned. Compare
        // display widths: the replay drops wide-char continuation cells,
        // so a char count would come up short on rows with CJK text.
        let width = str_display_width(
            grid.iter()
                .find(|l| l.contains('┌'))
                .expect("top border")
                .trim_end(),
        );
        for line in grid.iter().filter(|l| l.contains('│') || l.contains('├')) {
            assert_eq!(
                str_display_width(line.trim_end()),
                width,
                "misaligned overlay row:\n{}",
                grid.join("\n")
            );
        }
    }

    #[test]
    fn render_border_resets_inherited_sgr_before_drawing() {
        let renderer = WmRenderer::new();
        let mut pane = Pane::new(1, 12, 4);
        pane.focused = true;

        let mut out = Vec::new();
        renderer
            .render_border(&mut out, &pane, 0)
            .expect("render border");

        let output = String::from_utf8(out).expect("utf8");
        let border_start = output.find('┌').expect("top-left border");
        let before_border = &output[..border_start];

        assert!(
            before_border.contains("\x1b[0m"),
            "border rendering must reset inherited app colors and attributes before drawing"
        );
    }
}
