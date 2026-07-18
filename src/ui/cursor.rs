use std::io::{self, Write};

use crossterm::{
    cursor::{Hide, MoveTo, Show},
    execute,
};

use crate::wm::WindowManager;

#[derive(Default)]
pub(crate) struct CursorPresenter {
    last_visible: Option<bool>,
    last_shape: Option<u8>,
}

impl CursorPresenter {
    pub(crate) fn invalidate(&mut self) {
        self.last_visible = None;
        self.last_shape = None;
    }

    /// Record that the cursor was just hidden by the caller (e.g. at the start
    /// of a render frame), so the next show request actually re-sends Show.
    pub(crate) fn note_hidden(&mut self) {
        self.last_visible = Some(false);
    }

    pub(crate) fn show_focused_pane_cursor<W: Write>(
        &mut self,
        out: &mut W,
        wm: &WindowManager,
    ) -> io::Result<()> {
        let Some(tab) = wm.active_tab() else {
            return Ok(());
        };
        let Some(pane) = tab.focused_pane() else {
            return Ok(());
        };

        let cursor = pane.session.state.active_cursor();
        if !cursor.visible {
            if self.last_visible != Some(false) {
                execute!(out, Hide)?;
                self.last_visible = Some(false);
            }
            return Ok(());
        }

        let shape_code = cursor.shape.to_decscusr();
        if self.last_shape != Some(shape_code) {
            write!(out, "\x1b[{} q", shape_code)?;
            self.last_shape = Some(shape_code);
        }
        if self.last_visible != Some(true) {
            execute!(out, Show)?;
            self.last_visible = Some(true);
        }

        let (inner_x, inner_y) = pane.inner_pos();
        execute!(
            out,
            MoveTo(inner_x + cursor.col, wm.tab_bar_height + inner_y + cursor.row)
        )?;

        Ok(())
    }
}
