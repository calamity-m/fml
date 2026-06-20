//! Tabs, split trees, and the modal state shared across panes.
//!
//! A [`Workspace`] holds tabs; each [`Tab`] holds a binary-ish split tree of
//! pane ids plus the panes themselves. Splitting along the same axis extends
//! the existing split (vim-like even thirds) instead of nesting.

use std::collections::HashSet;

use ratatui::layout::{Direction, Rect};

use crate::event::PaneId;
use crate::log::{Source, SourceId};
use crate::tui::pane::Pane;

/// Global input mode. `TAIL` is not a mode: it is the focused pane's
/// `follow` flag, surfaced by the status line while mode is `Normal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    /// Visual selection anchored at a position in the focused pane.
    /// `v` selects characters; `V` selects whole lines.
    Visual {
        anchor_seq: u64,
        anchor_col: usize,
        linewise: bool,
    },
    /// Typing into the `/` prompt; results stream live into the pane.
    Search,
    /// Typing into the `:` prompt.
    Command,
}

/// Multi-key sequences awaiting their second key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prefix {
    /// `g` — awaiting `g`/`t`/`T`.
    G,
    /// `Ctrl-w` — awaiting a window command.
    Window,
}

/// Count and prefix state for the key currently being composed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Pending {
    pub count: Option<u32>,
    pub prefix: Option<Prefix>,
}

impl Pending {
    pub fn take_count(&mut self) -> u32 {
        self.count.take().unwrap_or(1).max(1)
    }

    pub fn clear(&mut self) {
        *self = Pending::default();
    }

    pub fn is_empty(&self) -> bool {
        *self == Pending::default()
    }
}

/// Minimal single-line editor backing the `/` and `:` prompts.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Prompt {
    pub buf: String,
    /// Cursor as a char offset into `buf`.
    pub cursor: usize,
}

impl Prompt {
    pub fn reset(&mut self) {
        self.buf.clear();
        self.cursor = 0;
    }

    fn byte_cursor(&self) -> usize {
        self.buf
            .char_indices()
            .nth(self.cursor)
            .map(|(idx, _)| idx)
            .unwrap_or(self.buf.len())
    }

    pub fn insert(&mut self, c: char) {
        let at = self.byte_cursor();
        self.buf.insert(at, c);
        self.cursor += 1;
    }

    pub fn insert_str(&mut self, s: &str) {
        let at = self.byte_cursor();
        self.buf.insert_str(at, s);
        self.cursor += s.chars().count();
    }

    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor -= 1;
        let at = self.byte_cursor();
        self.buf.remove(at);
    }

    pub fn left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    pub fn right(&mut self) {
        self.cursor = (self.cursor + 1).min(self.buf.chars().count());
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.buf.chars().count();
    }
}

/// Modal fuzzy source picker (`:sources`): type to narrow, Tab to toggle,
/// Enter writes the focused pane's filter as exact `=name` patterns.
#[derive(Debug, Default)]
pub struct SourcePicker {
    pub query: Prompt,
    /// Highlighted row index within the currently narrowed rows.
    pub cursor: usize,
    /// Toggled source ids (kept across query edits).
    pub selected: HashSet<SourceId>,
}

impl SourcePicker {
    /// Live sources narrowed by the query (case-insensitive substring over
    /// name, id, producer, and group).
    pub fn rows<'a>(&self, sources: &'a [Source]) -> Vec<&'a Source> {
        let query = self.query.buf.to_lowercase();
        sources
            .iter()
            .filter(|source| {
                query.is_empty()
                    || source.display_name.to_lowercase().contains(&query)
                    || source.id.to_lowercase().contains(&query)
                    || source.producer.to_lowercase().contains(&query)
                    || source
                        .group
                        .as_deref()
                        .is_some_and(|group| group.to_lowercase().contains(&query))
            })
            .collect()
    }
}

/// In-flight Tab completion on the `:` prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    pub candidates: Vec<String>,
    pub index: usize,
    /// Byte offset in the prompt where the completed token starts.
    pub token_start: usize,
}

/// One node of a tab's split tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Leaf(PaneId),
    /// `dir` is the ratatui layout direction of the children:
    /// `Horizontal` = panes side by side (vsplit), `Vertical` = stacked.
    Split {
        dir: Direction,
        children: Vec<Node>,
    },
}

