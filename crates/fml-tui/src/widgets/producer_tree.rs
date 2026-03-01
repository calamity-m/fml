//! Producer tree widget — collapsible tree of feed sources in the left pane.
//!
//! # Navigation
//! - `↑`/`k` and `↓`/`j` move the cursor up and down the visible list.
//! - `→`/`l` or `Enter` expands the focused node; `←`/`h` collapses it.
//! - `Space` toggles the selection state of the focused node.

use crate::event::{AppEvent, Direction};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, List, ListItem, ListState, StatefulWidget, Widget},
};
use tracing;

// ---------------------------------------------------------------------------
// Selection state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeSelection {
    /// All producers under this node are tailed.
    Selected,
    /// No producers under this node are tailed.
    Unselected,
    /// Some (but not all) producers under this node are tailed.
    Partial,
}

// ---------------------------------------------------------------------------
// Tree node
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct TreeNode {
    /// Stable identifier (used for mutations).
    pub id: String,
    /// Human-readable display label.
    pub label: String,
    pub expanded: bool,
    pub selection: NodeSelection,
    pub children: Vec<TreeNode>,
}

impl TreeNode {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            expanded: true,
            selection: NodeSelection::Unselected,
            children: Vec::new(),
        }
    }

    pub fn with_children(mut self, children: Vec<TreeNode>) -> Self {
        self.children = children;
        self
    }
}

// ---------------------------------------------------------------------------
// Tree state
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct ProducerTreeState {
    pub nodes: Vec<TreeNode>,
    /// Index into the currently-visible (flattened) list.
    pub cursor: usize,
}

impl ProducerTreeState {
    /// Return the id of the node at the cursor, if any.
    fn cursor_id(&self) -> Option<String> {
        self.visible()
            .into_iter()
            .nth(self.cursor)
            .map(|(_, n)| n.id.clone())
    }

    /// Flatten the tree into `(depth, &node)` pairs, respecting expanded state.
    pub fn visible(&self) -> Vec<(usize, &TreeNode)> {
        flatten(&self.nodes, 0)
    }

    /// Handle an [`AppEvent`], mutating state as appropriate.
    pub fn handle(&mut self, event: &AppEvent) {
        match event {
            AppEvent::TreeNav(Direction::Up) => {
                self.cursor = self.cursor.saturating_sub(1);
                tracing::debug!(cursor = self.cursor, "tree: cursor up");
            }
            AppEvent::TreeNav(Direction::Down) => {
                let max = self.visible().len().saturating_sub(1);
                if self.cursor < max {
                    self.cursor += 1;
                }
                tracing::debug!(cursor = self.cursor, "tree: cursor down");
            }
            AppEvent::TreeNav(Direction::Right) => {
                if let Some(id) = self.cursor_id() {
                    tracing::debug!(node = %id, "tree: expand");
                    set_expanded(&mut self.nodes, &id, true);
                }
            }
            AppEvent::Enter => {
                if let Some(id) = self.cursor_id() {
                    if is_leaf(&self.nodes, &id) {
                        tracing::debug!(node = %id, "tree: toggle selection (leaf enter)");
                        toggle_selection(&mut self.nodes, &id);
                    } else {
                        tracing::debug!(node = %id, "tree: toggle expand (parent enter)");
                        toggle_expanded(&mut self.nodes, &id);
                        self.clamp_cursor();
                    }
                }
            }
            AppEvent::TreeNav(Direction::Left) => {
                if let Some(id) = self.cursor_id() {
                    tracing::debug!(node = %id, "tree: collapse");
                    set_expanded(&mut self.nodes, &id, false);
                    self.clamp_cursor();
                }
            }
            AppEvent::Char(' ') => {
                if let Some(id) = self.cursor_id() {
                    tracing::debug!(node = %id, "tree: toggle selection (space)");
                    toggle_selection(&mut self.nodes, &id);
                }
            }
            _ => {}
        }
    }

    fn clamp_cursor(&mut self) {
        let max = self.visible().len().saturating_sub(1);
        if self.cursor > max {
            self.cursor = max;
        }
    }
}

// ---------------------------------------------------------------------------
// Recursive tree helpers
// ---------------------------------------------------------------------------

fn flatten(nodes: &[TreeNode], depth: usize) -> Vec<(usize, &TreeNode)> {
    let mut out = Vec::new();
    for node in nodes {
        out.push((depth, node));
        if node.expanded {
            out.extend(flatten(&node.children, depth + 1));
        }
    }
    out
}

