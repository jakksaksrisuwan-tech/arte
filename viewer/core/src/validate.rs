//! Validation beyond what serde enforces. Serde already rejects unknown
//! component types and missing required fields; here we catch the semantic rules.

use std::collections::HashSet;

use crate::protocol::UiNode;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ValidationError {
    #[error("empty id on a {kind} node")]
    EmptyId { kind: &'static str },
    #[error("duplicate id '{0}' within surface")]
    DuplicateId(String),
    #[error("table '{id}' row {row} has {got} cells, expected {expected}")]
    TableShape {
        id: String,
        row: usize,
        got: usize,
        expected: usize,
    },
    #[error("progress '{id}' value {value} out of range 0.0..=1.0")]
    ProgressRange { id: String, value: f64 },
}

/// Validate a surface's root node tree.
pub fn validate_tree(root: &UiNode) -> Result<(), ValidationError> {
    let mut seen = HashSet::new();
    walk(root, &mut seen)
}

fn walk(node: &UiNode, seen: &mut HashSet<String>) -> Result<(), ValidationError> {
    let id = node.id();
    if id.is_empty() {
        return Err(ValidationError::EmptyId { kind: kind(node) });
    }
    if !seen.insert(id.to_string()) {
        return Err(ValidationError::DuplicateId(id.to_string()));
    }

    match node {
        UiNode::Table { id, columns, rows } => {
            for (i, row) in rows.iter().enumerate() {
                if row.len() != columns.len() {
                    return Err(ValidationError::TableShape {
                        id: id.clone(),
                        row: i,
                        got: row.len(),
                        expected: columns.len(),
                    });
                }
            }
        }
        UiNode::Progress { id, value, .. } => {
            if !(0.0..=1.0).contains(value) {
                return Err(ValidationError::ProgressRange {
                    id: id.clone(),
                    value: *value,
                });
            }
        }
        UiNode::Panel { children, .. } => {
            for child in children {
                walk(child, seen)?;
            }
        }
        _ => {}
    }
    Ok(())
}

fn kind(node: &UiNode) -> &'static str {
    match node {
        UiNode::Panel { .. } => "Panel",
        UiNode::Text { .. } => "Text",
        UiNode::Metric { .. } => "Metric",
        UiNode::Table { .. } => "Table",
        UiNode::Log { .. } => "Log",
        UiNode::Progress { .. } => "Progress",
        UiNode::Board { .. } => "Board",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{LayoutKind, UiNode};

    fn panel(children: Vec<UiNode>) -> UiNode {
        UiNode::Panel {
            id: "root".into(),
            title: None,
            layout: LayoutKind::Vertical,
            children,
        }
    }

    #[test]
    fn accepts_valid_tree() {
        let tree = panel(vec![
            UiNode::Text { id: "t".into(), text: "hi".into() },
            UiNode::Progress { id: "p".into(), label: "l".into(), value: 0.5 },
        ]);
        assert!(validate_tree(&tree).is_ok());
    }

    #[test]
    fn rejects_empty_id() {
        let tree = panel(vec![UiNode::Text { id: "".into(), text: "x".into() }]);
        assert!(matches!(validate_tree(&tree), Err(ValidationError::EmptyId { .. })));
    }

    #[test]
    fn rejects_duplicate_id() {
        let tree = panel(vec![
            UiNode::Text { id: "dup".into(), text: "a".into() },
            UiNode::Text { id: "dup".into(), text: "b".into() },
        ]);
        assert_eq!(
            validate_tree(&tree),
            Err(ValidationError::DuplicateId("dup".into()))
        );
    }

    #[test]
    fn rejects_ragged_table() {
        let tree = panel(vec![UiNode::Table {
            id: "tbl".into(),
            columns: vec!["a".into(), "b".into()],
            rows: vec![vec!["1".into()]],
        }]);
        assert!(matches!(
            validate_tree(&tree),
            Err(ValidationError::TableShape { .. })
        ));
    }

    #[test]
    fn rejects_progress_out_of_range() {
        let tree = panel(vec![UiNode::Progress {
            id: "p".into(),
            label: "l".into(),
            value: 1.5,
        }]);
        assert!(matches!(
            validate_tree(&tree),
            Err(ValidationError::ProgressRange { .. })
        ));
    }
}
