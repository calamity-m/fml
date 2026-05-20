//! Shared word-boundary wrap helpers for styled spans.
//!
//! Used by both the info pane (vertical key/value paragraph layout) and the
//! log pane (per-entry continuation lines under the `msg` column). Generalized
//! from the original `InfoPane::wrap_spans` so both call sites share a single
//! implementation — see `[[feedback_no_test_only_helpers]]`: prefer
//! generalising the existing helper to adding a sibling.

use ratatui::{
    style::Style,
    text::{Line, Span},
};
use unicode_width::UnicodeWidthChar;

/// Wraps a styled span sequence into multiple lines using display-cell width.
///
/// Lines after the first are optionally prefixed with `hanging_indent` so callers
/// can align continuation text under a chosen column. The first line is never
/// prefixed; the caller is expected to render its own leading content there.
///
/// `preserve_leading_whitespace = false` (info-pane behavior) strips whitespace
/// at the start of each wrapped chunk so prose flows naturally onto continuation
/// lines. `preserve_leading_whitespace = true` (log-pane behavior) keeps the
/// in-text indentation of each chunk, so stack traces and pretty-printed JSON
/// retain their structure on continuation lines.
///
/// Embedded `\n` characters are treated as hard physical-line breaks before
/// width wrapping, so multi-line input is rendered as multiple wrapped sections
/// rather than collapsed onto one line.
///
/// Width is interpreted in terminal display cells. A zero or one-cell width is
/// clamped up to 1 so the routine always makes progress.
pub fn wrap_styled_spans(
    spans: Vec<Span<'static>>,
    width: u16,
    hanging_indent: &[Span<'static>],
    preserve_leading_whitespace: bool,
) -> Vec<Line<'static>> {
    let width = usize::from(width).max(1);

    let styled_chars: Vec<(char, Style)> = spans
        .into_iter()
        .flat_map(|span| {
            let style = span.style;
            span.content
                .chars()
                .map(move |ch| (ch, style))
                .collect::<Vec<_>>()
        })
        .collect();

    if styled_chars.is_empty() {
        return vec![Line::default()];
    }

    // Split on embedded newlines into physical sections. Each section is then
    // wrapped independently. The newline itself is dropped (it would otherwise
    // confuse the cell-width counter).
    let sections: Vec<&[(char, Style)]> = split_on_newline(&styled_chars);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut first_line_emitted = false;

    for section in sections {
        if section.is_empty() {
            // An empty physical line — emit one blank visual line so embedded
            // `\n\n` shows up as a blank in the output.
            push_line(
                &mut lines,
                Line::default(),
                hanging_indent,
                &mut first_line_emitted,
            );
            continue;
        }
        wrap_section(
            section,
            width,
            hanging_indent,
            preserve_leading_whitespace,
            &mut lines,
            &mut first_line_emitted,
        );
    }

    if lines.is_empty() {
        lines.push(Line::default());
    }
    lines
}

fn split_on_newline(chars: &[(char, Style)]) -> Vec<&[(char, Style)]> {
    let mut sections = Vec::new();
    let mut start = 0;
    for (idx, (ch, _)) in chars.iter().enumerate() {
        if *ch == '\n' {
            sections.push(&chars[start..idx]);
            start = idx + 1;
        }
    }
    sections.push(&chars[start..]);
    sections
}

