//! Rendering for the modal workspace: tab line, split panes, status line,
//! prompt, and the help/detail overlays.

use std::sync::Arc;

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::{
    config::tui::ThemeConfig,
    event::Match,
    log::LogEntry,
    state::AppState,
    store::StoreStats,
    tui::{
        pane::{Pane, View},
        workspace::{Mode, Prompt},
    },
};

/// Rows the cursor keeps from the pane edge while scrolling (vim scrolloff).
const SCROLLOFF: usize = 2;

pub fn draw(state: &mut AppState, frame: &mut Frame) {
    let theme = state.theme.clone();
    let area = frame.area();
    frame.render_widget(Block::default().style(theme.surface_style()), area);

    let show_tabs = state.workspace.tabs.len() > 1;
    let [tab_area, content, status_area] = Layout::vertical([
        Constraint::Length(if show_tabs { 1 } else { 0 }),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    if show_tabs {
        draw_tab_line(state, frame, tab_area);
    }

    // Lay out the active tab's split tree and draw every pane.
    let mut rects: Vec<(crate::event::PaneId, Rect)> = Vec::new();
    {
        let tab = state.workspace.tab();
        tab.tree.layout(content, &mut rects);
    }
    let stats = state.store.stats();
    let mode = state.workspace.mode;
    let focused = state.workspace.tab().focused;
    for (pane_id, rect) in &rects {
        let Some(pane) = state.workspace.pane_mut(*pane_id) else {
            continue;
        };
        draw_pane(pane, frame, *rect, &theme, *pane_id == focused, mode);
    }

    draw_status_line(state, frame, status_area, &theme, stats);

    if state.workspace.focused_pane().detail_open {
        let pane_rect = state.workspace.focused_pane().rect;
        draw_detail_overlay(state.workspace.focused_pane(), frame, pane_rect, &theme);
    }
    if state.workspace.help_open {
        draw_help_overlay(frame, area, &theme);
    }
}

fn draw_tab_line(state: &AppState, frame: &mut Frame, area: Rect) {
    let theme = &state.theme;
    let mut spans: Vec<Span> = Vec::new();
    for (idx, tab) in state.workspace.tabs.iter().enumerate() {
        let label = format!(" {}:{} ", idx + 1, tab.name);
        if idx == state.workspace.active_tab {
            spans.push(Span::styled(
                label,
                Style::default()
                    .fg(theme.primary_accent_fg)
                    .add_modifier(Modifier::BOLD | Modifier::REVERSED),
            ));
        } else {
            spans.push(Span::styled(
                label,
                Style::default().fg(theme.border_unfocused_fg),
            ));
        }
        spans.push(Span::raw(" "));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn pane_title(pane: &Pane) -> String {
    let filter = if pane.filter.is_empty() {
        "*".to_string()
    } else {
        pane.filter.join(",")
    };
    let mut title = format!(" [{filter}]");
    match &pane.view {
        View::Results { term, progress, .. } => {
            title.push_str(&format!(" /{term}"));
            if let Some(progress) = progress
                && progress.scanned < progress.total
            {
                title.push_str(&format!(" {}/{}", progress.scanned, progress.total));
            }
        }
        View::Stream { .. } => {
            if let Some(term) = &pane.last_search {
                title.push_str(&format!(" /{term}({})", pane.hits.len()));
            }
        }
    }
    if pane.follow {
        title.push_str(" TAIL");
    }
    title.push(' ');
    title
}

fn draw_pane(
    pane: &mut Pane,
    frame: &mut Frame,
    rect: Rect,
    theme: &ThemeConfig,
    focused: bool,
    mode: Mode,
) {
    let border_style = if focused {
        Style::default().fg(theme.primary_accent_fg)
    } else {
        Style::default().fg(theme.border_unfocused_fg)
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(pane_title(pane));
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    pane.rect = inner;
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let entries = pane.view.entries();
    if entries.is_empty() {
        let note = pane.empty_note.as_deref().unwrap_or("waiting for logs…");
        let para = Paragraph::new(Line::from(Span::styled(
            note,
            Style::default().fg(theme.border_unfocused_fg),
        )))
        .centered();
        let mid = Rect::new(inner.x, inner.y + inner.height / 2, inner.width, 1);
        frame.render_widget(para, mid);
        return;
    }

    let height = inner.height as usize;
    let cursor_idx = pane.cursor_index().unwrap_or(entries.len() - 1);

    // Keep the cursor on screen with a small scrolloff margin.
    let mut scroll = pane.scroll.min(entries.len().saturating_sub(1));
    if cursor_idx < scroll + SCROLLOFF {
        scroll = cursor_idx.saturating_sub(SCROLLOFF);
    }
    if cursor_idx + SCROLLOFF >= scroll + height {
        scroll = (cursor_idx + SCROLLOFF + 1).saturating_sub(height);
    }
    scroll = scroll.min(entries.len().saturating_sub(height.min(entries.len())));
    pane.scroll = scroll;

    // Visual selection bounds (only on the focused pane).
    let selection = match (focused, mode) {
        (true, Mode::Visual { anchor }) => pane
            .cursor_seq
            .map(|cursor| (anchor.min(cursor), anchor.max(cursor))),
        _ => None,
    };

    let matches = match &pane.view {
        View::Results { matches, .. } => Some(matches),
        View::Stream { .. } => None,
    };

    let mut lines: Vec<Line> = Vec::with_capacity(height);
    for (offset, entry) in entries.iter().skip(scroll).take(height).enumerate() {
        let idx = scroll + offset;
        let is_cursor = idx == cursor_idx;
        let in_selection = selection.is_some_and(|(lo, hi)| entry.seq >= lo && entry.seq <= hi);
        let row_style = if is_cursor && focused {
            theme.selected_style()
        } else if in_selection {
            Style::default().bg(theme.log_selected_bg)
        } else if is_cursor {
            // Unfocused pane cursor: visible but muted.
            Style::default().add_modifier(Modifier::UNDERLINED)
        } else {
            Style::default()
        };
        let entry_matches = matches.and_then(|m| m.get(&entry.seq));
        lines.push(entry_line(
            entry,
            theme,
            row_style,
            entry_matches,
            inner.width,
        ));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Render one log entry as `HH:MM:SS LEVEL source │ msg`, applying fuzzy
/// match highlights to the message when present.
fn entry_line<'a>(
    entry: &'a Arc<LogEntry>,
    theme: &ThemeConfig,
    row_style: Style,
    matches: Option<&Vec<Match>>,
    width: u16,
) -> Line<'a> {
    let level_fg = theme.log_row_fg(entry.level);
    let dim = Style::default()
        .fg(theme.border_unfocused_fg)
        .patch(row_style);
    let level_style = Style::default().fg(level_fg).patch(row_style);

    let ts = entry.ts.format("%H:%M:%S").to_string();
    let level = entry
        .level
        .map(|level| format!("{level:<5}"))
        .unwrap_or_else(|| "     ".to_string());
    let source = truncate_pad(&entry.source.display_name, 10);

    let mut spans = vec![
        Span::styled(format!("{ts} "), dim),
        Span::styled(format!("{level} "), level_style),
        Span::styled(source, dim),
        Span::styled("│ ", dim),
    ];

    let msg_indices: Option<&Vec<u32>> = matches
        .and_then(|ms| ms.iter().find(|m| m.key == "msg"))
        .map(|m| &m.indices);
    let base_msg_style = Style::default().fg(level_fg).patch(row_style);
    match msg_indices {
        Some(indices) if !indices.is_empty() => {
            spans.extend(highlight_msg(
                &entry.msg,
                indices,
                base_msg_style,
                theme.match_style().patch(row_style),
            ));
        }
        _ => spans.push(Span::styled(entry.msg.as_str(), base_msg_style)),
    }

    // Pad the row so selection/cursor backgrounds span the full width.
    let used: usize = spans.iter().map(|s| s.content.chars().count()).sum();
    if (used as u16) < width {
        spans.push(Span::styled(" ".repeat(width as usize - used), row_style));
    }
    Line::from(spans)
}

/// Split `msg` into styled spans, applying `hl` to chars at match indices.
fn highlight_msg<'a>(msg: &'a str, indices: &[u32], base: Style, hl: Style) -> Vec<Span<'a>> {
    let mut spans = Vec::new();
    let mut run_start = 0usize;
    let mut run_highlighted = indices.contains(&0);
    for (char_idx, (byte_idx, _)) in msg.char_indices().enumerate() {
        let highlighted = indices.contains(&(char_idx as u32));
        if highlighted != run_highlighted {
            if byte_idx > run_start {
                spans.push(Span::styled(
                    &msg[run_start..byte_idx],
                    if run_highlighted { hl } else { base },
                ));
            }
            run_start = byte_idx;
            run_highlighted = highlighted;
        }
    }
    if run_start < msg.len() {
        spans.push(Span::styled(
            &msg[run_start..],
            if run_highlighted { hl } else { base },
        ));
    }
    spans
}

fn truncate_pad(s: &str, width: usize) -> String {
    let truncated: String = s.chars().take(width).collect();
    format!("{truncated:<width$} ")
}

fn mode_badge(mode: Mode, follow: bool) -> (&'static str, Color) {
    match mode {
        Mode::Normal if follow => (" TAIL ", Color::Green),
        Mode::Normal => (" NORMAL ", Color::Blue),
        Mode::Visual { .. } => (" VISUAL ", Color::Yellow),
        Mode::Search => (" SEARCH ", Color::Cyan),
        Mode::Command => (" COMMAND ", Color::Magenta),
    }
}

fn draw_status_line(
    state: &AppState,
    frame: &mut Frame,
    area: Rect,
    theme: &ThemeConfig,
    stats: StoreStats,
) {
    let ws = &state.workspace;
    let pane = ws.focused_pane();
    let (badge, badge_color) = mode_badge(ws.mode, pane.follow);
    let mut spans = vec![Span::styled(
        badge,
        Style::default()
            .fg(Color::Black)
            .bg(badge_color)
            .add_modifier(Modifier::BOLD),
    )];

    match ws.mode {
        Mode::Search => spans.extend(prompt_spans("/", &ws.prompt, theme)),
        Mode::Command => spans.extend(prompt_spans(":", &ws.prompt, theme)),
        _ => {
            let dim = Style::default().fg(theme.border_unfocused_fg);
            let text = Style::default();
            if let Some(seq) = pane.cursor_seq {
                spans.push(Span::styled(" seq ", dim));
                spans.push(Span::styled(format!("{seq}"), text));
                spans.push(Span::styled(format!("/{}", stats.bounds.1), dim));
            }
            spans.push(Span::styled(
                format!("  store {}/{}", stats.retained, stats.capacity),
                dim,
            ));
            if let Some(notice) = &ws.notice {
                spans.push(Span::styled(
                    format!("  {notice}"),
                    Style::default().fg(theme.primary_accent_fg),
                ));
            } else if let Some(term) = &pane.last_search {
                spans.push(Span::styled(
                    format!("  /{term} ({} hits, n/N to jump)", pane.hits.len()),
                    dim,
                ));
            }
            // Pending count/prefix feedback, vim-style, far right.
            let mut pending = String::new();
            if let Some(count) = ws.pending.count {
                pending.push_str(&count.to_string());
            }
            match ws.pending.prefix {
                Some(crate::tui::workspace::Prefix::G) => pending.push('g'),
                Some(crate::tui::workspace::Prefix::Window) => pending.push_str("^W"),
                None => {}
            }
            if !pending.is_empty() {
                spans.push(Span::styled(
                    format!("  {pending}"),
                    Style::default().fg(theme.secondary_accent_fg),
                ));
            }
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Prompt text with a reversed block cursor at the edit position.
fn prompt_spans<'a>(sigil: &'a str, prompt: &'a Prompt, theme: &ThemeConfig) -> Vec<Span<'a>> {
    let mut spans = vec![Span::styled(
        format!(" {sigil}"),
        Style::default().fg(theme.secondary_accent_fg),
    )];
    let chars: Vec<char> = prompt.buf.chars().collect();
    let before: String = chars[..prompt.cursor.min(chars.len())].iter().collect();
    let at: String = chars
        .get(prompt.cursor)
        .map(|c| c.to_string())
        .unwrap_or_else(|| " ".to_string());
    let after: String = chars[prompt.cursor.saturating_add(1).min(chars.len())..]
        .iter()
        .collect();
    spans.push(Span::raw(before));
    spans.push(Span::styled(
        at,
        Style::default().add_modifier(Modifier::REVERSED),
    ));
    spans.push(Span::raw(after));
    spans
}

fn draw_detail_overlay(pane: &Pane, frame: &mut Frame, pane_rect: Rect, theme: &ThemeConfig) {
    let Some(entry) = pane.cursor_entry() else {
        return;
    };
    let json = serde_json::to_string_pretty(&**entry).unwrap_or_default();
    let height = (pane_rect.height as f32 * 0.6) as u16;
    let rect = Rect::new(
        pane_rect.x,
        pane_rect.y + pane_rect.height.saturating_sub(height),
        pane_rect.width,
        height,
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.primary_accent_fg))
        .title(format!(" entry {} — j/k scroll · Esc close ", entry.seq));
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(json)
            .style(theme.surface_style())
            .scroll((pane.detail_scroll, 0))
            .block(block),
        rect,
    );
}

const HELP_TEXT: &str = "\
  TAIL/NORMAL ────────────────────────────────
  j k / ↓ ↑      move cursor (counts work: 5j)
  Ctrl-d Ctrl-u  half page · Ctrl-f Ctrl-b page
  gg G           oldest / newest entry
  F              follow tail again
  /              fuzzy search this pane
  n N            next / previous hit
  v V            visual select · y yank lines
  y              yank entry as JSON
  Enter          results: open in context
                 stream: entry detail
  Esc            leave results / clear

  WINDOWS & TABS ─────────────────────────────
  Ctrl-w v s     vsplit / split (clones pane)
  Ctrl-w h j k l move focus · Ctrl-w q close
  Ctrl-w o       only this pane
  gt gT          next / previous tab

  COMMANDS ───────────────────────────────────
  :filter api,db  pane source filter · :filter
  :vs :sp :q :qa :only :tabnew [name] :tabclose
  :tail :clear :help          Ctrl-c quits";

fn draw_help_overlay(frame: &mut Frame, area: Rect, theme: &ThemeConfig) {
    let width = 50.min(area.width);
    let height = 27.min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height - height) / 2,
        width,
        height,
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.primary_accent_fg))
        .title(" keys — Esc to close ");
    frame.render_widget(Clear, rect);
    frame.render_widget(
        Paragraph::new(HELP_TEXT)
            .style(theme.surface_style())
            .block(block),
        rect,
    );
}
