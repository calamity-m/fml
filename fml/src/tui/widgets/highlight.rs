use std::collections::HashSet;

use ratatui::{
    style::Style,
    text::{Line, Span},
};

use crate::event::Match;

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
    let mut chunk = String::new();
    let mut chunk_is_match = false;
    let mut has_chunk = false;

    for (idx, ch) in text.chars().enumerate() {
        let is_match = matched.contains(&idx);
        if has_chunk && is_match != chunk_is_match {
            let style = if chunk_is_match {
                match_style
            } else {
                base_style
            };
            spans.push(Span::styled(std::mem::take(&mut chunk), style));
        }
        chunk.push(ch);
        chunk_is_match = is_match;
        has_chunk = true;
    }

    if has_chunk {
        let style = if chunk_is_match {
            match_style
        } else {
            base_style
        };
        spans.push(Span::styled(chunk, style));
    }

    spans
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