impl Node {
    /// Replace the leaf for `target` with a split of `[target, new]`, or
    /// extend the parent split when it already runs along `dir`.
    /// Returns true when the target was found.
    fn split(&mut self, target: PaneId, dir: Direction, new: PaneId) -> bool {
        match self {
            Node::Leaf(id) if *id == target => {
                *self = Node::Split {
                    dir,
                    children: vec![Node::Leaf(target), Node::Leaf(new)],
                };
                true
            }
            Node::Leaf(_) => false,
            Node::Split {
                dir: split_dir,
                children,
            } => {
                if *split_dir == dir
                    && let Some(pos) = children
                        .iter()
                        .position(|child| matches!(child, Node::Leaf(id) if *id == target))
                {
                    children.insert(pos + 1, Node::Leaf(new));
                    return true;
                }
                children
                    .iter_mut()
                    .any(|child| child.split(target, dir, new))
            }
        }
    }

    /// Remove the leaf for `target`, collapsing single-child splits.
    /// Returns true when the tree itself became empty.
    fn remove(&mut self, target: PaneId) -> bool {
        match self {
            Node::Leaf(id) => *id == target,
            Node::Split { children, .. } => {
                children.retain_mut(|child| !child.remove(target));
                if children.len() == 1 {
                    *self = children.pop().expect("len checked");
                    false
                } else {
                    children.is_empty()
                }
            }
        }
    }

    fn first_leaf(&self) -> Option<PaneId> {
        match self {
            Node::Leaf(id) => Some(*id),
            Node::Split { children, .. } => children.iter().find_map(Node::first_leaf),
        }
    }

    /// Assign rects to leaves by recursive even splitting.
    ///
    /// Side-by-side (Horizontal) splits get a one-column gutter between
    /// children, pushed to `gutters`, so the renderer can draw a vim-style
    /// vsplit bar. Stacked splits need no gap — each pane's statusline
    /// already separates rows.
    pub fn layout(&self, area: Rect, out: &mut Vec<(PaneId, Rect)>, gutters: &mut Vec<Rect>) {
        match self {
            Node::Leaf(id) => out.push((*id, area)),
            Node::Split { dir, children } => {
                use ratatui::layout::Constraint;
                let with_gutters = *dir == Direction::Horizontal;
                let mut constraints: Vec<Constraint> = Vec::new();
                for idx in 0..children.len() {
                    if with_gutters && idx > 0 {
                        constraints.push(Constraint::Length(1));
                    }
                    constraints.push(Constraint::Ratio(1, children.len() as u32));
                }
                let areas = ratatui::layout::Layout::default()
                    .direction(*dir)
                    .constraints(constraints)
                    .split(area);
                let mut child_iter = children.iter();
                for (idx, child_area) in areas.iter().enumerate() {
                    if with_gutters && idx % 2 == 1 {
                        gutters.push(*child_area);
                    } else if let Some(child) = child_iter.next() {
                        child.layout(*child_area, out, gutters);
                    }
                }
            }
        }
    }
}

pub struct Tab {
    pub name: String,
    pub tree: Node,
    pub panes: Vec<Pane>,
    pub focused: PaneId,
}

impl Tab {
    fn new(name: String, pane: Pane) -> Self {
        Self {
            name,
            tree: Node::Leaf(pane.id),
            focused: pane.id,
            panes: vec![pane],
        }
    }

