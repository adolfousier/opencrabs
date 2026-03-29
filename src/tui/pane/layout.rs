//! Split layout tree — recursive binary tree describing how panes are arranged.

use super::state::PaneId;
use ratatui::layout::Rect;
use serde::{Deserialize, Serialize};

/// Direction of a split.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SplitDirection {
    /// Left | Right
    Horizontal,
    /// Top / Bottom
    Vertical,
}

/// Recursive binary tree node for the pane layout.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SplitNode {
    /// A terminal node — contains a single pane.
    Leaf(PaneId),
    /// An internal split — divides space between two children.
    Split {
        direction: SplitDirection,
        /// Fraction of space given to `first` (0.0–1.0).
        ratio: f32,
        first: Box<SplitNode>,
        second: Box<SplitNode>,
    },
}

impl SplitNode {
    /// Compute the screen rectangles for every leaf pane in the tree.
    pub fn layout(&self, area: Rect) -> Vec<(PaneId, Rect)> {
        let mut result = Vec::new();
        self.layout_inner(area, &mut result);
        result
    }

    fn layout_inner(&self, area: Rect, out: &mut Vec<(PaneId, Rect)>) {
        match self {
            SplitNode::Leaf(id) => {
                out.push((*id, area));
            }
            SplitNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                let (a, b) = split_rect(area, *direction, *ratio);
                first.layout_inner(a, out);
                second.layout_inner(b, out);
            }
        }
    }

    /// Collect all leaf PaneIds in left-to-right / top-to-bottom order.
    pub fn leaves(&self) -> Vec<PaneId> {
        let mut out = Vec::new();
        self.collect_leaves(&mut out);
        out
    }

    fn collect_leaves(&self, out: &mut Vec<PaneId>) {
        match self {
            SplitNode::Leaf(id) => out.push(*id),
            SplitNode::Split { first, second, .. } => {
                first.collect_leaves(out);
                second.collect_leaves(out);
            }
        }
    }

    /// Return the first leaf in the tree.
    pub fn first_leaf(&self) -> PaneId {
        match self {
            SplitNode::Leaf(id) => *id,
            SplitNode::Split { first, .. } => first.first_leaf(),
        }
    }

    /// Replace a leaf with a new split containing the original leaf and a new pane.
    pub fn replace_leaf(self, target: PaneId, direction: SplitDirection, new: PaneId) -> Self {
        match self {
            SplitNode::Leaf(id) if id == target => SplitNode::Split {
                direction,
                ratio: 0.5,
                first: Box::new(SplitNode::Leaf(id)),
                second: Box::new(SplitNode::Leaf(new)),
            },
            SplitNode::Leaf(id) => SplitNode::Leaf(id),
            SplitNode::Split {
                direction: d,
                ratio,
                first,
                second,
            } => SplitNode::Split {
                direction: d,
                ratio,
                first: Box::new(first.replace_leaf(target, direction, new)),
                second: Box::new(second.replace_leaf(target, direction, new)),
            },
        }
    }

    /// Remove a leaf from the tree. Returns the simplified tree.
    /// If the removed leaf's sibling is the only remaining child, it replaces the parent.
    pub fn remove_leaf(self, target: PaneId) -> Self {
        match self {
            SplitNode::Leaf(id) => SplitNode::Leaf(id), // can't remove self
            SplitNode::Split {
                direction,
                ratio,
                first,
                second,
            } => {
                // Check if one child is the target leaf.
                if matches!(&*first, SplitNode::Leaf(id) if *id == target) {
                    return *second;
                }
                if matches!(&*second, SplitNode::Leaf(id) if *id == target) {
                    return *first;
                }
                // Recurse.
                SplitNode::Split {
                    direction,
                    ratio,
                    first: Box::new(first.remove_leaf(target)),
                    second: Box::new(second.remove_leaf(target)),
                }
            }
        }
    }
}

/// Split a rectangle in a given direction at the specified ratio.
fn split_rect(area: Rect, direction: SplitDirection, ratio: f32) -> (Rect, Rect) {
    match direction {
        SplitDirection::Horizontal => {
            let left_width = ((area.width as f32) * ratio).round() as u16;
            let right_width = area.width.saturating_sub(left_width);
            let left = Rect {
                width: left_width,
                ..area
            };
            let right = Rect {
                x: area.x + left_width,
                width: right_width,
                ..area
            };
            (left, right)
        }
        SplitDirection::Vertical => {
            let top_height = ((area.height as f32) * ratio).round() as u16;
            let bottom_height = area.height.saturating_sub(top_height);
            let top = Rect {
                height: top_height,
                ..area
            };
            let bottom = Rect {
                y: area.y + top_height,
                height: bottom_height,
                ..area
            };
            (top, bottom)
        }
    }
}
