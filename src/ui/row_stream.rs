use std::io::{self, Write};

use crate::core::term::Cell;

/// Emoji presentation selector (VS16). When appended to a narrow base
/// character it asks the terminal to render a double-width emoji glyph,
/// which would desync our column accounting.
const VS16: char = '\u{FE0F}';

#[derive(Clone)]
pub struct RenderRow<'a> {
    cells: &'a [Cell],
    content_width: usize,
    /// Screen position (x, y) of the row start. When present, the stream
    /// re-anchors the host cursor at every style-run boundary so that any
    /// width disagreement between our cell accounting and the host
    /// terminal's glyph rendering cannot accumulate across the row and
    /// bleed into a neighboring pane.
    origin: Option<(u16, u16)>,
}

impl<'a> RenderRow<'a> {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(cells: &'a [Cell], content_width: usize) -> Self {
        Self { cells, content_width, origin: None }
    }

    pub fn with_origin(cells: &'a [Cell], content_width: usize, x: u16, y: u16) -> Self {
        Self { cells, content_width, origin: Some((x, y)) }
    }

    pub fn cells(&self) -> &'a [Cell] {
        self.cells
    }

    pub fn content_width(&self) -> usize {
        self.content_width
    }

    pub fn origin(&self) -> Option<(u16, u16)> {
        self.origin
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
    let origin = row.origin();
    // Column of the first cell buffered in `line_buffer`. Runs are anchored
    // at this grid column — never at an accumulated width sum — so a grid
    // whose width accounting is inconsistent (e.g. a wide cell orphaned from
    // its continuation by a partial erase) can never push output past the
    // row's clipped right edge.
    let mut run_start_col = 0usize;
    // Column right after the last emitted cell; a gap between this and the
    // next cell's column means the buffered run must be flushed so the next
    // cell re-anchors at its true column.
    let mut next_col = 0usize;
    // End column of the rendered content (for the caller's tail clearing).
    let mut end_col = 0usize;
    // Whether the previously emitted cell was one whose host-rendered width
    // could disagree with our accounting (see `is_risky_cell`). When it was,
    // the following cell must start with a fresh cursor re-anchor so any drift
    // from that glyph cannot shift it.
    let mut prev_risky = false;

    for (col_idx, cell) in row.cells().iter().enumerate() {
        if col_idx >= row.content_width() {
            break;
        }
        if cell.is_continuation() && col_idx < next_col {
            // Second half of a wide char we already emitted.
            continue;
        }
        // An orphaned continuation cell (its wide char was overwritten or
        // erased without repair) renders as a blank so the column is still
        // painted and the run stays contiguous.
        let orphan = cell.is_continuation();

        let cell_width = if orphan { 1 } else { cell.width.max(1) as usize };
        // A wide cell that would cross the right edge must not be emitted:
        // the host would paint its second half past the pane boundary.
        if col_idx + cell_width > row.content_width() {
            break;
        }

        let next_style = style_for(col_idx, cell);
        // Re-anchor within a run (not just at style boundaries) around any
        // glyph whose host-rendered width we can't be sure of. Without origin
        // there is no cursor position to anchor to, so this only applies to
        // positioned rows. This bounds cursor drift to a single cell so a long
        // same-style run of wide glyphs (e.g. a CJK paragraph) on a terminal
        // that miscounts widths (legacy conhost) cannot accumulate drift and
        // bleed past the pane's right edge into a neighboring pane.
        let risky = origin.is_some() && !orphan && is_risky_cell(cell);
        let style_changed = current_style.as_ref() != Some(&next_style);
        let discontinuous = !line_buffer.is_empty() && col_idx != next_col;

        if style_changed || risky || prev_risky || discontinuous {
            if let Some(style) = current_style.as_ref() {
                let pos = origin.map(|(x, y)| (x + run_start_col as u16, y));
                flush_row_run(stdout, &mut line_buffer, style, pos, &mut apply_style)?;
            }
            if style_changed {
                current_style = Some(next_style);
            }
        }

        if line_buffer.is_empty() {
            run_start_col = col_idx;
        }
        if orphan {
            line_buffer.push(' ');
        } else {
            push_grapheme(&mut line_buffer, cell);
        }
        next_col = col_idx + cell_width;
        end_col = next_col;
        prev_risky = risky;
    }

    if let Some(style) = current_style.as_ref() {
        let pos = origin.map(|(x, y)| (x + run_start_col as u16, y));
        flush_row_run(stdout, &mut line_buffer, style, pos, &mut apply_style)?;
    }

    Ok(end_col)
}

