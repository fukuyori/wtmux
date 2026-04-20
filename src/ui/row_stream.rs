use std::io::{self, Write};

use crate::core::term::Cell;

#[derive(Clone)]
pub struct RenderRow<'a> {
    cells: &'a [Cell],
    content_width: usize,
}

impl<'a> RenderRow<'a> {
    pub fn new(cells: &'a [Cell], content_width: usize) -> Self {
        Self { cells, content_width }
    }

    pub fn cells(&self) -> &'a [Cell] {
        self.cells
    }

    pub fn content_width(&self) -> usize {
        self.content_width
    }
}

pub fn render_row_stream<W, S, FStyle, FApply>(
    stdout: &mut W,
    row: RenderRow<'_>,
    mut style_for: FStyle,
    mut apply_style: FApply,
) -> io::Result<usize>
where
    W: Write,
    S: PartialEq,
    FStyle: FnMut(usize, &Cell) -> S,
    FApply: FnMut(&mut W, &S) -> io::Result<()>,
{
    let mut line_buffer = String::with_capacity(256);
    let mut current_style: Option<S> = None;
    let mut run_width = 0usize;
    let mut total_width = 0usize;

    for (col_idx, cell) in row.cells().iter().enumerate() {
        if col_idx >= row.content_width() {
            break;
        }
        if cell.is_continuation() {
            continue;
        }

        let next_style = style_for(col_idx, cell);
        if current_style.as_ref() != Some(&next_style) {
            if let Some(style) = current_style.as_ref() {
                flush_row_run(stdout, &mut line_buffer, style, &mut apply_style)?;
                total_width += run_width;
                run_width = 0;
            }
            current_style = Some(next_style);
        }

        line_buffer.push_str(cell.display_char());
        run_width += cell.width.max(1) as usize;
    }

    if let Some(style) = current_style.as_ref() {
        flush_row_run(stdout, &mut line_buffer, style, &mut apply_style)?;
        total_width += run_width;
    }

    Ok(total_width)
}

fn flush_row_run<W, S, FApply>(
    stdout: &mut W,
    line_buffer: &mut String,
    style: &S,
    apply_style: &mut FApply,
) -> io::Result<()>
where
    W: Write,
    FApply: FnMut(&mut W, &S) -> io::Result<()>,
{
    if line_buffer.is_empty() {
        return Ok(());
    }

    apply_style(stdout, style)?;
    write!(stdout, "{}", line_buffer)?;
    line_buffer.clear();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{render_row_stream, RenderRow};
    use std::io::Write;
    use crate::core::term::{Cell, CellAttrs};

    #[test]
    fn render_row_stream_skips_continuations_and_tracks_display_width() {
        let attrs = CellAttrs::default();
        let cells = vec![
            Cell {
                grapheme: "A".to_string(),
                width: 1,
                attrs: attrs.clone(),
            },
            Cell::continuation(&attrs),
            Cell {
                grapheme: "日".to_string(),
                width: 2,
                attrs,
            },
        ];

        let mut out = Vec::new();
        let rendered_width = render_row_stream(&mut out, RenderRow::new(&cells, 4), |_col_idx, _cell| (), |stdout, _style| {
            write!(stdout, "")
        })
        .expect("row stream renders");

        assert_eq!(rendered_width, 3);
        assert_eq!(String::from_utf8(out).expect("utf8"), "A日");
    }

    #[test]
    fn render_row_stream_flushes_on_style_change() {
        let attrs = CellAttrs::default();
        let cells = vec![
            Cell {
                grapheme: "A".to_string(),
                width: 1,
                attrs: attrs.clone(),
            },
            Cell {
                grapheme: "B".to_string(),
                width: 1,
                attrs: attrs.clone(),
            },
            Cell {
                grapheme: "C".to_string(),
                width: 1,
                attrs,
            },
        ];

        let mut out = Vec::new();
        render_row_stream(&mut out, RenderRow::new(&cells, 3), |col_idx, _cell| col_idx >= 2, |stdout, style| {
            if *style {
                write!(stdout, "[1]")
            } else {
                write!(stdout, "[0]")
            }
        })
        .expect("row stream renders");

        assert_eq!(String::from_utf8(out).expect("utf8"), "[0]AB[1]C");
    }
}
