use super::*;

/// Extract (detach) a node from the tree at the given path WITHOUT killing it.
/// Returns (remaining_tree, extracted_node).
/// If the path points to the root, returns (None, root).
pub fn extract_node(root: Node, path: &[usize]) -> (Option<Node>, Option<Node>) {
    if path.is_empty() {
        return (None, Some(root));
    }
    match root {
        Node::Leaf(p) => (Some(Node::Leaf(p)), None), // path doesn't exist
        Node::Split { kind, sizes, children } => {
            let idx = path[0];
            if idx >= children.len() {
                return (Some(Node::Split { kind, sizes, children }), None);
            }
            if path.len() == 1 {
                // Extract child at idx
                let mut remaining: Vec<Node> = Vec::new();
                let mut extracted: Option<Node> = None;
                for (i, child) in children.into_iter().enumerate() {
                    if i == idx { extracted = Some(child); }
                    else { remaining.push(child); }
                }
                let tree = if remaining.is_empty() {
                    None
                } else if remaining.len() == 1 {
                    Some(remaining.into_iter().next().unwrap())
                } else {
                    let mut eq = vec![100 / remaining.len() as u16; remaining.len()];
                    let rem = 100 - eq.iter().sum::<u16>();
                    if let Some(last) = eq.last_mut() { *last += rem; }
                    Some(Node::Split { kind, sizes: eq, children: remaining })
                };
                (tree, extracted)
            } else {
                // Recurse into the child at idx
                let mut new_children: Vec<Node> = Vec::new();
                let mut extracted: Option<Node> = None;
                for (i, child) in children.into_iter().enumerate() {
                    if i == idx {
                        let (rem, ext) = extract_node(child, &path[1..]);
                        extracted = ext;
                        if let Some(r) = rem { new_children.push(r); }
                    } else {
                        new_children.push(child);
                    }
                }
                let tree = if new_children.is_empty() {
                    None
                } else if new_children.len() == 1 {
                    Some(new_children.into_iter().next().unwrap())
                } else {
                    let mut eq = vec![100 / new_children.len() as u16; new_children.len()];
                    let rem = 100 - eq.iter().sum::<u16>();
                    if let Some(last) = eq.last_mut() { *last += rem; }
                    Some(Node::Split { kind, sizes: eq, children: new_children })
                };
                (tree, extracted)
            }
        }
    }
}