/// Set the `expanded` flag on the node with `id`. Returns `true` if found.
#[allow(clippy::ptr_arg)] // Vec retained for future dynamic-size tree operations
fn set_expanded(nodes: &mut Vec<TreeNode>, id: &str, expanded: bool) -> bool {
    for node in nodes.iter_mut() {
        if node.id == id {
            node.expanded = expanded;
            return true;
        }
        if set_expanded(&mut node.children, id, expanded) {
            return true;
        }
    }
    false
}

/// Flip the `expanded` flag on the node with `id`. Returns `true` if found.
#[allow(clippy::ptr_arg)]
fn toggle_expanded(nodes: &mut Vec<TreeNode>, id: &str) -> bool {
    for node in nodes.iter_mut() {
        if node.id == id {
            node.expanded = !node.expanded;
            return true;
        }
        if toggle_expanded(&mut node.children, id) {
            return true;
        }
    }
    false
}

/// Returns `Some(true)` if the node with `id` is a leaf, `Some(false)` if it
/// has children, or `None` if the id is not found in the subtree.
fn find_is_leaf(nodes: &[TreeNode], id: &str) -> Option<bool> {
    for node in nodes {
        if node.id == id {
            return Some(node.children.is_empty());
        }
        if let Some(result) = find_is_leaf(&node.children, id) {
            return Some(result);
        }
    }
    None
}

fn is_leaf(nodes: &[TreeNode], id: &str) -> bool {
    find_is_leaf(nodes, id).unwrap_or(true)
}

/// Toggle the selection state of the node with `id`.
///
/// When the toggled node is found, its new state is pushed down to every
/// descendant via [`set_all_selection`]. On the way back up the call stack,
/// each ancestor recomputes its own state from its children via
/// [`compute_selection_from_children`].
#[allow(clippy::ptr_arg)]
fn toggle_selection(nodes: &mut Vec<TreeNode>, id: &str) -> bool {
    for node in nodes.iter_mut() {
        if node.id == id {
            let new_state = match node.selection {
                NodeSelection::Selected | NodeSelection::Partial => NodeSelection::Unselected,
                NodeSelection::Unselected => NodeSelection::Selected,
            };
            node.selection = new_state;
            // Push the new state down to every descendant
            set_all_selection(&mut node.children, new_state);
            return true;
        }
        if toggle_selection(&mut node.children, id) {
            // Recompute this node's state from its (now-updated) children
            node.selection = compute_selection_from_children(&node.children);
            return true;
        }
    }
    false
}

/// Recursively set every node in the subtree to `state`.
#[allow(clippy::ptr_arg)]
fn set_all_selection(nodes: &mut Vec<TreeNode>, state: NodeSelection) {
    for node in nodes.iter_mut() {
        node.selection = state;
        set_all_selection(&mut node.children, state);
    }
}

/// Derive a parent's selection state from the states of its direct children.
///
/// - All `Selected`   → `Selected`
/// - All `Unselected` → `Unselected`
/// - Any mix (or any child is `Partial`) → `Partial`
fn compute_selection_from_children(children: &[TreeNode]) -> NodeSelection {
    if children.is_empty() {
        return NodeSelection::Unselected;
    }
    let all_sel = children
        .iter()
        .all(|c| c.selection == NodeSelection::Selected);
    let all_unsel = children
        .iter()
        .all(|c| c.selection == NodeSelection::Unselected);
    if all_sel {
        NodeSelection::Selected
    } else if all_unsel {
        NodeSelection::Unselected
    } else {
        NodeSelection::Partial
    }
}

// ---------------------------------------------------------------------------
// Widget
// ---------------------------------------------------------------------------

pub struct ProducerTree<'a> {
    state: &'a ProducerTreeState,
    focused: bool,
    theme: &'a crate::theme::Theme,
}

impl<'a> ProducerTree<'a> {
    pub fn new(
        state: &'a ProducerTreeState,
        focused: bool,
        theme: &'a crate::theme::Theme,
    ) -> Self {
        Self {
            state,
            focused,
            theme,
        }
    }
}

