//! Rendering for the modal workspace, styled like a text editor rather than
//! a boxed dashboard: panes have no borders, vertical splits are separated
//! by a one-column gutter, and every pane carries a vim-style reversed
//! statusline. Only overlays (detail/help) use borders.
//!
//! Rows are styled through a per-char style buffer so cursor cell, charwise
//! visual selection, and fuzzy-match highlights compose over the same
//! [`row_text`] the yank path copies — what you see is what you yank.

use std::{collections::HashMap, sync::Arc};

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
        layout::{DisplayRow, col_to_display, layout_entry},
        pane::{MSG_CHAR_OFFSET, Pane, ScrollAnchor, View, row_text},
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
    if state.workspace.picker.is_some() {
        draw_picker_overlay(state, frame, content, &theme);
    }
    if state.workspace.help_open {
        draw_help_overlay(frame, area, &theme);
    }
}

/// Centered fuzzy source picker: query line on top, narrowed source rows
/// below, toggled rows marked, highlighted row reversed.
fn draw_picker_overlay(state: &AppState, frame: &mut Frame, area: Rect, theme: &ThemeConfig) {
    let Some(picker) = &state.workspace.picker else {
        return;
    };
    let rows = picker.rows(&state.producer.sources);

    let width = 64.min(area.width);
    let height = (rows.len() as u16 + 3)
        .clamp(5, (area.height as f32 * 0.7) as u16)
        .min(area.height);
    let rect = Rect::new(
        area.x + (area.width - width) / 2,
        area.y + (area.height.saturating_sub(height)) / 2,
        width,
        height,
    );
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(theme.primary_accent_fg))
        .title(" sources — Tab toggle · C-a all · Enter apply ");
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block.style(theme.surface_style()), rect);
    if inner.height == 0 {
        return;
    }

    let mut lines: Vec<Line> = Vec::with_capacity(inner.height as usize);
    lines.push(Line::from(prompt_spans(">", &picker.query, theme)));

    let visible = (inner.height as usize).saturating_sub(1);
    let cursor = picker.cursor.min(rows.len().saturating_sub(1));
    let scroll = cursor.saturating_sub(visible.saturating_sub(1));
    if rows.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no sources match",
            Style::default().fg(theme.border_unfocused_fg),
        )));
    }
    for (offset, source) in rows.iter().skip(scroll).take(visible).enumerate() {
        let idx = scroll + offset;
        let toggled = picker.selected.contains(&source.id);
        let marker = if toggled { "●" } else { " " };
        let origin = match &source.group {
            Some(group) => format!("{}/{}", source.producer, group),
            None => source.producer.clone(),
        };
        let mut spans = vec![
            Span::styled(
                format!(" {marker} "),
                Style::default().fg(theme.primary_accent_fg),
            ),
            Span::raw(source.display_name.clone()),
            Span::styled(
                format!("  {origin} ({})", source.id),
                Style::default().fg(theme.border_unfocused_fg),
            ),
        ];
        if idx == cursor {
            spans = spans
                .into_iter()
                .map(|span| {
                    let style = span.style.add_modifier(Modifier::REVERSED);
                    Span::styled(span.content, style)
                })
                .collect();
        }
        lines.push(Line::from(spans));
    }
    frame.render_widget(Paragraph::new(lines), inner);
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
    let width = content.width as usize;
    let cursor_idx = pane.cursor_index().unwrap_or(entries.len() - 1);
    let effective_col = pane.effective_col();

    // Lay out the visible window as `(entry_index, display_row)` pairs and
    // persist the matching scroll anchor. Wrap off keeps the historical
    // entry-index scroll (one truncated row per entry, byte-for-byte); wrap on
    // walks display rows so the cursor's display row stays on screen.
    let (anchor, visible): (ScrollAnchor, Vec<(usize, DisplayRow)>) = if pane.line_wrap {
        wrap_window(
            entries,
            width,
            MSG_CHAR_OFFSET,
            cursor_idx,
            effective_col,
            height,
            SCROLLOFF,
            pane.scroll,
        )
    } else {
        let mut scroll = pane.scroll.entry().min(entries.len().saturating_sub(1));
        if cursor_idx < scroll + SCROLLOFF {
            scroll = cursor_idx.saturating_sub(SCROLLOFF);
        }
        if cursor_idx + SCROLLOFF >= scroll + height {
            scroll = (cursor_idx + SCROLLOFF + 1).saturating_sub(height);
        }
        scroll = scroll.min(entries.len().saturating_sub(height.min(entries.len())));
        let rows = entries
            .iter()
            .enumerate()
            .skip(scroll)
            .take(height)
            .map(|(idx, entry)| (idx, layout_entry(&row_text(entry), width, 0, false)[0]))
            .collect();
        (ScrollAnchor::Entry(scroll), rows)
    };
    pane.scroll = anchor;

    let matches = match &pane.view {
        View::Results { matches, .. } => Some(matches),
        View::Stream { .. } => None,
    };

    // An entry can occupy several consecutive display rows; compute its full
    // per-char style buffer once and slice each display row out of it. Wrap off
    // only ever shows one display row truncated to `width`, so cap the styled
    // span there instead of styling the whole (possibly tens-of-KB) log line.
    let style_limit = if pane.line_wrap { usize::MAX } else { width };
    let mut lines: Vec<Line> = Vec::with_capacity(height);
    let mut cached: Option<(usize, Vec<char>, Vec<Style>, Style)> = None;
    for (entry_idx, drow) in visible {
        if cached.as_ref().is_none_or(|c| c.0 != entry_idx) {
            let entry = &entries[entry_idx];
            let is_cursor_row = entry_idx == cursor_idx;
            let (chars, styles, row_bg) = entry_row_styles(
                entry,
                pane,
                theme,
                focused,
                mode,
                matches,
                is_cursor_row,
                effective_col,
                style_limit,
            );
            cached = Some((entry_idx, chars, styles, row_bg));
        }
        let (_, chars, styles, row_bg) = cached.as_ref().unwrap();
        let start = drow.start_col.min(chars.len());
        let end = drow.end_col.min(chars.len());
        lines.push(spans_from_styles(
            &chars[start..end],
            &styles[start..end],
            *row_bg,
            drow.indent,
            width,
        ));
    }
    frame.render_widget(Paragraph::new(lines), content);
}

