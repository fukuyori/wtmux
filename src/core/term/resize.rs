use std::collections::VecDeque;

use super::state::{Cell, Row, ScreenBuffer};

// Additional policies are introduced in Phase 1 so later releases can switch
// resize behavior without re-entangling the implementation.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizePolicy {
    HostDriven,
    LocalReflow,
    NoReflow,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResizeOutcome {
    pub primary_cursor: Option<(u16, u16)>,
    pub prompt_anchor: Option<(u16, u16)>,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ReflowAnchor {
    pub abs_row: usize,
    pub col: u16,
}

pub(crate) struct ScreenResizePlan {
    pub rows: Vec<Row>,
    pub scrollback: VecDeque<Row>,
    pub scroll_offset: usize,
    pub anchor_positions: Vec<Option<(u16, u16)>>,
}

#[cfg(windows)]
pub const DEFAULT_SESSION_RESIZE_POLICY: ResizePolicy = ResizePolicy::HostDriven;

#[cfg(not(windows))]
pub const DEFAULT_SESSION_RESIZE_POLICY: ResizePolicy = ResizePolicy::LocalReflow;

pub(crate) fn reflow_screen(
    screen: &ScreenBuffer,
    new_cols: u16,
    new_rows: u16,
    anchors: &[ReflowAnchor],
    pin_from_abs_row: Option<usize>,
) -> ScreenResizePlan {
    let new_cols = new_cols.max(1);
    let old_scroll_offset = screen.scroll_offset;
    let old_visible_start = (old_scroll_offset > 0).then(|| screen.screen_to_buffer_row(0));
    let mut anchor_meta: Vec<Option<(usize, usize)>> = vec![None; anchors.len()];
    let mut logical_lines: Vec<Vec<Cell>> = Vec::new();
    let mut current_line: Vec<Cell> = Vec::new();
    let mut current_width = 0usize;
    let max_visible_anchor_row = anchors
        .iter()
        .filter_map(|anchor| anchor.abs_row.checked_sub(screen.scrollback.len()))
        .max();
    let last_content_row = screen.rows.iter().rposition(row_has_content);
    let visible_rows_len = last_content_row
        .into_iter()
        .chain(max_visible_anchor_row)
        .max()
        .map(|idx| idx + 1)
        .unwrap_or(0);
    let total_abs_len = screen.scrollback.len() + visible_rows_len;
    // Rows at/below this absolute row bypass the rewrap and are carried
    // through physically (see the pinned-region block below).
    let pin_from = pin_from_abs_row
        .unwrap_or(total_abs_len)
        .clamp(screen.scrollback.len(), total_abs_len);

    for (abs_row, row) in screen
        .scrollback
        .iter()
        .chain(screen.rows.iter().take(visible_rows_len))
        .enumerate()
    {
        if abs_row >= pin_from {
            break;
        }
        let mut preserve_until_col = None;
        for (idx, anchor) in anchors.iter().enumerate() {
            if anchor.abs_row == abs_row {
                let offset = current_width + display_offset_before_col(row, anchor.col);
                anchor_meta[idx] = Some((logical_lines.len(), offset));
                preserve_until_col =
                    Some(preserve_until_col.map_or(anchor.col, |col: u16| col.max(anchor.col)));
            }
        }

        let extracted = extract_reflow_cells(row, preserve_until_col);
        current_width += extracted
            .iter()
            .map(|cell| cell.width.max(1) as usize)
            .sum::<usize>();
        current_line.extend(extracted);

        if !row.wrapped {
            logical_lines.push(current_line);
            current_line = Vec::new();
            current_width = 0;
        }
    }

    if !current_line.is_empty() || (logical_lines.is_empty() && pin_from >= total_abs_len) {
        logical_lines.push(current_line);
    }

    let mut anchor_positions_abs: Vec<Option<(usize, u16)>> = vec![None; anchors.len()];
    let mut physical_rows: Vec<Row> = Vec::new();

    for (line_idx, line_cells) in logical_lines.into_iter().enumerate() {
        let line_start_abs_row = physical_rows.len();
        let row_start_offsets = append_wrapped_line(&mut physical_rows, line_cells, new_cols);

        for (anchor_idx, meta) in anchor_meta.iter().enumerate() {
            let Some((anchor_line_idx, anchor_offset)) = meta else {
                continue;
            };
            if *anchor_line_idx != line_idx {
                continue;
            }

            let mut row_in_line = 0usize;
            while row_in_line + 1 < row_start_offsets.len()
                && row_start_offsets[row_in_line + 1] <= *anchor_offset
            {
                row_in_line += 1;
            }

            let row_start = row_start_offsets[row_in_line];
            let col = anchor_offset
                .saturating_sub(row_start)
                .min(new_cols.saturating_sub(1) as usize) as u16;
            anchor_positions_abs[anchor_idx] = Some((line_start_abs_row + row_in_line, col));
        }
    }

    // Pinned region: rows at/below the shell's active prompt/input line are
    // carried through as-is (truncated or padded to the new width) instead
    // of being rewrapped. The shell repaints this region itself on the
    // post-resize SIGWINCH, and its cursor-relative erase/reprint sequences
    // only line up if these rows keep their physical layout.
    let pinned_base = physical_rows.len();
    for screen_row in (pin_from - screen.scrollback.len())..visible_rows_len {
        let mut row = screen.rows[screen_row].clone();
        row.resize(new_cols);
        physical_rows.push(row);
    }
    for (anchor_idx, anchor) in anchors.iter().enumerate() {
        if anchor.abs_row >= pin_from && anchor.abs_row < total_abs_len {
            anchor_positions_abs[anchor_idx] = Some((
                pinned_base + (anchor.abs_row - pin_from),
                anchor.col.min(new_cols.saturating_sub(1)),
            ));
        }
    }

    let total_rows = physical_rows.len();
    let (scrollback, rows) = if total_rows > new_rows as usize {
        let split_at = total_rows - new_rows as usize;
        let visible_rows = physical_rows.split_off(split_at);
        (physical_rows.into_iter().collect(), visible_rows)
    } else {
        let mut rows = physical_rows;
        while rows.len() < new_rows as usize {
            rows.push(Row::new(new_cols));
        }
        (VecDeque::new(), rows)
    };

    let visible_start = scrollback.len();
    let anchor_positions = anchor_positions_abs
        .into_iter()
        .map(|pos| {
            pos.and_then(|(abs_row, col)| {
                if abs_row < visible_start {
                    None
                } else {
                    Some(((abs_row - visible_start) as u16, col))
                }
            })
        })
        .collect();

    let new_scrollback_len = scrollback.len();

    ScreenResizePlan {
        rows,
        scrollback,
        scroll_offset: remap_scroll_offset(old_scroll_offset, old_visible_start, new_scrollback_len),
        anchor_positions,
    }
}

pub(crate) fn host_resize_screen(
    screen: &ScreenBuffer,
    new_cols: u16,
    new_rows: u16,
) -> ScreenResizePlan {
    let old_scroll_offset = screen.scroll_offset;
    let old_visible_start = (old_scroll_offset > 0).then(|| screen.screen_to_buffer_row(0));
    let new_cols = new_cols.max(1);
    let new_rows = new_rows.max(1);

    let mut physical_rows: Vec<Row> = screen
        .scrollback
        .iter()
        .chain(screen.rows.iter())
        .cloned()
        .collect();

    let total_rows = physical_rows.len();
    let (mut scrollback, mut rows) = if total_rows > new_rows as usize {
        let split_at = total_rows - new_rows as usize;
        let visible_rows = physical_rows.split_off(split_at);
        (physical_rows.into_iter().collect(), visible_rows)
    } else {
        let mut rows = physical_rows;
        while rows.len() < new_rows as usize {
            rows.push(Row::new(new_cols));
        }
        (VecDeque::new(), rows)
    };

    for row in &mut scrollback {
        row.resize(new_cols);
    }
    for row in &mut rows {
        row.resize(new_cols);
    }

    let new_scrollback_len = scrollback.len();

    ScreenResizePlan {
        rows,
        scrollback,
        scroll_offset: remap_scroll_offset(old_scroll_offset, old_visible_start, new_scrollback_len),
        anchor_positions: Vec::new(),
    }
}

fn remap_scroll_offset(
    old_scroll_offset: usize,
    old_visible_start: Option<usize>,
    new_scrollback_len: usize,
) -> usize {
    old_visible_start.map_or(old_scroll_offset, |visible_start| {
        new_scrollback_len.saturating_sub(visible_start.min(new_scrollback_len))
    })
}

fn extract_reflow_cells(row: &Row, preserve_until_col: Option<u16>) -> Vec<Cell> {
    let mut last_used_col = preserve_until_col.unwrap_or(0) as usize;
    for (col_idx, cell) in row.cells.iter().enumerate() {
        if cell.is_continuation() {
            continue;
        }
        if !cell.grapheme.is_empty() {
            last_used_col = last_used_col.max(col_idx + cell.width.max(1) as usize);
        }
    }

    let mut cells = Vec::new();
    for (col_idx, cell) in row.cells.iter().enumerate() {
        if col_idx >= last_used_col {
            break;
        }
        if cell.is_continuation() {
            continue;
        }
        cells.push(Cell {
            grapheme: cell.grapheme.clone(),
            width: cell.width.max(1),
            attrs: cell.attrs.clone(),
        });
    }
    cells
}

fn display_offset_before_col(row: &Row, col: u16) -> usize {
    let mut offset = 0usize;
    let target_col = col as usize;
    for (col_idx, cell) in row.cells.iter().enumerate() {
        if col_idx >= target_col {
            break;
        }
        if cell.is_continuation() {
            continue;
        }
        offset += cell.width.max(1) as usize;
    }
    offset.min(target_col)
}

fn row_has_content(row: &Row) -> bool {
    row.wrapped
        || row
            .cells
            .iter()
            .any(|cell| !cell.is_continuation() && !cell.grapheme.is_empty())
}

fn append_wrapped_line(rows: &mut Vec<Row>, line_cells: Vec<Cell>, cols: u16) -> Vec<usize> {
    let cols = cols.max(1);
    let mut row = Row::new(cols);
    let mut col_idx = 0usize;
    let mut offset = 0usize;
    let mut row_start_offsets = vec![0usize];

    if line_cells.is_empty() {
        rows.push(row);
        return row_start_offsets;
    }

    for cell in line_cells {
        let cell_width = cell.width.max(1) as usize;

        if col_idx >= cols as usize {
            row.wrapped = true;
            rows.push(row);
            row = Row::new(cols);
            col_idx = 0;
            row_start_offsets.push(offset);
        }

        let attrs = cell.attrs.clone();
        row.cells[col_idx] = Cell {
            grapheme: cell.grapheme,
            width: cell.width.max(1),
            attrs: attrs.clone(),
        };
        if cell_width == 2 && col_idx + 1 < cols as usize {
            row.cells[col_idx + 1] = Cell::continuation(&attrs);
        }

        col_idx += cell_width;
        offset += cell_width;
    }

    rows.push(row);
    row_start_offsets
}