    pub fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes.iter().find(|pane| pane.id == id)
    }

    pub fn pane_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.panes.iter_mut().find(|pane| pane.id == id)
    }

    pub fn focused_pane(&self) -> &Pane {
        self.pane(self.focused).expect("focused pane exists")
    }

    pub fn focused_pane_mut(&mut self) -> &mut Pane {
        let focused = self.focused;
        self.pane_mut(focused).expect("focused pane exists")
    }

    /// Move focus to the geometrically nearest pane in `(dx, dy)` direction,
    /// based on last-rendered rects. No-op before the first render.
    pub fn focus_direction(&mut self, dx: i32, dy: i32) {
        let current = self.focused_pane().rect;
        let (cx, cy) = (
            current.x as i32 + current.width as i32 / 2,
            current.y as i32 + current.height as i32 / 2,
        );
        let mut best: Option<(i32, PaneId)> = None;
        for pane in &self.panes {
            if pane.id == self.focused {
                continue;
            }
            let rect = pane.rect;
            let (px, py) = (
                rect.x as i32 + rect.width as i32 / 2,
                rect.y as i32 + rect.height as i32 / 2,
            );
            let (vx, vy) = (px - cx, py - cy);
            // Candidate must lie in the requested half-plane.
            if (dx != 0 && vx.signum() != dx) || (dy != 0 && vy.signum() != dy) {
                continue;
            }
            // Weight off-axis distance heavily so focus moves intuitively.
            let dist = if dx != 0 {
                vx.abs() + vy.abs() * 4
            } else {
                vy.abs() + vx.abs() * 4
            };
            if best.is_none_or(|(best_dist, _)| dist < best_dist) {
                best = Some((dist, pane.id));
            }
        }
        if let Some((_, id)) = best {
            self.focused = id;
        }
    }
}

pub struct Workspace {
    pub tabs: Vec<Tab>,
    pub active_tab: usize,
    pub mode: Mode,
    pub pending: Pending,
    pub prompt: Prompt,
    /// Transient status message, cleared on the next key press.
    pub notice: Option<String>,
    pub help_open: bool,
    /// Open source picker, if any. Swallows input while present.
    pub picker: Option<SourcePicker>,
    /// Active `:` prompt completion; cleared by any non-Tab edit.
    pub completion: Option<Completion>,
    next_pane_id: u64,
}

impl Workspace {
    /// Build a workspace whose initial pane starts at the configured wrap
    /// default. Splits and new tabs inherit/seed from there.
    pub fn new(line_wrap: bool) -> Self {
        let mut pane = Pane::new(PaneId(1));
        pane.line_wrap = line_wrap;
        Self {
            tabs: vec![Tab::new("main".to_string(), pane)],
            active_tab: 0,
            mode: Mode::Normal,
            pending: Pending::default(),
            prompt: Prompt::default(),
            notice: None,
            help_open: false,
            picker: None,
            completion: None,
            next_pane_id: 2,
        }
    }

    fn alloc_pane_id(&mut self) -> PaneId {
        let id = PaneId(self.next_pane_id);
        self.next_pane_id += 1;
        id
    }

    pub fn tab(&self) -> &Tab {
        &self.tabs[self.active_tab]
    }

    pub fn tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active_tab]
    }

    pub fn focused_pane(&self) -> &Pane {
        self.tab().focused_pane()
    }

    pub fn focused_pane_mut(&mut self) -> &mut Pane {
        self.tab_mut().focused_pane_mut()
    }

    /// Find a pane by id across all tabs (search results route here).
    pub fn pane_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.tabs.iter_mut().find_map(|tab| tab.pane_mut(id))
    }

    pub fn pane_ids(&self) -> Vec<PaneId> {
        self.tabs
            .iter()
            .flat_map(|tab| tab.panes.iter().map(|pane| pane.id))
            .collect()
    }

    /// Split the focused pane, cloning its viewpoint. Returns the new id so
    /// the caller can dispatch the clone's search.
    pub fn split(&mut self, dir: Direction) -> PaneId {
        let new_id = self.alloc_pane_id();
        let tab = self.tab_mut();
        let new_pane = tab.focused_pane().clone_into(new_id);
        let focused = tab.focused;
        tab.tree.split(focused, dir, new_id);
        tab.panes.push(new_pane);
        tab.focused = new_id;
        new_id
    }

    /// Close the focused pane. Returns the closed pane ids (so the caller
    /// can cancel their search engines). Empty tabs are removed; the result
    /// also says whether the whole workspace is now empty (= quit).
    pub fn close_focused_pane(&mut self) -> (Vec<PaneId>, bool) {
        let tab = self.tab_mut();
        let target = tab.focused;
        // `remove` returning true means the whole tree emptied (the root
        // was the target leaf); the stale root must not be consulted.
        let tree_empty = tab.tree.remove(target);
        tab.panes.retain(|pane| pane.id != target);
        if !tree_empty && let Some(next) = tab.tree.first_leaf() {
            tab.focused = next;
            return (vec![target], false);
        }
        self.tabs.remove(self.active_tab);
        self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
        (vec![target], self.tabs.is_empty())
    }

    /// Close every pane in the active tab except the focused one.
    pub fn only_focused_pane(&mut self) -> Vec<PaneId> {
        let tab = self.tab_mut();
        let keep = tab.focused;
        let closed: Vec<PaneId> = tab
            .panes
            .iter()
            .map(|pane| pane.id)
            .filter(|id| *id != keep)
            .collect();
        tab.panes.retain(|pane| pane.id == keep);
        tab.tree = Node::Leaf(keep);
        closed
    }

    /// Create a new tab with a fresh tailing pane and focus it.
    /// Returns the new pane id for dispatch.
    pub fn new_tab(&mut self, name: Option<String>, line_wrap: bool) -> PaneId {
        let id = self.alloc_pane_id();
        let name = name.unwrap_or_else(|| format!("tab {}", self.tabs.len() + 1));
        let mut pane = Pane::new(id);
        pane.line_wrap = line_wrap;
        self.tabs.push(Tab::new(name, pane));
        self.active_tab = self.tabs.len() - 1;
        id
    }

    /// Close the active tab. Returns closed pane ids and whether the
    /// workspace is now empty.
    pub fn close_tab(&mut self) -> (Vec<PaneId>, bool) {
        let tab = self.tabs.remove(self.active_tab);
        let closed = tab.panes.iter().map(|pane| pane.id).collect();
        self.active_tab = self.active_tab.min(self.tabs.len().saturating_sub(1));
        (closed, self.tabs.is_empty())
    }

    pub fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active_tab = (self.active_tab + 1) % self.tabs.len();
        }
    }

    pub fn prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active_tab = (self.active_tab + self.tabs.len() - 1) % self.tabs.len();
        }
    }
}

