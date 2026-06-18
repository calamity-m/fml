//! Pure display-layout math for the log view.
//!
//! Navigation and selection stay entry/logical-based on the flat
//! [`row_text`](super::pane::row_text); the only place a logical char offset is
//! turned into a `(display_row, display_col)` position is at render time. This
//! module is that single source of truth — no rendering, no pane state, just
//! the wrap arithmetic, so it can be exhaustively unit-tested.
//!
//! Wrapping is **greedy char-wrap**: rows break exactly at the usable width
//! using Rust `char` offsets as the unit. It is intentionally **not**
//! terminal-cell/grapheme aware, so wide characters, emoji, and combining
//! marks may misalign — the same limitation the truncation and yank paths
//! already carry.

/// One rendered display row of an entry.
///
/// `start_col..end_col` is the half-open logical char range (offsets into the
/// entry's `row_text`) this row shows. `indent` is the render-column offset at
/// which that text is drawn: `0` on the first row of an entry, the hanging
/// indent on continuation rows. Consumers must use this `indent` rather than
/// recomputing it, so per-char highlights line up on continuation rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayRow {
    pub start_col: usize,
    pub end_col: usize,
    pub indent: usize,
}

impl DisplayRow {
    /// Number of logical chars this row shows.
    pub fn len(&self) -> usize {
        self.end_col - self.start_col
    }

    /// Whether the row carries no logical chars (an empty entry).
    pub fn is_empty(&self) -> bool {
        self.end_col == self.start_col
    }
}

/// Lay `text` out into ordered display rows for a content area `width` chars
/// wide, using `indent` as the continuation-row hanging indent.
///
/// With `wrap` off this returns exactly one row truncated to `width`,
/// reproducing the historical `.chars().take(width)` behavior. With `wrap` on
/// the first row spans the full `width` and continuation rows are shifted right
/// by the effective indent (`min(indent, width - 1)`), with at least one usable
/// content cell whenever `width > 0`.
pub fn layout_entry(text: &str, width: usize, indent: usize, wrap: bool) -> Vec<DisplayRow> {
    let len = text.chars().count();

    if !wrap {
        return vec![DisplayRow {
            start_col: 0,
            end_col: len.min(width),
            indent: 0,
        }];
    }

    // Degenerate zero-width area: nothing is renderable, but still return one
    // row so the renderer always has a row per entry.
    if width == 0 {
        return vec![DisplayRow {
            start_col: 0,
            end_col: 0,
            indent: 0,
        }];
    }

    // First row uses the full width with no indent.
    let first_end = len.min(width);
    let mut rows = vec![DisplayRow {
        start_col: 0,
        end_col: first_end,
        indent: 0,
    }];
    let mut col = first_end;
    if col >= len {
        return rows;
    }

    // Continuation rows: shift right by the indent, keeping at least one cell.
    let eff_indent = indent.min(width.saturating_sub(1));
    let usable = (width - eff_indent).max(1);
    while col < len {
        let end = (col + usable).min(len);
        rows.push(DisplayRow {
            start_col: col,
            end_col: end,
            indent: eff_indent,
        });
        col = end;
    }
    rows
}

