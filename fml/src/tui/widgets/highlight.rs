use std::collections::HashSet;

use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::{event::Match, tui::widgets::truncate};

pub fn styled_field(
    text: &str,
    matches: Option<&[Match]>,
    key: &str,
    base_style: Style,
    match_style: Style,
) -> Vec<Span<'static>> {
    let matched = matched_indices(matches, key);
    if matched.is_empty() {
        return vec![Span::styled(text.to_string(), base_style)];
    }

    let mut spans = Vec::new();
    push_run(
        &mut spans,
        text.chars().enumerate(),
        &matched,
        base_style,
        match_style,
    );
    spans
}

/// Like [`styled_field`], but middle-truncates the rendered text to at most
/// `max_width` display cells. Match highlights survive the cut: the surviving
/// head and tail keep their per-character styling because match indices are
/// remapped through the original char offsets that [`truncate::plan`] reports.
pub fn styled_truncated_field(
    text: &str,
    matches: Option<&[Match]>,
    key: &str,
    base_style: Style,
    match_style: Style,
    max_width: u16,
) -> Vec<Span<'static>> {
    let Some(plan) = truncate::plan(text, max_width) else {
        return styled_field(text, matches, key, base_style, match_style);
    };

    let matched = matched_indices(matches, key);
    let chars: Vec<char> = text.chars().collect();

    let mut spans = Vec::new();
    push_run(
        &mut spans,
        chars[..plan.head].iter().copied().enumerate(),
        &matched,
        base_style,
        match_style,
    );
    spans.push(Span::styled(truncate::ELLIPSIS.to_string(), base_style));
    push_run(
        &mut spans,
        chars[plan.tail_start..]
            .iter()
            .copied()
            .enumerate()
            .map(|(offset, ch)| (plan.tail_start + offset, ch)),
        &matched,
        base_style,
        match_style,
    );
    spans
}

/// Coalesce `(char_index, char)` pairs into styled spans, switching style at
/// each transition between matched and unmatched characters. `char_index` is
/// the offset into the full (untruncated) text so callers can render a slice
/// while keeping the original match offsets aligned.
fn push_run(
    spans: &mut Vec<Span<'static>>,
    chars: impl Iterator<Item = (usize, char)>,
    matched: &HashSet<usize>,
    base_style: Style,
    match_style: Style,
) {
    let mut chunk = String::new();
    let mut chunk_is_match = false;
    let mut has_chunk = false;

    let style_for = |is_match: bool| if is_match { match_style } else { base_style };

    for (idx, ch) in chars {
        let is_match = matched.contains(&idx);
        if has_chunk && is_match != chunk_is_match {
            spans.push(Span::styled(
                std::mem::take(&mut chunk),
                style_for(chunk_is_match),
            ));
        }
        chunk.push(ch);
        chunk_is_match = is_match;
        has_chunk = true;
    }

    if has_chunk {
        spans.push(Span::styled(chunk, style_for(chunk_is_match)));
    }
}

pub fn field_line(
    label: &str,
    value: &str,
    match_key: &str,
    matches: Option<&[Match]>,
    label_style: Style,
    value_style: Style,
    match_style: Style,
) -> Line<'static> {
    let mut spans = vec![
        Span::styled(label.to_string(), label_style),
        Span::styled(": ", label_style),
    ];
    spans.extend(styled_field(
        value,
        matches,
        match_key,
        value_style,
        match_style,
    ));
    Line::from(spans)
}

fn matched_indices(matches: Option<&[Match]>, key: &str) -> HashSet<usize> {
    matches
        .into_iter()
        .flat_map(|matches| matches.iter())
        .filter(|m| m.key == key)
        .flat_map(|m| m.indices.iter())
        .map(|idx| *idx as usize)
        .collect()
}