/// Build the full per-char style buffer for one entry's `row_text`.
///
/// Styles are indexed by logical char offset (not truncated to the pane width)
/// so the wrap renderer can slice any display row out of them. The returned
/// `row_bg` is the entry-wide cursorline/selection tint to pad each display row
/// with. The block cursor's reversed cell is patched at `effective_col`; with
/// wrap off, a column past the pane width stays out of the single visible slice
/// (the pre-existing off-screen-cursor behavior, unchanged).
///
/// `limit` caps how many leading chars are collected and styled. Wrap on passes
/// `usize::MAX` (the wrap renderer may slice any display row); wrap off passes
/// the pane width so the bounded single-row work is preserved.
#[allow(clippy::too_many_arguments)]
fn entry_row_styles(
    entry: &LogEntry,
    pane: &Pane,
    theme: &ThemeConfig,
    focused: bool,
    mode: Mode,
    matches: Option<&HashMap<u64, Vec<Match>>>,
    is_cursor_row: bool,
    effective_col: usize,
    limit: usize,
) -> (Vec<char>, Vec<Style>, Style) {
    let chars: Vec<char> = row_text(entry).chars().take(limit).collect();
    let level_style = Style::default().fg(theme.log_row_fg(entry.level));
    let dim = Style::default().fg(theme.border_unfocused_fg);

    // Base style per char: frame fields dimmed, level + msg in the row's color.
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
                if let Some((from, to)) = pane.charwise_row_range(entry, anchor_seq, anchor_col) {
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
        if effective_col < styles.len() {
            styles[effective_col] = styles[effective_col].add_modifier(Modifier::REVERSED);
        }
    } else if is_cursor_row {
        row_bg = Style::default().add_modifier(Modifier::UNDERLINED);
    }
    if row_bg != Style::default() {
        for style in &mut styles {
            *style = row_bg.patch(*style);
        }
    }

    (chars, styles, row_bg)
}

