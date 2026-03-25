//! Tests for the split pane module — layout tree, pane management, and rect splitting.

use crate::tui::pane::{PaneId, PaneManager, SplitDirection, SplitNode};
use ratatui::layout::Rect;

#[test]
fn test_pane_manager_default_single_pane() {
    let mgr = PaneManager::new();
    assert!(!mgr.is_split());
    assert_eq!(mgr.pane_count(), 1);
    assert_eq!(mgr.focused, PaneId::ROOT);
}

#[test]
fn test_split_horizontal_creates_two_panes() {
    let mut mgr = PaneManager::new();
    let new_id = mgr.split(SplitDirection::Horizontal);
    assert!(mgr.is_split());
    assert_eq!(mgr.pane_count(), 2);
    // Focus moves to new pane
    assert_eq!(mgr.focused, new_id);
    assert!(mgr.root.is_some());
}

#[test]
fn test_split_vertical_creates_two_panes() {
    let mut mgr = PaneManager::new();
    let new_id = mgr.split(SplitDirection::Vertical);
    assert!(mgr.is_split());
    assert_eq!(mgr.pane_count(), 2);
    assert_eq!(mgr.focused, new_id);
}

#[test]
fn test_close_focused_returns_to_single() {
    let mut mgr = PaneManager::new();
    mgr.split(SplitDirection::Horizontal);
    assert!(mgr.is_split());
    assert!(mgr.close_focused());
    assert!(!mgr.is_split());
    assert_eq!(mgr.pane_count(), 1);
}

#[test]
fn test_close_single_pane_noop() {
    let mut mgr = PaneManager::new();
    assert!(!mgr.close_focused());
    assert_eq!(mgr.pane_count(), 1);
}

#[test]
fn test_focus_next_cycles() {
    let mut mgr = PaneManager::new();
    mgr.split(SplitDirection::Horizontal);
    let ids = mgr.pane_ids_in_order();
    assert_eq!(ids.len(), 2);

    // Currently focused on the new pane (second)
    let second = mgr.focused;
    mgr.focus_next();
    let after_next = mgr.focused;
    assert_ne!(second, after_next);

    // Cycling back
    mgr.focus_next();
    assert_eq!(mgr.focused, second);
}

#[test]
fn test_focus_prev_cycles() {
    let mut mgr = PaneManager::new();
    mgr.split(SplitDirection::Horizontal);
    let first = mgr.focused;
    mgr.focus_prev();
    assert_ne!(mgr.focused, first);
    mgr.focus_prev();
    assert_eq!(mgr.focused, first);
}

#[test]
fn test_focus_single_pane_noop() {
    let mut mgr = PaneManager::new();
    let before = mgr.focused;
    mgr.focus_next();
    assert_eq!(mgr.focused, before);
    mgr.focus_prev();
    assert_eq!(mgr.focused, before);
}

#[test]
fn test_pane_session_assignment() {
    let mut mgr = PaneManager::new();
    let session = uuid::Uuid::new_v4();
    if let Some(pane) = mgr.focused_pane_mut() {
        pane.session_id = Some(session);
    }
    assert_eq!(mgr.focused_pane().unwrap().session_id, Some(session));
}

#[test]
fn test_nested_split() {
    let mut mgr = PaneManager::new();
    // First split: horizontal
    mgr.split(SplitDirection::Horizontal);
    assert_eq!(mgr.pane_count(), 2);
    // Second split: vertical on the new pane
    mgr.split(SplitDirection::Vertical);
    assert_eq!(mgr.pane_count(), 3);

    let ids = mgr.pane_ids_in_order();
    assert_eq!(ids.len(), 3);
}

#[test]
fn test_close_nested_preserves_siblings() {
    let mut mgr = PaneManager::new();
    mgr.split(SplitDirection::Horizontal);
    mgr.split(SplitDirection::Vertical);
    assert_eq!(mgr.pane_count(), 3);

    // Close the focused (most recently created) pane
    mgr.close_focused();
    assert_eq!(mgr.pane_count(), 2);
    assert!(mgr.is_split());

    // Close again
    mgr.close_focused();
    assert_eq!(mgr.pane_count(), 1);
    assert!(!mgr.is_split());
}

#[test]
fn test_split_node_layout_horizontal() {
    let node = SplitNode::Split {
        direction: SplitDirection::Horizontal,
        ratio: 0.5,
        first: Box::new(SplitNode::Leaf(PaneId(0))),
        second: Box::new(SplitNode::Leaf(PaneId(1))),
    };

    let area = Rect::new(0, 0, 100, 50);
    let rects = node.layout(area);
    assert_eq!(rects.len(), 2);

    let (id0, r0) = &rects[0];
    let (id1, r1) = &rects[1];
    assert_eq!(*id0, PaneId(0));
    assert_eq!(*id1, PaneId(1));

    // Left half
    assert_eq!(r0.x, 0);
    assert_eq!(r0.width, 50);
    // Right half
    assert_eq!(r1.x, 50);
    assert_eq!(r1.width, 50);
    // Same height
    assert_eq!(r0.height, 50);
    assert_eq!(r1.height, 50);
}

