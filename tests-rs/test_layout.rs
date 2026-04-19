use super::*;

// ════════════════════════════════════════════════════════════════════════════
//  parse_layout_string: standalone LayoutNode parsing
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn parse_single_pane() {
    let node = parse_layout_string("34b0,120x30,0,0,0").unwrap();
    match node {
        LayoutNode::Leaf { width, height, x, y, pane_id } => {
            assert_eq!(width, 120);
            assert_eq!(height, 30);
            assert_eq!(x, 0);
            assert_eq!(y, 0);
            assert_eq!(pane_id, Some(0));
        }
        _ => panic!("expected Leaf, got Split"),
    }
}

#[test]
fn parse_two_panes_horizontal() {
    let node = parse_layout_string("5e08,120x30,0,0{60x30,0,0,0,59x30,61,0,1}").unwrap();
    match &node {
        LayoutNode::Split { kind, width, height, children, .. } => {
            assert_eq!(*kind, LayoutKind::Horizontal);
            assert_eq!(*width, 120);
            assert_eq!(*height, 30);
            assert_eq!(children.len(), 2);
            match &children[0] {
                LayoutNode::Leaf { width, height, pane_id, .. } => {
                    assert_eq!(*width, 60);
                    assert_eq!(*height, 30);
                    assert_eq!(*pane_id, Some(0));
                }
                _ => panic!("expected first child to be Leaf"),
            }
            match &children[1] {
                LayoutNode::Leaf { width, height, x, pane_id, .. } => {
                    assert_eq!(*width, 59);
                    assert_eq!(*height, 30);
                    assert_eq!(*x, 61);
                    assert_eq!(*pane_id, Some(1));
                }
                _ => panic!("expected second child to be Leaf"),
            }
        }
        _ => panic!("expected Split, got Leaf"),
    }
}

#[test]
fn parse_two_panes_vertical() {
    let node = parse_layout_string("5e08,120x30,0,0[120x15,0,0,0,120x14,0,16,1]").unwrap();
    match &node {
        LayoutNode::Split { kind, children, .. } => {
            assert_eq!(*kind, LayoutKind::Vertical);
            assert_eq!(children.len(), 2);
            match &children[0] {
                LayoutNode::Leaf { width, height, y, pane_id, .. } => {
                    assert_eq!(*width, 120);
                    assert_eq!(*height, 15);
                    assert_eq!(*y, 0);
                    assert_eq!(*pane_id, Some(0));
                }
                _ => panic!("expected Leaf"),
            }
            match &children[1] {
                LayoutNode::Leaf { width, height, y, pane_id, .. } => {
                    assert_eq!(*width, 120);
                    assert_eq!(*height, 14);
                    assert_eq!(*y, 16);
                    assert_eq!(*pane_id, Some(1));
                }
                _ => panic!("expected Leaf"),
            }
        }
        _ => panic!("expected Split"),
    }
}

#[test]
fn parse_nested_layout() {
    // H-split: left leaf + right V-split of two leaves
    let node = parse_layout_string(
        "d9e0,120x30,0,0{60x30,0,0,0,59x30,61,0[59x15,61,0,1,59x14,61,16,2]}"
    ).unwrap();
    match &node {
        LayoutNode::Split { kind, children, .. } => {
            assert_eq!(*kind, LayoutKind::Horizontal);
            assert_eq!(children.len(), 2);
            assert!(matches!(&children[0], LayoutNode::Leaf { .. }));
            match &children[1] {
                LayoutNode::Split { kind, children: inner, .. } => {
                    assert_eq!(*kind, LayoutKind::Vertical);
                    assert_eq!(inner.len(), 2);
                    assert!(matches!(&inner[0], LayoutNode::Leaf { pane_id: Some(1), .. }));
                    assert!(matches!(&inner[1], LayoutNode::Leaf { pane_id: Some(2), .. }));
                }
                _ => panic!("expected nested Split"),
            }
        }
        _ => panic!("expected Split"),
    }
}

#[test]
fn count_leaves_single() {
    let node = parse_layout_string("34b0,120x30,0,0,0").unwrap();
    assert_eq!(node.count_leaves(), 1);
}

#[test]
fn count_leaves_two() {
    let node = parse_layout_string("5e08,120x30,0,0{60x30,0,0,0,59x30,61,0,1}").unwrap();
    assert_eq!(node.count_leaves(), 2);
}

#[test]
fn count_leaves_three_nested() {
    let node = parse_layout_string(
        "d9e0,120x30,0,0{60x30,0,0,0,59x30,61,0[59x15,61,0,1,59x14,61,16,2]}"
    ).unwrap();
    assert_eq!(node.count_leaves(), 3);
}

// ════════════════════════════════════════════════════════════════════════════
//  Error handling: invalid inputs return None
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn parse_empty_string_returns_none() {
    assert!(parse_layout_string("").is_none());
}

#[test]
fn parse_too_short_returns_none() {
    assert!(parse_layout_string("abc").is_none());
}

#[test]
fn parse_bad_checksum_returns_none() {
    // 'zzzz' has non-hex chars
    assert!(parse_layout_string("zzzz,120x30,0,0,0").is_none());
}

#[test]
fn parse_no_comma_after_checksum_returns_none() {
    assert!(parse_layout_string("5e08x120x30,0,0,0").is_none());
}

#[test]
fn parse_garbage_returns_none() {
    assert!(parse_layout_string("5e08,not_a_layout").is_none());
}

