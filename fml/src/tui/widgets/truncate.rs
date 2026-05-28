//! Middle-truncation for source display names.
//!
//! Kubernetes sources in particular carry very long display names
//! (`pod-name-7d4b8c9f5d-x2k4p/container`). Rendered verbatim they crowd out
//! the message column. Middle-truncation keeps the informative head (workload
//! prefix) and tail (the most-specific suffix — the container name, or a file
//! name) while dropping only the noisy middle (replicaset/pod hashes), so the
//! result stays recognisable regardless of producer.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Single-cell marker inserted where the dropped middle run used to be.
pub const ELLIPSIS: char = '…';

/// Smallest width a truncated source name may shrink to. Below this a
/// `pod/container` name can't keep both a readable workload prefix and a
/// readable container suffix, so two pods of one workload become
/// indistinguishable.
pub const MIN_SOURCE_WIDTH: u16 = 16;

/// Largest width a source name may use on roomy panes, so a single very long
/// name can't dominate a wide terminal even when there's space to spare.
pub const MAX_SOURCE_WIDTH: u16 = 30;

/// Maximum display width a source name may use given the pane's inner width.
///
/// Targets roughly half the row — enough to keep the workload prefix and
/// container suffix legible — clamped to [`MIN_SOURCE_WIDTH`]..=[`MAX_SOURCE_WIDTH`]
/// so the source column neither collapses on narrow panes nor swallows wide ones.
pub fn source_budget(inner_width: u16) -> u16 {
    (inner_width / 2).clamp(MIN_SOURCE_WIDTH, MAX_SOURCE_WIDTH)
}

/// A planned middle truncation: keep the first `head` chars, drop the middle,
/// then keep every char from index `tail_start` onward, joined by an ellipsis.
///
/// Indices are char offsets into the original text, which lets callers that
/// highlight per-character matches remap their offsets across the cut.
pub(crate) struct Plan {
    pub head: usize,
    pub tail_start: usize,
}

/// Plan a middle truncation of `text` so it fits within `max_width` display
/// cells (ellipsis included). Returns `None` when `text` already fits.
pub(crate) fn plan(text: &str, max_width: u16) -> Option<Plan> {
    if text.width() as u16 <= max_width {
        return None;
    }

    let chars: Vec<char> = text.chars().collect();
    // One cell goes to the ellipsis; bias the remaining budget toward the tail
    // so the most-specific suffix survives when the split is uneven.
    let budget = max_width.saturating_sub(1) as usize;
    let tail_budget = budget / 2 + budget % 2;
    let head_budget = budget - tail_budget;

    let head = take_width(chars.iter().copied(), head_budget);
    let tail_kept = take_width(chars.iter().rev().copied(), tail_budget);
    let tail_start = chars.len() - tail_kept;

    // On degenerate inputs (very small budget, wide chars) the head and tail
    // could meet or overlap; clamp so the kept ranges never cross.
    let tail_start = tail_start.max(head);
    Some(Plan { head, tail_start })
}

/// Count how many leading items from `chars` fit within `budget` display cells.
fn take_width(chars: impl Iterator<Item = char>, budget: usize) -> usize {
    let mut used = 0;
    let mut count = 0;
    for c in chars {
        let w = UnicodeWidthChar::width(c).unwrap_or(0);
        if used + w > budget {
            break;
        }
        used += w;
        count += 1;
    }
    count
}

/// Middle-truncate `text` to at most `max_width` display cells, returning the
/// original string when it already fits.
pub fn truncate_middle(text: &str, max_width: u16) -> String {
    match plan(text, max_width) {
        None => text.to_string(),
        Some(Plan { head, tail_start }) => {
            let chars: Vec<char> = text.chars().collect();
            let mut out = String::with_capacity(text.len());
            out.extend(&chars[..head]);
            out.push(ELLIPSIS);
            out.extend(&chars[tail_start..]);
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_names_are_returned_unchanged() {
        assert_eq!(truncate_middle("nginx", 12), "nginx");
        // Exactly at the budget is not truncated.
        assert_eq!(truncate_middle("123456789012", 12), "123456789012");
    }

    #[test]
    fn long_names_keep_head_and_tail_around_an_ellipsis() {
        let out = truncate_middle("web-7d4b8c9f5d-x2k4p/nginx", 12);
        assert_eq!(out.width(), 12);
        assert!(out.starts_with("web"), "lost the workload prefix: {out:?}");
        assert!(out.ends_with("nginx"), "lost the container suffix: {out:?}");
        assert!(out.contains(ELLIPSIS));
    }

    #[test]
    fn tail_keeps_at_least_as_much_as_head() {
        // Odd leftover budget biases toward the tail (the specific suffix).
        let out = truncate_middle("aaaaaaaaaaXbbbbbbbbbb", 8);
        assert_eq!(out.width(), 8);
        let (head, tail) = out.split_once(ELLIPSIS).expect("ellipsis present");
        assert!(tail.chars().count() >= head.chars().count());
    }

    #[test]
    fn source_budget_clamps_to_min_and_max() {
        assert_eq!(source_budget(0), MIN_SOURCE_WIDTH);
        assert_eq!(source_budget(20), MIN_SOURCE_WIDTH); // 20/2 == 10 < floor
        assert_eq!(source_budget(48), 24); // 48/2, within range
        assert_eq!(source_budget(120), MAX_SOURCE_WIDTH); // 60 capped
    }
}
