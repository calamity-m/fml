//! Rendering for the modal workspace, styled like a text editor rather than
//! a boxed dashboard: panes have no borders, vertical splits are separated
//! by a one-column gutter, and every pane carries a vim-style reversed
//! statusline. Only overlays (detail/help) use borders.
//!
//! Rows are styled through a per-char style buffer so cursor cell, charwise
//! visual selection, and fuzzy-match highlights compose over the same
//! [`row_text`] the yank path copies — what you see is what you yank.

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

use crate::{
    config::tui::ThemeConfig,
    state::AppState,
    store::StoreStats,
    tui::{
        pane::{MSG_CHAR_OFFSET, Pane, View, row_text},
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
    let [tab_area, content, cmd_area] = Layout::vertical([
        Constraint::Length(if show_tabs { 1 } else { 0 }),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .areas(area);

    if show_tabs {
        draw_tab_line(state, frame, tab_area);
    }

    // Lay out the active tab's split tree; render gutters between vsplits.
    let mut rects: Vec<(crate::event::PaneId, Rect)> = Vec::new();
    let mut gutters: Vec<Rect> = Vec::new();
    state
        .workspace
        .tab()
        .tree
        .layout(content, &mut rects, &mut gutters);
    for gutter in gutters {
        let bar = "│\n".repeat(gutter.height as usize);
        frame.render_widget(
            Paragraph::new(bar.trim_end()).style(Style::default().fg(theme.border_unfocused_fg)),
            gutter,
        );
    }

    let stats = state.store.stats();
    let mode = state.workspace.mode;
    let focused = state.workspace.tab().focused;
    for (pane_id, rect) in &rects {
        let Some(pane) = state.workspace.pane_mut(*pane_id) else {
            continue;
        };
        draw_pane(pane, frame, *rect, &theme, *pane_id == focused, mode, stats);
    }

    draw_cmdline(state, frame, cmd_area, &theme, stats);

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

fn draw_pane(
    pane: &mut Pane,
    frame: &mut Frame,
    rect: Rect,
    theme: &ThemeConfig,
    focused: bool,
    mode: Mode,
    stats: StoreStats,
) {
    if rect.height == 0 || rect.width == 0 {
        return;
    }
    // Last row is the pane's statusline; the rest is log content.
    let [content, status] =
        Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(rect);
    draw_pane_statusline(pane, frame, status, theme, focused, stats);

    pane.rect = content;
    if content.height == 0 {
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
        let mid = Rect::new(content.x, content.y + content.height / 2, content.width, 1);
        frame.render_widget(para, mid);
        return;
    }

    let height = content.height as usize;
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

    let matches = match &pane.view {
        View::Results { matches, .. } => Some(matches),
        View::Stream { .. } => None,
    };

    let mut lines: Vec<Line> = Vec::with_capacity(height);
    for (offset, entry) in entries.iter().skip(scroll).take(height).enumerate() {
        let idx = scroll + offset;
        let is_cursor_row = idx == cursor_idx;
        let width = content.width as usize;

        let text = row_text(entry);
        let chars: Vec<char> = text.chars().take(width).collect();
        let level_style = Style::default().fg(theme.log_row_fg(entry.level));
        let dim = Style::default().fg(theme.border_unfocused_fg);

        // Base style per char: frame fields dimmed, level + msg in the
        // row's level color.
        let mut styles: Vec<Style> = (0..chars.len())
            .map(|col| {
                if (9..14).contains(&col) || col >= MSG_CHAR_OFFSET {
                    level_style
                } else {
                    dim
                }
            })
            .collect();

        // Fuzzy match highlights (msg-relative char indices).
        if let Some(entry_matches) = matches.and_then(|m| m.get(&entry.seq))
            && let Some(msg_match) = entry_matches.iter().find(|m| m.key == "msg")
        {
            let hl = theme.match_style();
            for &idx in &msg_match.indices {
                let col = MSG_CHAR_OFFSET + idx as usize;
                if col < styles.len() {
                    styles[col] = styles[col].patch(hl);
                }
            }
        }

        // Visual selection (focused pane only).
        let mut row_bg = Style::default();
        if focused {
            match mode {
                Mode::Visual {
                    anchor_seq,
                    linewise: true,
                    ..
                } => {
                    if let Some(cursor_seq) = pane.cursor_seq {
                        let (lo, hi) = (anchor_seq.min(cursor_seq), anchor_seq.max(cursor_seq));
                        if entry.seq >= lo && entry.seq <= hi {
                            row_bg = Style::default().bg(theme.log_selected_bg);
                        }
                    }
                }
                Mode::Visual {
                    anchor_seq,
                    anchor_col,
                    linewise: false,
                } => {
                    if let Some((from, to)) = pane.charwise_row_range(entry, anchor_seq, anchor_col)
                    {
                        let sel = Style::default().bg(theme.log_selected_bg);
                        for style in styles.iter_mut().take((to + 1).min(chars.len())).skip(from) {
                            *style = style.patch(sel);
                        }
                    }
                }
                _ => {}
            }
        }

        // Cursorline tint and the block cursor cell.
        if is_cursor_row && focused {
            row_bg = theme.selected_style();
            let col = pane.effective_col();
            if col < styles.len() {
                styles[col] = styles[col].add_modifier(Modifier::REVERSED);
            }
        } else if is_cursor_row {
            row_bg = Style::default().add_modifier(Modifier::UNDERLINED);
        }
        if row_bg != Style::default() {
            for style in &mut styles {
                *style = row_bg.patch(*style);
            }
        }

        lines.push(spans_from_styles(&chars, &styles, row_bg, width));
    }
    frame.render_widget(Paragraph::new(lines), content);
}

/// Group adjacent equal styles into spans; pad rows that carry a background
/// to the full pane width so the tint spans the line.
fn spans_from_styles(
    chars: &[char],
    styles: &[Style],
    row_bg: Style,
    width: usize,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_style = styles.first().copied().unwrap_or_default();
    for (c, style) in chars.iter().zip(styles) {
        if *style != run_style && !run.is_empty() {
            spans.push(Span::styled(std::mem::take(&mut run), run_style));
        }
        run_style = *style;
        run.push(*c);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, run_style));
    }
    if row_bg != Style::default() && chars.len() < width {
        spans.push(Span::styled(" ".repeat(width - chars.len()), row_bg));
    }
    Line::from(spans)
}

/// The vim-style reversed statusline at the bottom of each pane.
fn draw_pane_statusline(
    pane: &Pane,
    frame: &mut Frame,
    area: Rect,
    theme: &ThemeConfig,
    focused: bool,
    stats: StoreStats,
) {
    let style = if focused {
        Style::default()
            .fg(theme.primary_accent_fg)
            .add_modifier(Modifier::REVERSED | Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.border_unfocused_fg)
            .add_modifier(Modifier::REVERSED)
    };

    let filter = if pane.filter.is_empty() {
        "*".to_string()
    } else {
        pane.filter.join(",")
    };
    let mut left = format!(" [{filter}]");
    match &pane.view {
        View::Results { term, progress, .. } => {
            left.push_str(&format!(" /{term}"));
            if let Some(progress) = progress
                && progress.scanned < progress.total
            {
                left.push_str(&format!(" {}/{}", progress.scanned, progress.total));
            }
        }
        View::Stream { .. } => {
            if let Some(term) = &pane.last_search {
                left.push_str(&format!(" /{term}({})", pane.hits.len()));
            }
        }
    }
    if pane.follow {
        left.push_str(" TAIL");
    }

    let right = match pane.cursor_seq {
        Some(seq) => format!("{seq}/{} ", stats.bounds.1),
        None => String::new(),
    };
    let pad = (area.width as usize).saturating_sub(left.chars().count() + right.chars().count());
    let bar = format!("{left}{}{right}", " ".repeat(pad));
    frame.render_widget(Paragraph::new(bar).style(style), area);
}

fn mode_badge(mode: Mode, follow: bool) -> (&'static str, Color) {
    match mode {
        Mode::Normal if follow => (" TAIL ", Color::Green),
        Mode::Normal => (" NORMAL ", Color::Blue),
        Mode::Visual {
            linewise: false, ..
        } => (" VISUAL ", Color::Yellow),
        Mode::Visual { linewise: true, .. } => (" V-LINE ", Color::Yellow),
        Mode::Search => (" SEARCH ", Color::Cyan),
        Mode::Command => (" COMMAND ", Color::Magenta),
    }
}

/// The global bottom line: mode badge plus prompt, notice, or store info.
fn draw_cmdline(
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
            spans.push(Span::styled(
                format!(" store {}/{}", stats.retained, stats.capacity),
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
  h l 0 $ w b    move within the line
  Ctrl-d Ctrl-u  half page · Ctrl-f Ctrl-b page
  gg G           oldest / newest entry
  F              follow tail again
  /              fuzzy search this pane
  n N            next / previous hit
  v V            visual chars / lines · y yanks
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
    let height = 28.min(area.height);
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