/// Map a logical char offset to `(row_index, display_col)` within `rows`.
///
/// `display_col` already includes the row's indent. Offsets past the end of
/// the text are clamped onto the last char cell, so callers can feed a sticky
/// cursor column without bounds-checking first. `rows` must be non-empty (every
/// [`layout_entry`] result is).
pub fn col_to_display(rows: &[DisplayRow], col: usize) -> (usize, usize) {
    if rows.is_empty() {
        return (0, 0);
    }
    let len = rows[rows.len() - 1].end_col;
    let col = col.min(len.saturating_sub(1));
    for (idx, row) in rows.iter().enumerate() {
        if col < row.end_col || idx == rows.len() - 1 {
            return (idx, row.indent + col.saturating_sub(row.start_col));
        }
    }
    (0, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(start: usize, end: usize, indent: usize) -> DisplayRow {
        DisplayRow {
            start_col: start,
            end_col: end,
            indent,
        }
    }

    #[test]
    fn wrap_off_truncates_to_one_row() {
        // Long text, narrow width: a single row clipped to width, no indent.
        let rows = layout_entry("hello world", 5, 4, false);
        assert_eq!(rows, vec![row(0, 5, 0)]);
    }

    #[test]
    fn wrap_off_short_line_keeps_full_text() {
        let rows = layout_entry("hi", 10, 4, false);
        assert_eq!(rows, vec![row(0, 2, 0)]);
    }

    #[test]
    fn wrap_on_short_line_is_one_row() {
        let rows = layout_entry("hi", 10, 4, true);
        assert_eq!(rows, vec![row(0, 2, 0)]);
    }

    #[test]
    fn wrap_on_exact_width_boundary_is_one_row() {
        // Text exactly fills the first row; no empty continuation row.
        let rows = layout_entry("abcde", 5, 2, true);
        assert_eq!(rows, vec![row(0, 5, 0)]);
    }

    #[test]
    fn wrap_on_multi_row_with_hanging_indent() {
        // width 10, indent 4 → first row 10 chars, continuations 6 chars each.
        let text: String = ('a'..='z').collect(); // 26 chars
        let rows = layout_entry(&text, 10, 4, true);
        assert_eq!(
            rows,
            vec![
                row(0, 10, 0),
                row(10, 16, 4),
                row(16, 22, 4),
                row(22, 26, 4)
            ]
        );
    }

    #[test]
    fn wrap_on_empty_text_is_one_empty_row() {
        let rows = layout_entry("", 10, 4, true);
        assert_eq!(rows, vec![row(0, 0, 0)]);
        assert!(rows[0].is_empty());
    }

    #[test]
    fn wrap_on_width_equal_to_indent_keeps_one_usable_cell() {
        // width == indent: effective indent saturates to width-1 so at least
        // one content cell remains and the layout cannot loop forever.
        let rows = layout_entry("abcd", 4, 4, true);
        // First row: 4 chars. Continuation: indent 3, usable 1.
        assert_eq!(rows[0], row(0, 4, 0));
        // "abcd" is exactly width, so it fits in one row here.
        assert_eq!(rows.len(), 1);

        // Now force a continuation by exceeding the first row.
        let rows = layout_entry("abcdef", 4, 4, true);
        assert_eq!(rows[0], row(0, 4, 0));
        for cont in &rows[1..] {
            assert_eq!(cont.indent, 3);
            assert_eq!(cont.len(), 1);
        }
        assert_eq!(rows.last().unwrap().end_col, 6);
    }

    #[test]
    fn wrap_on_width_smaller_than_indent_saturates() {
        let rows = layout_entry("abcdef", 3, 10, true);
        assert_eq!(rows[0], row(0, 3, 0));
        // effective indent clamped to width-1 = 2, usable = 1.
        for cont in &rows[1..] {
            assert_eq!(cont.indent, 2);
            assert_eq!(cont.len(), 1);
        }
        assert_eq!(rows.last().unwrap().end_col, 6);
    }

    #[test]
    fn wrap_on_zero_width_is_one_empty_row() {
        let rows = layout_entry("abc", 0, 4, true);
        assert_eq!(rows, vec![row(0, 0, 0)]);
    }

    #[test]
    fn col_to_display_within_first_row() {
        let rows = layout_entry("abcdefghij", 10, 4, true);
        assert_eq!(col_to_display(&rows, 0), (0, 0));
        assert_eq!(col_to_display(&rows, 3), (0, 3));
    }

    #[test]
    fn col_to_display_at_row_boundaries() {
        // rows: [0,10) indent 0, [10,16) indent 4, ...
        let text: String = ('a'..='z').collect();
        let rows = layout_entry(&text, 10, 4, true);
        // col 9 is the last cell of row 0.
        assert_eq!(col_to_display(&rows, 9), (0, 9));
        // col 10 is the first cell of the first continuation row, offset by 4.
        assert_eq!(col_to_display(&rows, 10), (1, 4));
        // col 16 starts the second continuation row.
        assert_eq!(col_to_display(&rows, 16), (2, 4));
    }

    #[test]
    fn col_to_display_past_end_clamps_to_last_cell() {
        let text: String = ('a'..='z').collect(); // last row [22,26)
        let rows = layout_entry(&text, 10, 4, true);
        // col way past end clamps onto the last char (col 25) of the last row.
        assert_eq!(col_to_display(&rows, 999), (3, 4 + (25 - 22)));
    }

    #[test]
    fn col_to_display_empty_text() {
        let rows = layout_entry("", 10, 4, true);
        assert_eq!(col_to_display(&rows, 0), (0, 0));
        assert_eq!(col_to_display(&rows, 5), (0, 0));
    }

    #[test]
    fn col_to_display_wrap_off_truncated() {
        // Wrap off: one row clipped to width. A cursor past width clamps onto
        // the last visible cell (the pre-existing off-screen quirk is a render
        // concern; the mapping itself stays in range).
        let rows = layout_entry("abcdefghij", 5, 4, false);
        assert_eq!(col_to_display(&rows, 2), (0, 2));
        assert_eq!(col_to_display(&rows, 9), (0, 4));
    }

    #[test]
    fn unicode_is_treated_as_chars_not_cells() {
        // Known limitation: layout counts Rust chars, not terminal cells. A
        // wide char occupies one logical column here.
        let rows = layout_entry("a😀b😀c", 3, 1, true);
        // 5 chars, width 3 → first row 3 chars, continuation 2 chars.
        assert_eq!(rows[0], row(0, 3, 0));
        assert_eq!(rows[1].start_col, 3);
        assert_eq!(rows[1].end_col, 5);
    }
}