fn wrap_section(
    section: &[(char, Style)],
    width: usize,
    hanging_indent: &[Span<'static>],
    preserve_leading_whitespace: bool,
    lines: &mut Vec<Line<'static>>,
    first_line_emitted: &mut bool,
) {
    let mut start = 0;
    let mut at_section_start = true;
    while start < section.len() {
        // `preserve_leading_whitespace` only applies at the start of a physical
        // section (i.e. right after a hard `\n` break). After a word-boundary
        // wrap inside the same section the leading space is an artifact of the
        // wrap and must be stripped, otherwise continuation lines drift right
        // of their intended column.
        let strip_leading_ws = !preserve_leading_whitespace || !at_section_start;
        if strip_leading_ws {
            while section
                .get(start)
                .is_some_and(|(ch, _)| ch.is_whitespace() && *ch != '\n')
            {
                start += 1;
            }
            if start >= section.len() {
                break;
            }
        }
        at_section_start = false;

        // Walk forward in display cells until we either hit the width budget
        // or run out of characters.
        let mut cells = 0usize;
        let mut hard_end = start;
        while hard_end < section.len() {
            let (ch, _) = section[hard_end];
            let w = UnicodeWidthChar::width(ch).unwrap_or(0);
            if cells + w > width && hard_end > start {
                break;
            }
            cells += w;
            hard_end += 1;
        }

        // Prefer a word boundary inside the window.
        let end = if hard_end < section.len() {
            section[start..hard_end]
                .iter()
                .rposition(|(ch, _)| ch.is_whitespace())
                .filter(|idx| *idx > 0)
                .map(|idx| start + idx)
                .unwrap_or(hard_end)
        } else {
            hard_end
        };

        let line = line_from_styled_chars(&section[start..end]);
        push_line(lines, line, hanging_indent, first_line_emitted);
        start = end;
    }
}

fn push_line(
    lines: &mut Vec<Line<'static>>,
    mut line: Line<'static>,
    hanging_indent: &[Span<'static>],
    first_line_emitted: &mut bool,
) {
    if *first_line_emitted && !hanging_indent.is_empty() {
        let mut prefixed: Vec<Span<'static>> = hanging_indent.to_vec();
        prefixed.append(&mut line.spans);
        line = Line::from(prefixed);
    }
    lines.push(line);
    *first_line_emitted = true;
}

/// Collapse a `(char, Style)` slice into a `Line` with one span per style run.
pub fn line_from_styled_chars(chars: &[(char, Style)]) -> Line<'static> {
    let mut spans = Vec::new();
    let mut current = String::new();
    let mut current_style: Option<Style> = None;

    for (ch, style) in chars {
        if current_style.is_some_and(|cs| cs != *style) {
            let cs = current_style.expect("checked above");
            spans.push(Span::styled(std::mem::take(&mut current), cs));
        }
        current.push(*ch);
        current_style = Some(*style);
    }

    if let Some(style) = current_style {
        spans.push(Span::styled(current, style));
    }

    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier, Style};

    fn plain(s: &str) -> Vec<Span<'static>> {
        vec![Span::raw(s.to_string())]
    }

    fn flat_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn empty_spans_returns_one_empty_line() {
        let out = wrap_styled_spans(Vec::new(), 10, &[], false);
        assert_eq!(out.len(), 1);
        assert!(out[0].spans.is_empty());
    }

    #[test]
    fn single_span_fits_in_width() {
        let out = wrap_styled_spans(plain("hello"), 10, &[], false);
        assert_eq!(out.len(), 1);
        assert_eq!(flat_text(&out[0]), "hello");
    }

    #[test]
    fn wraps_at_word_boundary_preserving_style() {
        let red = Style::default().fg(Color::Red);
        let blue = Style::default().fg(Color::Blue);
        let spans = vec![
            Span::styled("alpha ".to_string(), red),
            Span::styled("beta gamma".to_string(), blue),
        ];

        let out = wrap_styled_spans(spans, 10, &[], false);
        assert_eq!(out.len(), 2);
        assert_eq!(flat_text(&out[0]), "alpha");
        assert_eq!(flat_text(&out[1]), "beta gamma");
        // The 'beta' on line 2 must still be blue.
        let beta_span = out[1]
            .spans
            .iter()
            .find(|s| s.content.contains("beta"))
            .expect("beta span");
        assert_eq!(beta_span.style.fg, Some(Color::Blue));
    }

    #[test]
    fn hanging_indent_applied_to_continuation_only() {
        let indent = vec![Span::raw("    ".to_string())];
        let out = wrap_styled_spans(plain("alpha beta gamma delta"), 10, &indent, false);
        assert!(out.len() >= 2);
        assert!(!flat_text(&out[0]).starts_with("    "));
        for line in &out[1..] {
            assert!(
                flat_text(line).starts_with("    "),
                "continuation line missing indent: {:?}",
                flat_text(line)
            );
        }
    }

    #[test]
    fn hard_break_on_long_unbroken_token() {
        let out = wrap_styled_spans(plain("abcdefghijklmnop"), 4, &[], false);
        assert_eq!(out.len(), 4);
        assert_eq!(flat_text(&out[0]), "abcd");
        assert_eq!(flat_text(&out[1]), "efgh");
        assert_eq!(flat_text(&out[2]), "ijkl");
        assert_eq!(flat_text(&out[3]), "mnop");
    }

    #[test]
    fn preserve_leading_whitespace_keeps_indentation() {
        // A JSON-shaped input that wraps and must keep its 4-space indent on
        // continuation chunks.
        let input = "    \"key\": \"value-that-is-long-enough-to-wrap\"";
        let out = wrap_styled_spans(plain(input), 20, &[], true);
        assert!(out.len() >= 2);
        // First chunk keeps its leading 4 spaces.
        assert!(flat_text(&out[0]).starts_with("    \""));
    }

    #[test]
    fn info_pane_mode_strips_leading_whitespace_on_continuation() {
        // Mirrors the previous InfoPane behavior: continuation chunks have
        // their leading whitespace dropped so prose flows.
        let out = wrap_styled_spans(plain("alpha    beta gamma"), 5, &[], false);
        assert_eq!(flat_text(&out[0]), "alpha");
        // No leading whitespace.
        assert!(!flat_text(&out[1]).starts_with(' '));
    }

    #[test]
    fn hanging_indent_combined_with_preserve_leading_whitespace() {
        let indent = vec![Span::raw(">>".to_string())];
        let out = wrap_styled_spans(plain("alpha     beta"), 6, &indent, true);
        assert!(out.len() >= 2);
        assert!(!flat_text(&out[0]).starts_with(">>"));
        let second = flat_text(&out[1]);
        // The indent comes before any preserved leading whitespace of the chunk.
        assert!(
            second.starts_with(">>"),
            "expected line 2 to start with indent, got {second:?}"
        );
    }

    #[test]
    fn embedded_newline_is_hard_break() {
        let out = wrap_styled_spans(plain("alpha\nbeta"), 80, &[], false);
        assert_eq!(out.len(), 2);
        assert_eq!(flat_text(&out[0]), "alpha");
        assert_eq!(flat_text(&out[1]), "beta");
    }

    #[test]
    fn embedded_newline_preserves_indent_in_preserve_mode() {
        let out = wrap_styled_spans(plain("alpha\n    beta").to_vec(), 80, &[], true);
        assert_eq!(out.len(), 2);
        assert_eq!(flat_text(&out[1]), "    beta");
    }

    #[test]
    fn wide_characters_respect_display_width() {
        // CJK double-width characters take 2 cells each; 4 cells of width fits 2 of them.
        let out = wrap_styled_spans(plain("漢字漢字"), 4, &[], false);
        assert_eq!(out.len(), 2);
        assert_eq!(flat_text(&out[0]), "漢字");
        assert_eq!(flat_text(&out[1]), "漢字");
    }

    #[test]
    fn line_from_styled_chars_collapses_equal_style_runs() {
        let s = Style::default().add_modifier(Modifier::BOLD);
        let chars = vec![('a', s), ('b', s), ('c', s)];
        let line = line_from_styled_chars(&chars);
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content.as_ref(), "abc");
    }
}