/// Lazily-laid-out wrap geometry over a pane's entries, memoized per entry so a
/// giant entry is only wrapped once per frame even as the anchor walk revisits
/// it.
struct WrapCtx<'a> {
    entries: &'a [Arc<LogEntry>],
    width: usize,
    indent: usize,
    cache: HashMap<usize, Vec<DisplayRow>>,
}

impl WrapCtx<'_> {
    fn rows(&mut self, entry: usize) -> &[DisplayRow] {
        self.cache.entry(entry).or_insert_with(|| {
            layout_entry(
                &row_text(&self.entries[entry]),
                self.width,
                self.indent,
                true,
            )
        })
    }

    fn len_at(&mut self, entry: usize) -> usize {
        self.rows(entry).len()
    }

    /// One display row forward, crossing into the next entry at its first row.
    fn step_fwd(&mut self, pos: (usize, usize)) -> Option<(usize, usize)> {
        if pos.1 + 1 < self.len_at(pos.0) {
            Some((pos.0, pos.1 + 1))
        } else if pos.0 + 1 < self.entries.len() {
            Some((pos.0 + 1, 0))
        } else {
            None
        }
    }

    /// One display row back, crossing into the previous entry at its last row.
    fn step_back(&mut self, pos: (usize, usize)) -> Option<(usize, usize)> {
        if pos.1 > 0 {
            Some((pos.0, pos.1 - 1))
        } else if pos.0 > 0 {
            Some((pos.0 - 1, self.len_at(pos.0 - 1) - 1))
        } else {
            None
        }
    }
}

/// Wrap-on scroll: keep the cursor's *display row* on screen while measuring
/// scroll in display rows. Anchored on `(entry_index, display_row)` rather than
/// a global row count (which would be circular), with a bounded backward walk
/// from the previous anchor. Returns the new anchor plus the ordered visible
/// `(entry_index, display_row)` pairs filling the viewport.
#[allow(clippy::too_many_arguments)]
fn wrap_window(
    entries: &[Arc<LogEntry>],
    width: usize,
    indent: usize,
    cursor_idx: usize,
    cursor_col: usize,
    height: usize,
    scrolloff: usize,
    prev: ScrollAnchor,
) -> (ScrollAnchor, Vec<(usize, DisplayRow)>) {
    let mut ctx = WrapCtx {
        entries,
        width,
        indent,
        cache: HashMap::new(),
    };

    let cur_disp = col_to_display(ctx.rows(cursor_idx), cursor_col).0;
    let cursor_pos = (cursor_idx, cur_disp);

    // Normalize the previous anchor into a valid (entry, row) for this mode.
    let mut top = match prev {
        ScrollAnchor::Display { entry, row } => {
            let entry = entry.min(entries.len() - 1);
            (entry, row.min(ctx.len_at(entry).saturating_sub(1)))
        }
        ScrollAnchor::Entry(entry) => (entry.min(entries.len() - 1), 0),
    };

    // Bound the backward walk: an anchor more than a viewport of entries above
    // the cursor is stale (every entry is at least one display row), snap near.
    if cursor_idx > top.0 + height + 1 {
        top = (cursor_idx, 0);
    }
    // The anchor must never sit below the cursor.
    if top > cursor_pos {
        top = cursor_pos;
    }

    // Display-row distance from the anchor down to the cursor.
    let mut dist = 0usize;
    let mut walk = top;
    while walk != cursor_pos {
        walk = ctx
            .step_fwd(walk)
            .expect("cursor is at or below the anchor by construction");
        dist += 1;
    }

    // Bottom scrolloff: if the cursor sits too low, scroll down (anchor moves
    // forward) until it is within `height - 1 - scrolloff` of the top.
    let max_below = height.saturating_sub(1).saturating_sub(scrolloff);
    while dist > max_below {
        top = ctx
            .step_fwd(top)
            .expect("a lower cursor leaves room to advance");
        dist -= 1;
    }
    // Top scrolloff: keep context above the cursor when any exists.
    while dist < scrolloff {
        match ctx.step_back(top) {
            Some(prev_top) => {
                top = prev_top;
                dist += 1;
            }
            None => break,
        }
    }
    // Avoid blank space at the bottom: when fewer than `height` rows remain
    // below the anchor, pull it back to fill (mirrors the entry-scroll clamp).
    let mut avail = 1usize;
    let mut probe = top;
    while avail < height {
        match ctx.step_fwd(probe) {
            Some(next) => {
                probe = next;
                avail += 1;
            }
            None => break,
        }
    }
    let mut deficit = height.saturating_sub(avail);
    while deficit > 0 {
        match ctx.step_back(top) {
            Some(prev_top) => {
                top = prev_top;
                deficit -= 1;
            }
            None => break,
        }
    }

    // Emit visible display rows from the anchor forward.
    let mut out = Vec::with_capacity(height);
    let mut pos = top;
    loop {
        let row = ctx.rows(pos.0)[pos.1];
        out.push((pos.0, row));
        if out.len() == height {
            break;
        }
        match ctx.step_fwd(pos) {
            Some(next) => pos = next,
            None => break,
        }
    }

    (
        ScrollAnchor::Display {
            entry: top.0,
            row: top.1,
        },
        out,
    )
}