#[test]
fn parse_unclosed_bracket_returns_none() {
    assert!(parse_layout_string("5e08,120x30,0,0{60x30,0,0,0").is_none());
}

// ════════════════════════════════════════════════════════════════════════════
//  layout_node_to_node: size computation (tested via LayoutNode structure)
//  Since Pane requires real PTY objects, we verify size computation
//  through the LayoutNode dimensions directly.
// ════════════════════════════════════════════════════════════════════════════

/// Helper: compute the proportional sizes that layout_node_to_node would
/// generate, given a split's children dimensions and split kind.
fn compute_sizes(layout: &LayoutNode) -> Option<Vec<u16>> {
    match layout {
        LayoutNode::Leaf { .. } => None,
        LayoutNode::Split { kind, children, .. } => {
            let total_size: u32 = match kind {
                LayoutKind::Horizontal => children.iter().map(|c| c.width() as u32).sum(),
                LayoutKind::Vertical => children.iter().map(|c| c.height() as u32).sum(),
            };
            if total_size == 0 {
                let n = children.len().max(1) as u16;
                return Some(vec![100 / n; children.len()]);
            }
            let mut szs: Vec<u16> = children.iter().map(|c| {
                let dim = match kind {
                    LayoutKind::Horizontal => c.width() as u32,
                    LayoutKind::Vertical => c.height() as u32,
                };
                (dim * 100 / total_size) as u16
            }).collect();
            let sum: u16 = szs.iter().sum();
            if sum < 100 { if let Some(last) = szs.last_mut() { *last += 100 - sum; } }
            Some(szs)
        }
    }
}

#[test]
fn sizes_sum_to_100_equal_horizontal() {
    let layout = parse_layout_string("aaaa,100x50,0,0{50x50,0,0,0,50x50,50,0,1}").unwrap();
    let sizes = compute_sizes(&layout).unwrap();
    assert_eq!(sizes, vec![50, 50]);
}

#[test]
fn sizes_sum_to_100_unequal_horizontal() {
    // 80 + 39 = 119
    let layout = parse_layout_string("aaaa,120x50,0,0{80x50,0,0,0,39x50,81,0,1}").unwrap();
    let sizes = compute_sizes(&layout).unwrap();
    let sum: u16 = sizes.iter().sum();
    assert_eq!(sum, 100);
    // 80/119*100 = 67, 39/119*100 = 32, remainder 1 added to last
    assert_eq!(sizes[0], 67);
    assert_eq!(sizes[1], 33);
}

#[test]
fn sizes_vertical_split_uses_heights() {
    // 20 + 29 = 49
    let layout = parse_layout_string("aaaa,120x50,0,0[120x20,0,0,0,120x29,0,21,1]").unwrap();
    let sizes = compute_sizes(&layout).unwrap();
    let sum: u16 = sizes.iter().sum();
    assert_eq!(sum, 100);
    match &layout {
        LayoutNode::Split { kind, .. } => assert_eq!(*kind, LayoutKind::Vertical),
        _ => panic!("expected Split"),
    }
}

#[test]
fn sizes_three_way_split() {
    // 3 even columns: 40 + 39 + 40 = 119
    let layout = parse_layout_string(
        "aaaa,120x50,0,0{40x50,0,0,0,39x50,41,0,1,40x50,81,0,2}"
    ).unwrap();
    let sizes = compute_sizes(&layout).unwrap();
    assert_eq!(sizes.len(), 3);
    let sum: u16 = sizes.iter().sum();
    assert_eq!(sum, 100);
}

// ════════════════════════════════════════════════════════════════════════════
//  Whitespace tolerance
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn parse_with_leading_trailing_whitespace() {
    let node = parse_layout_string("  34b0,120x30,0,0,0  ");
    assert!(node.is_some());
}

// ════════════════════════════════════════════════════════════════════════════
//  Complex real-world layout strings
// ════════════════════════════════════════════════════════════════════════════

#[test]
fn parse_four_pane_tiled() {
    // 4-pane tiled: V-split of two H-splits
    let layout = "1234,200x50,0,0[200x25,0,0{100x25,0,0,0,99x25,101,0,1},200x24,0,26{100x24,0,26,2,99x24,101,26,3}]";
    let node = parse_layout_string(layout).unwrap();
    assert_eq!(node.count_leaves(), 4);
    match &node {
        LayoutNode::Split { kind, children, .. } => {
            assert_eq!(*kind, LayoutKind::Vertical);
            assert_eq!(children.len(), 2);
            for child in children {
                match child {
                    LayoutNode::Split { kind, children: inner, .. } => {
                        assert_eq!(*kind, LayoutKind::Horizontal);
                        assert_eq!(inner.len(), 2);
                    }
                    _ => panic!("expected inner H-split"),
                }
            }
        }
        _ => panic!("expected outer V-split"),
    }
}

#[test]
fn parse_three_even_vertical() {
    // 3 panes stacked vertically
    let layout = "abcd,120x60,0,0[120x20,0,0,0,120x19,0,21,1,120x19,0,41,2]";
    let node = parse_layout_string(layout).unwrap();
    assert_eq!(node.count_leaves(), 3);
    match &node {
        LayoutNode::Split { kind, children, .. } => {
            assert_eq!(*kind, LayoutKind::Vertical);
            assert_eq!(children.len(), 3);
        }
        _ => panic!("expected V-split"),
    }
}