#[test]
fn test_split_node_layout_vertical() {
    let node = SplitNode::Split {
        direction: SplitDirection::Vertical,
        ratio: 0.5,
        first: Box::new(SplitNode::Leaf(PaneId(0))),
        second: Box::new(SplitNode::Leaf(PaneId(1))),
    };

    let area = Rect::new(0, 0, 100, 50);
    let rects = node.layout(area);
    assert_eq!(rects.len(), 2);

    let (_, r0) = &rects[0];
    let (_, r1) = &rects[1];

    // Top half
    assert_eq!(r0.y, 0);
    assert_eq!(r0.height, 25);
    // Bottom half
    assert_eq!(r1.y, 25);
    assert_eq!(r1.height, 25);
    // Same width
    assert_eq!(r0.width, 100);
    assert_eq!(r1.width, 100);
}

#[test]
fn test_split_node_leaves() {
    let node = SplitNode::Split {
        direction: SplitDirection::Horizontal,
        ratio: 0.5,
        first: Box::new(SplitNode::Leaf(PaneId(0))),
        second: Box::new(SplitNode::Split {
            direction: SplitDirection::Vertical,
            ratio: 0.5,
            first: Box::new(SplitNode::Leaf(PaneId(1))),
            second: Box::new(SplitNode::Leaf(PaneId(2))),
        }),
    };

    let leaves = node.leaves();
    assert_eq!(leaves, vec![PaneId(0), PaneId(1), PaneId(2)]);
}

#[test]
fn test_split_node_first_leaf() {
    let node = SplitNode::Split {
        direction: SplitDirection::Horizontal,
        ratio: 0.5,
        first: Box::new(SplitNode::Split {
            direction: SplitDirection::Vertical,
            ratio: 0.5,
            first: Box::new(SplitNode::Leaf(PaneId(3))),
            second: Box::new(SplitNode::Leaf(PaneId(4))),
        }),
        second: Box::new(SplitNode::Leaf(PaneId(5))),
    };

    assert_eq!(node.first_leaf(), PaneId(3));
}

#[test]
fn test_split_node_replace_leaf() {
    let node = SplitNode::Leaf(PaneId(0));
    let replaced = node.replace_leaf(PaneId(0), SplitDirection::Horizontal, PaneId(1));

    match replaced {
        SplitNode::Split {
            direction,
            first,
            second,
            ..
        } => {
            assert_eq!(direction, SplitDirection::Horizontal);
            assert!(matches!(*first, SplitNode::Leaf(PaneId(0))));
            assert!(matches!(*second, SplitNode::Leaf(PaneId(1))));
        }
        _ => panic!("Expected Split node"),
    }
}

#[test]
fn test_split_node_remove_leaf() {
    let node = SplitNode::Split {
        direction: SplitDirection::Horizontal,
        ratio: 0.5,
        first: Box::new(SplitNode::Leaf(PaneId(0))),
        second: Box::new(SplitNode::Leaf(PaneId(1))),
    };

    let simplified = node.remove_leaf(PaneId(1));
    assert!(matches!(simplified, SplitNode::Leaf(PaneId(0))));
}

#[test]
fn test_pane_ids_in_order_single() {
    let mgr = PaneManager::new();
    let ids = mgr.pane_ids_in_order();
    assert_eq!(ids, vec![PaneId::ROOT]);
}

#[test]
fn test_pane_ids_in_order_split() {
    let mut mgr = PaneManager::new();
    mgr.split(SplitDirection::Horizontal);
    let ids = mgr.pane_ids_in_order();
    assert_eq!(ids.len(), 2);
    assert_eq!(ids[0], PaneId::ROOT);
}

#[test]
fn test_get_pane_by_id() {
    let mut mgr = PaneManager::new();
    let new_id = mgr.split(SplitDirection::Horizontal);
    assert!(mgr.get(PaneId::ROOT).is_some());
    assert!(mgr.get(new_id).is_some());
    assert!(mgr.get(PaneId(999)).is_none());
}

#[test]
fn test_pane_auto_scroll_default() {
    let mgr = PaneManager::new();
    let pane = mgr.focused_pane().unwrap();
    assert!(pane.auto_scroll);
    assert_eq!(pane.scroll_offset, 0);
}