/// Group adjacent equal styles into spans, prefixed by the row's hanging indent
/// and padded to the full pane width when the row carries a background, so the
/// cursorline/selection tint spans the whole display row (indent included).
fn spans_from_styles(
    chars: &[char],
    styles: &[Style],
    row_bg: Style,
    indent: usize,
    width: usize,
) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    if indent > 0 {
        spans.push(Span::styled(" ".repeat(indent), row_bg));
    }
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
    let used = indent + chars.len();
    if row_bg != Style::default() && used < width {
        spans.push(Span::styled(" ".repeat(width - used), row_bg));
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
    let (badge, badge_color) = if ws.picker.is_some() {
        (" PICKER ", Color::White)
    } else {
        mode_badge(ws.mode, pane.follow)
    };
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
  F              follow tail · go live on a search
  /              fuzzy search (TAIL = live, else frozen)
  n N            next / previous hit (first jump freezes)
  v V            visual chars / lines · y yanks
  y              yank entry as JSON
  W              toggle line wrap (:wrap · :set wrap/nowrap)
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
  :filter =Name   exact match · Tab completes
  :sources        fuzzy source picker
  :vs vsplit · :sp :hs hsplit (stacked)
  :q :qa :only :tabnew [name] :tabclose
  :tail go live · :refresh re-rank frozen search
  :clear :help                Ctrl-c quits";

fn draw_help_overlay(frame: &mut Frame, area: Rect, theme: &ThemeConfig) {
    let width = 50.min(area.width);
    let height = 30.min(area.height);
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

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::DateTime;
    use ratatui::{Terminal, backend::TestBackend, buffer::Buffer};

    use super::*;
    use crate::{
        event::{Match, PaneId},
        log::{LogLevel, Source},
        tui::pane::View,
    };

    const W: u16 = 40;
    const INDENT: usize = MSG_CHAR_OFFSET;

    fn entry(seq: u64, msg: &str) -> Arc<LogEntry> {
        Arc::new(LogEntry {
            seq,
            msg: msg.to_string(),
            // Fixed 00:00:00 timestamp so row_text is deterministic.
            ts: DateTime::from_timestamp(0, 0).unwrap(),
            level: Some(LogLevel::Info),
            source: Source {
                producer: "p".to_string(),
                id: "s".to_string(),
                display_name: "src".to_string(),
                group: None,
            },
            fields: HashMap::new(),
        })
    }

    fn stream_pane(entries: Vec<Arc<LogEntry>>, cursor: u64) -> Pane {
        let mut pane = Pane::new(PaneId(1));
        pane.follow = false;
        pane.cursor_seq = Some(cursor);
        pane.view = View::Stream { entries };
        pane
    }

    fn render(pane: &mut Pane, h: u16, focused: bool, mode: Mode) -> Buffer {
        let backend = TestBackend::new(W, h);
        let mut terminal = Terminal::new(backend).unwrap();
        let theme = ThemeConfig::default();
        let stats = StoreStats {
            retained: 1,
            capacity: 100,
            bounds: (1, 99),
        };
        terminal
            .draw(|frame| draw_pane(pane, frame, frame.area(), &theme, focused, mode, stats))
            .unwrap();
        terminal.backend().buffer().clone()
    }

    fn row_string(buf: &Buffer, y: u16) -> String {
        (buf.area.left()..buf.area.right())
            .map(|x| buf.cell((x, y)).unwrap().symbol())
            .collect()
    }

    // ---- D2: wrap-off golden gate -------------------------------------

    #[test]
    fn wrap_off_renders_single_truncated_row_byte_for_byte() {
        let e = entry(1, "hello world this message is far longer than forty cols");
        let mut pane = stream_pane(vec![e.clone()], 1);
        let buf = render(&mut pane, 5, true, Mode::Normal);

        let expected: String = row_text(&e).chars().take(W as usize).collect();
        assert_eq!(row_string(&buf, 0).trim_end(), expected.trim_end());
        // No wrapping: the second content row stays empty.
        assert_eq!(row_string(&buf, 1).trim_end(), "");
    }

    // ---- D2: wrap-on layout -------------------------------------------

    #[test]
    fn wrap_on_continuation_has_hanging_indent_and_no_frame() {
        let e = entry(1, "abcdefghijklmnopqrstuvwxyz0123456789");
        let rows = layout_entry(&row_text(&e), W as usize, INDENT, true);
        assert!(rows.len() >= 2, "entry must wrap for this test");

        let mut pane = stream_pane(vec![e.clone()], 1);
        pane.line_wrap = true;
        let buf = render(&mut pane, 6, false, Mode::Normal);

        let text: Vec<char> = row_text(&e).chars().collect();
        // First row carries the timestamp frame.
        assert!(row_string(&buf, 0).starts_with("00:00:00"));
        // Continuation row: indent spaces, then the next logical slice.
        let cont = &rows[1];
        let line1 = row_string(&buf, 1);
        assert_eq!(&line1[..INDENT], &" ".repeat(INDENT));
        let expected_slice: String = text[cont.start_col..cont.end_col].iter().collect();
        assert!(line1[INDENT..].starts_with(&expected_slice));
    }

    #[test]
    fn wrap_on_cursor_lands_on_continuation_display_row() {
        let e = entry(1, "abcdefghijklmnopqrstuvwxyz0123456789");
        let mut pane = stream_pane(vec![e.clone()], 1);
        pane.line_wrap = true;
        // A column in the second display row.
        pane.cursor_col = 45;
        let rows = layout_entry(&row_text(&e), W as usize, INDENT, true);
        let (r, c) = col_to_display(&rows, 45);
        assert_eq!(r, 1, "col 45 should fall on the first continuation row");

        let buf = render(&mut pane, 6, true, Mode::Normal);
        let cell = buf.cell((c as u16, r as u16)).unwrap();
        assert!(
            cell.modifier.contains(Modifier::REVERSED),
            "block cursor cell should be reversed on the continuation row"
        );
    }

    #[test]
    fn wrap_on_fuzzy_highlight_maps_to_continuation_row() {
        let e = entry(1, "abcdefghijklmnopqrstuvwxyz0123456789");
        let mut matches: HashMap<u64, Vec<Match>> = HashMap::new();
        // msg index 20 → logical col MSG_CHAR_OFFSET + 20, on a continuation row.
        matches.insert(
            1,
            vec![Match {
                key: "msg".to_string(),
                indices: vec![20],
            }],
        );
        let mut pane = stream_pane(vec![e.clone()], 1);
        pane.line_wrap = true;
        pane.view = View::Results {
            entries: vec![e.clone()],
            matches,
            progress: None,
            term: "x".to_string(),
        };

        let rows = layout_entry(&row_text(&e), W as usize, INDENT, true);
        let (r, c) = col_to_display(&rows, MSG_CHAR_OFFSET + 20);
        assert!(r >= 1, "match should land on a continuation row");

        let buf = render(&mut pane, 6, false, Mode::Normal);
        let theme = ThemeConfig::default();
        if let Some(fg) = theme.match_style().fg {
            assert_eq!(buf.cell((c as u16, r as u16)).unwrap().fg, fg);
        }
    }

    #[test]
    fn wrap_on_charwise_tint_spans_wrap_boundary() {
        let theme = ThemeConfig::default();
        let e1 = entry(1, "abcdefghijklmnopqrstuvwxyz0123456789");
        let e2 = entry(2, "short");
        let mut pane = stream_pane(vec![e1.clone(), e2.clone()], 2);
        pane.line_wrap = true;
        pane.cursor_col = 5;
        // Selection from (e1, col 35) down into e2 — e1 is the start entry, so
        // its tint runs col 35..end across the wrap boundary.
        let mode = Mode::Visual {
            anchor_seq: 1,
            anchor_col: 35,
            linewise: false,
        };
        let buf = render(&mut pane, 8, true, mode);

        // col 38 is on the first display row; logical col 43 on the second.
        assert_eq!(buf.cell((38, 0)).unwrap().bg, theme.log_selected_bg);
        let rows = layout_entry(&row_text(&e1), W as usize, INDENT, true);
        let (r, c) = col_to_display(&rows, 43);
        assert_eq!(
            buf.cell((c as u16, r as u16)).unwrap().bg,
            theme.log_selected_bg
        );
        // A column before the selection start is not tinted.
        assert_ne!(buf.cell((30, 0)).unwrap().bg, theme.log_selected_bg);
    }

    #[test]
    fn wrap_on_linewise_tint_pads_full_rows_including_indent() {
        let theme = ThemeConfig::default();
        let e = entry(1, "abcdefghijklmnopqrstuvwxyz0123456789");
        let rows = layout_entry(&row_text(&e), W as usize, INDENT, true);
        let mut pane = stream_pane(vec![e.clone()], 1);
        pane.line_wrap = true;
        let mode = Mode::Visual {
            anchor_seq: 1,
            anchor_col: 0,
            linewise: true,
        };
        let buf = render(&mut pane, 8, true, mode);

        // Every display row of the entry is tinted across the full width,
        // including the hanging-indent gap and trailing padding.
        for y in 0..rows.len() as u16 {
            for x in 0..W {
                assert_eq!(
                    buf.cell((x, y)).unwrap().bg,
                    theme.log_selected_bg,
                    "cell ({x},{y}) should carry the selection background"
                );
            }
        }
    }

    // ---- D3: wrap-aware scroll ----------------------------------------

    #[test]
    fn wrap_on_tall_entry_keeps_cursor_display_row_visible() {
        // One entry taller than the viewport.
        let long: String = "abcdefghij".repeat(20); // 200 chars
        let e = entry(1, &long);
        let mut pane = stream_pane(vec![e], 1);
        pane.line_wrap = true;

        // Cursor parked at the end → the last display row must be on screen.
        pane.cursor_col = usize::MAX;
        let buf = render(&mut pane, 5, true, Mode::Normal);
        let reversed = (0..buf.area.height)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .any(|(x, y)| {
                buf.cell((x, y))
                    .unwrap()
                    .modifier
                    .contains(Modifier::REVERSED)
            });
        assert!(reversed, "cursor on the last display row must be visible");

        // Cursor at the start → the first display row (with the frame) shows.
        pane.cursor_col = 0;
        let buf = render(&mut pane, 5, true, Mode::Normal);
        assert!(row_string(&buf, 0).starts_with("00:00:00"));
        assert!(
            buf.cell((0, 0))
                .unwrap()
                .modifier
                .contains(Modifier::REVERSED)
        );
    }

    #[test]
    fn wrap_off_scroll_keeps_cursor_visible() {
        let entries: Vec<_> = (1..=20).map(|s| entry(s, "msg")).collect();
        let mut pane = stream_pane(entries, 10);
        // Wrap off, cursor in the middle: it must remain rendered.
        let buf = render(&mut pane, 6, true, Mode::Normal);
        let reversed = (0..buf.area.height)
            .flat_map(|y| (0..W).map(move |x| (x, y)))
            .any(|(x, y)| {
                buf.cell((x, y))
                    .unwrap()
                    .modifier
                    .contains(Modifier::REVERSED)
            });
        assert!(reversed, "cursor entry should stay on screen with wrap off");
    }
}