impl Default for Workspace {
    fn default() -> Self {
        Self::new(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_same_axis_extends_instead_of_nesting() {
        let mut ws = Workspace::new(false);
        ws.split(Direction::Horizontal);
        ws.split(Direction::Horizontal);

        match &ws.tab().tree {
            Node::Split { dir, children } => {
                assert_eq!(*dir, Direction::Horizontal);
                assert_eq!(children.len(), 3);
                assert!(children.iter().all(|c| matches!(c, Node::Leaf(_))));
            }
            node => panic!("expected flat split, got {node:?}"),
        }
    }

    #[test]
    fn split_other_axis_nests_under_focused_leaf() {
        let mut ws = Workspace::new(false);
        ws.split(Direction::Horizontal);
        ws.split(Direction::Vertical);

        match &ws.tab().tree {
            Node::Split { dir, children } => {
                assert_eq!(*dir, Direction::Horizontal);
                assert_eq!(children.len(), 2);
                assert!(matches!(children[0], Node::Leaf(PaneId(1))));
                match &children[1] {
                    Node::Split { dir, children } => {
                        assert_eq!(*dir, Direction::Vertical);
                        assert_eq!(children.len(), 2);
                    }
                    node => panic!("expected nested vertical split, got {node:?}"),
                }
            }
            node => panic!("expected split root, got {node:?}"),
        }
    }

    #[test]
    fn split_clones_focused_pane_filter() {
        let mut ws = Workspace::new(false);
        ws.focused_pane_mut().filter = vec!["api".to_string()];

        let new_id = ws.split(Direction::Horizontal);

        assert_eq!(ws.tab().focused, new_id);
        assert_eq!(ws.focused_pane().filter, vec!["api".to_string()]);
        assert_eq!(ws.tab().panes.len(), 2);
    }

    #[test]
    fn startup_default_seeds_initial_pane_wrap() {
        let ws = Workspace::new(true);
        assert!(ws.focused_pane().line_wrap);
    }

    #[test]
    fn split_inherits_wrap_state_of_source_pane() {
        let mut ws = Workspace::new(false);
        ws.focused_pane_mut().line_wrap = true;

        ws.split(Direction::Horizontal);

        assert!(ws.focused_pane().line_wrap, "split inherits the wrap flag");
    }

    #[test]
    fn new_tab_seeds_wrap_from_config_default() {
        let mut ws = Workspace::new(false);
        ws.new_tab(Some("wrapped".to_string()), true);
        assert!(ws.focused_pane().line_wrap);
    }

    #[test]
    fn close_collapses_split_and_last_pane_closes_tab() {
        let mut ws = Workspace::new(false);
        ws.split(Direction::Horizontal);

        let (closed, empty) = ws.close_focused_pane();
        assert_eq!(closed, vec![PaneId(2)]);
        assert!(!empty);
        assert!(matches!(ws.tab().tree, Node::Leaf(PaneId(1))));

        let (closed, empty) = ws.close_focused_pane();
        assert_eq!(closed, vec![PaneId(1)]);
        assert!(empty);
    }

    #[test]
    fn only_keeps_focused_pane() {
        let mut ws = Workspace::new(false);
        ws.split(Direction::Horizontal);
        ws.split(Direction::Vertical);
        let focused = ws.tab().focused;

        let closed = ws.only_focused_pane();

        assert_eq!(closed.len(), 2);
        assert_eq!(ws.tab().panes.len(), 1);
        assert_eq!(ws.tab().focused, focused);
        assert!(matches!(ws.tab().tree, Node::Leaf(id) if id == focused));
    }

    #[test]
    fn tabs_cycle_and_close() {
        let mut ws = Workspace::new(false);
        ws.new_tab(Some("errors".to_string()), false);
        assert_eq!(ws.active_tab, 1);
        assert_eq!(ws.tab().name, "errors");

        ws.next_tab();
        assert_eq!(ws.active_tab, 0);
        ws.prev_tab();
        assert_eq!(ws.active_tab, 1);

        let (closed, empty) = ws.close_tab();
        assert_eq!(closed.len(), 1);
        assert!(!empty);
        assert_eq!(ws.active_tab, 0);
    }

    #[test]
    fn directional_focus_uses_rendered_rects() {
        let mut ws = Workspace::new(false);
        let right = ws.split(Direction::Horizontal);
        let left = PaneId(1);
        ws.tab_mut().pane_mut(left).unwrap().rect = Rect::new(0, 0, 40, 20);
        ws.tab_mut().pane_mut(right).unwrap().rect = Rect::new(40, 0, 40, 20);

        ws.tab_mut().focus_direction(-1, 0);
        assert_eq!(ws.tab().focused, left);
        // No pane further left: focus stays put.
        ws.tab_mut().focus_direction(-1, 0);
        assert_eq!(ws.tab().focused, left);
        ws.tab_mut().focus_direction(1, 0);
        assert_eq!(ws.tab().focused, right);
    }

    #[test]
    fn layout_splits_area_evenly_with_vsplit_gutter() {
        let mut ws = Workspace::new(false);
        ws.split(Direction::Horizontal);
        let mut rects = Vec::new();
        let mut gutters = Vec::new();
        ws.tab()
            .tree
            .layout(Rect::new(0, 0, 80, 24), &mut rects, &mut gutters);

        assert_eq!(rects.len(), 2);
        assert_eq!(rects[0].0, PaneId(1));
        assert_eq!(gutters.len(), 1);
        assert_eq!(gutters[0].width, 1);
        assert_eq!(rects[0].1.width + gutters[0].width + rects[1].1.width, 80);
        assert_eq!(rects[0].1.height, 24);
    }

    #[test]
    fn stacked_layout_has_no_gutters() {
        let mut ws = Workspace::new(false);
        ws.split(Direction::Vertical);
        let mut rects = Vec::new();
        let mut gutters = Vec::new();
        ws.tab()
            .tree
            .layout(Rect::new(0, 0, 80, 24), &mut rects, &mut gutters);

        assert_eq!(rects.len(), 2);
        assert!(gutters.is_empty());
        assert_eq!(rects[0].1.height + rects[1].1.height, 24);
    }

    #[test]
    fn prompt_edits_respect_char_boundaries() {
        let mut prompt = Prompt::default();
        prompt.insert('é');
        prompt.insert('x');
        prompt.left();
        prompt.insert('ö');
        assert_eq!(prompt.buf, "éöx");
        prompt.backspace();
        assert_eq!(prompt.buf, "éx");
        prompt.home();
        prompt.right();
        prompt.insert('!');
        assert_eq!(prompt.buf, "é!x");
    }
}