/// Whether a cell's host-rendered width might disagree with our accounting.
///
/// A single ASCII printable byte (width 1) is rendered identically by every
/// terminal, so a run of them never drifts and can be batched freely. Anything
/// else — wide glyphs, or multi-byte graphemes (combining marks, emoji, VS16,
/// ZWJ sequences) — is where terminals (notably legacy Windows conhost)
/// disagree on cursor advancement, so we re-anchor around it.
fn is_risky_cell(cell: &Cell) -> bool {
    let grapheme = cell.display_char();
    cell.width != 1 || grapheme.len() != 1
}

/// Append a cell's grapheme, keeping the emitted display width equal to
/// the width we accounted for the cell. A VS16 on a cell we track as
/// narrow would make the host render a double-width emoji glyph, shifting
/// the rest of the row right — strip it so the narrow text-presentation
/// glyph is used instead.
fn push_grapheme(line_buffer: &mut String, cell: &Cell) {
    let grapheme = cell.display_char();
    if cell.width == 1 && grapheme.contains(VS16) {
        line_buffer.extend(grapheme.chars().filter(|&c| c != VS16));
    } else {
        line_buffer.push_str(grapheme);
    }
}

fn flush_row_run<W, S, FApply>(
    stdout: &mut W,
    line_buffer: &mut String,
    style: &S,
    pos: Option<(u16, u16)>,
    apply_style: &mut FApply,
) -> io::Result<()>
where
    W: Write,
    FApply: FnMut(&mut W, &S) -> io::Result<()>,
{
    if line_buffer.is_empty() {
        return Ok(());
    }

    if let Some((x, y)) = pos {
        // Re-anchor the cursor at the run's computed column (CUP is 1-based).
        write!(stdout, "\x1b[{};{}H", y as u32 + 1, x as u32 + 1)?;
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

        // The continuation at col 1 is orphaned (col 0 is narrow), so it is
        // painted as a blank to keep 日 at its true grid column (2..4).
        assert_eq!(rendered_width, 4);
        assert_eq!(String::from_utf8(out).expect("utf8"), "A 日");
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

    #[test]
    fn render_row_stream_clips_wide_char_at_right_edge() {
        let attrs = CellAttrs::default();
        let cells = vec![
            Cell {
                grapheme: "A".to_string(),
                width: 1,
                attrs: attrs.clone(),
            },
            Cell {
                grapheme: "日".to_string(),
                width: 2,
                attrs,
            },
        ];

        // content_width 2: the wide char at col 1 would occupy cols 1-2,
        // crossing the boundary — it must be clipped.
        let mut out = Vec::new();
        let rendered_width = render_row_stream(&mut out, RenderRow::new(&cells, 2), |_col_idx, _cell| (), |stdout, _style| {
            write!(stdout, "")
        })
        .expect("row stream renders");

        assert_eq!(rendered_width, 1);
        assert_eq!(String::from_utf8(out).expect("utf8"), "A");
    }

    #[test]
    fn render_row_stream_strips_vs16_on_narrow_cells() {
        let attrs = CellAttrs::default();
        let cells = vec![
            Cell {
                grapheme: "⚠\u{FE0F}".to_string(),
                width: 1,
                attrs: attrs.clone(),
            },
            Cell {
                grapheme: "✅\u{FE0F}".to_string(),
                width: 2,
                attrs: attrs.clone(),
            },
            Cell::continuation(&attrs),
        ];

        let mut out = Vec::new();
        let rendered_width = render_row_stream(&mut out, RenderRow::new(&cells, 4), |_col_idx, _cell| (), |stdout, _style| {
            write!(stdout, "")
        })
        .expect("row stream renders");

        assert_eq!(rendered_width, 3);
        // VS16 stripped from the width-1 cell, kept on the width-2 cell.
        assert_eq!(String::from_utf8(out).expect("utf8"), "⚠✅\u{FE0F}");
    }

    #[test]
    fn render_row_stream_reanchors_each_wide_glyph_in_a_single_style_run() {
        // A same-style run of CJK glyphs must re-anchor the cursor before every
        // glyph, so per-glyph width drift on a terminal that miscounts widths
        // cannot accumulate and bleed past the row's right edge.
        let attrs = CellAttrs::default();
        let cells = vec![
            Cell { grapheme: "日".to_string(), width: 2, attrs: attrs.clone() },
            Cell::continuation(&attrs),
            Cell { grapheme: "本".to_string(), width: 2, attrs: attrs.clone() },
            Cell::continuation(&attrs),
            Cell { grapheme: "語".to_string(), width: 2, attrs },
        ];

        let mut out = Vec::new();
        render_row_stream(
            &mut out,
            RenderRow::with_origin(&cells, 6, 0, 0),
            |_col_idx, _cell| (),
            |stdout, _style| write!(stdout, ""),
        )
        .expect("row stream renders");

        // Each glyph is placed with its own CUP at its computed column
        // (cols 0, 2, 4 → CUP 1;1, 1;3, 1;5).
        assert_eq!(
            String::from_utf8(out).expect("utf8"),
            "\x1b[1;1H日\x1b[1;3H本\x1b[1;5H語"
        );
    }

    #[test]
    fn render_row_stream_batches_ascii_run_without_per_cell_anchoring() {
        // A plain ASCII run has no width ambiguity, so it stays batched into a
        // single positioned write — no per-cell CUP overhead.
        let attrs = CellAttrs::default();
        let cells = vec![
            Cell { grapheme: "a".to_string(), width: 1, attrs: attrs.clone() },
            Cell { grapheme: "b".to_string(), width: 1, attrs: attrs.clone() },
            Cell { grapheme: "c".to_string(), width: 1, attrs },
        ];

        let mut out = Vec::new();
        render_row_stream(
            &mut out,
            RenderRow::with_origin(&cells, 3, 0, 0),
            |_col_idx, _cell| (),
            |stdout, _style| write!(stdout, ""),
        )
        .expect("row stream renders");

        assert_eq!(String::from_utf8(out).expect("utf8"), "\x1b[1;1Habc");
    }

    #[test]
    fn orphaned_wide_cells_anchor_at_grid_column_and_never_pass_right_edge() {
        // A grid corrupted by a partial erase: width-2 cells with no
        // continuation cells between them. Width-sum accounting would anchor
        // these at columns 0/2/4 (past the true columns) and push the tail
        // beyond the pane edge; anchoring must use the actual grid column.
        let attrs = CellAttrs::default();
        let cells = vec![
            Cell { grapheme: "こ".to_string(), width: 2, attrs: attrs.clone() },
            Cell { grapheme: "う".to_string(), width: 2, attrs: attrs.clone() },
            Cell { grapheme: "じ".to_string(), width: 2, attrs },
        ];

        let mut out = Vec::new();
        let rendered_width = render_row_stream(
            &mut out,
            RenderRow::with_origin(&cells, 4, 0, 0),
            |_col_idx, _cell| (),
            |stdout, _style| write!(stdout, ""),
        )
        .expect("row stream renders");

        // Cols 0, 1, 2 — the glyph at col 2 ends exactly at the edge (col 4).
        assert_eq!(
            String::from_utf8(out).expect("utf8"),
            "\x1b[1;1Hこ\x1b[1;2Hう\x1b[1;3Hじ"
        );
        assert_eq!(rendered_width, 4);
    }

    #[test]
    fn orphaned_continuation_cell_renders_as_blank_keeping_columns_aligned() {
        // An orphaned continuation (its wide char was overwritten without
        // repair) must occupy its column as a blank — skipping it would shift
        // everything after it one column left.
        let attrs = CellAttrs::default();
        let cells = vec![
            Cell { grapheme: "A".to_string(), width: 1, attrs: attrs.clone() },
            Cell::continuation(&attrs),
            Cell { grapheme: "B".to_string(), width: 1, attrs },
        ];

        let mut out = Vec::new();
        let rendered_width = render_row_stream(
            &mut out,
            RenderRow::with_origin(&cells, 3, 0, 0),
            |_col_idx, _cell| (),
            |stdout, _style| write!(stdout, ""),
        )
        .expect("row stream renders");

        assert_eq!(String::from_utf8(out).expect("utf8"), "\x1b[1;1HA B");
        assert_eq!(rendered_width, 3);
    }

    #[test]
    fn render_row_stream_reanchors_cursor_per_style_run() {
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
                attrs,
            },
        ];

        let mut out = Vec::new();
        render_row_stream(
            &mut out,
            RenderRow::with_origin(&cells, 2, 10, 5),
            |col_idx, _cell| col_idx,
            |stdout, _style| write!(stdout, ""),
        )
        .expect("row stream renders");

        // Row 5 col 10 → CUP 6;11, then col 11 → CUP 6;12.
        assert_eq!(String::from_utf8(out).expect("utf8"), "\x1b[6;11HA\x1b[6;12HB");
    }
}