impl Widget for ProducerTree<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            self.theme.border_focused
        } else {
            self.theme.border_unfocused
        };

        let block = Block::bordered()
            .title("Producers")
            .border_style(border_style);

        let inner = block.inner(area);
        block.render(area, buf);

        let visible = self.state.visible();

        let items: Vec<ListItem> = visible
            .iter()
            .map(|(depth, node)| {
                let indent = "  ".repeat(*depth);
                let expand = if node.children.is_empty() {
                    "  "
                } else if node.expanded {
                    "▼ "
                } else {
                    "▶ "
                };
                let sel = match node.selection {
                    NodeSelection::Selected => " ✓",
                    NodeSelection::Unselected => " ○",
                    NodeSelection::Partial => " ◐",
                };
                ListItem::new(Line::from(format!(
                    "{}{}{}{}",
                    indent, expand, node.label, sel
                )))
            })
            .collect();

        let list =
            List::new(items).highlight_style(Style::default().add_modifier(Modifier::REVERSED));

        let mut list_state = ListState::default().with_selected(Some(self.state.cursor));
        StatefulWidget::render(list, inner, buf, &mut list_state);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a small tree: root → [a (leaf), b (leaf), c (leaf)]
    fn three_leaf_tree() -> Vec<TreeNode> {
        vec![TreeNode::new("root", "root").with_children(vec![
            TreeNode::new("a", "a"),
            TreeNode::new("b", "b"),
            TreeNode::new("c", "c"),
        ])]
    }

    fn find_sel(nodes: &[TreeNode], id: &str) -> NodeSelection {
        for n in nodes {
            if n.id == id {
                return n.selection;
            }
            if let Some(result) = find_sel_opt(&n.children, id) {
                return result;
            }
        }
        NodeSelection::Unselected
    }

    fn find_sel_opt(nodes: &[TreeNode], id: &str) -> Option<NodeSelection> {
        for n in nodes {
            if n.id == id {
                return Some(n.selection);
            }
            if let Some(r) = find_sel_opt(&n.children, id) {
                return Some(r);
            }
        }
        None
    }

    #[test]
    fn toggling_leaf_selects_it() {
        let mut nodes = three_leaf_tree();
        toggle_selection(&mut nodes, "a");
        assert_eq!(find_sel(&nodes, "a"), NodeSelection::Selected);
        assert_eq!(find_sel(&nodes, "b"), NodeSelection::Unselected);
    }

    #[test]
    fn toggling_leaf_makes_parent_partial() {
        let mut nodes = three_leaf_tree();
        toggle_selection(&mut nodes, "a");
        assert_eq!(find_sel(&nodes, "root"), NodeSelection::Partial);
    }

    #[test]
    fn toggling_all_leaves_makes_parent_selected() {
        let mut nodes = three_leaf_tree();
        toggle_selection(&mut nodes, "a");
        toggle_selection(&mut nodes, "b");
        toggle_selection(&mut nodes, "c");
        assert_eq!(find_sel(&nodes, "root"), NodeSelection::Selected);
    }

    #[test]
    fn toggling_parent_selects_all_children() {
        let mut nodes = three_leaf_tree();
        toggle_selection(&mut nodes, "root");
        assert_eq!(find_sel(&nodes, "root"), NodeSelection::Selected);
        assert_eq!(find_sel(&nodes, "a"), NodeSelection::Selected);
        assert_eq!(find_sel(&nodes, "b"), NodeSelection::Selected);
        assert_eq!(find_sel(&nodes, "c"), NodeSelection::Selected);
    }

    #[test]
    fn toggling_selected_parent_deselects_all_children() {
        let mut nodes = three_leaf_tree();
        toggle_selection(&mut nodes, "root"); // → Selected
        toggle_selection(&mut nodes, "root"); // → Unselected
        assert_eq!(find_sel(&nodes, "root"), NodeSelection::Unselected);
        assert_eq!(find_sel(&nodes, "a"), NodeSelection::Unselected);
        assert_eq!(find_sel(&nodes, "b"), NodeSelection::Unselected);
    }

    #[test]
    fn partial_parent_toggle_clears_all() {
        let mut nodes = three_leaf_tree();
        toggle_selection(&mut nodes, "a"); // root → Partial
        toggle_selection(&mut nodes, "root"); // Partial → Unselected, children cleared
        assert_eq!(find_sel(&nodes, "root"), NodeSelection::Unselected);
        assert_eq!(find_sel(&nodes, "a"), NodeSelection::Unselected);
    }

    #[test]
    fn deselecting_last_child_makes_parent_unselected() {
        let mut nodes = three_leaf_tree();
        toggle_selection(&mut nodes, "a");
        toggle_selection(&mut nodes, "a"); // back to unselected
        assert_eq!(find_sel(&nodes, "root"), NodeSelection::Unselected);
    }
}
