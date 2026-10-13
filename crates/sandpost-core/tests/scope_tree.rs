//! Integration tests for scope tree topology, traversal, and policy updates.

use sandpost_core::{Scope, ScopeIdentifier, ScopeTree, TreeError};

/// Create a scope fixture with an optional parent and initial policy version.
fn scope(parent: Option<ScopeIdentifier>) -> Scope {
    Scope {
        identifier: ScopeIdentifier::new(),
        parent,
        name: "scope".into(),
        description: None,
        filter: String::new(),
        position: 0,
        policy_version: 1,
    }
}

/// Verify traversal order and that invalid moves leave the original tree intact.
#[test]
fn traversal_and_atomic_moves() {
    let root_scope = scope(None);
    let child_scope = scope(Some(root_scope.identifier));
    let grandchild_scope = scope(Some(child_scope.identifier));
    let destination_scope = scope(None);
    let mut tree = ScopeTree::new(
        [
            root_scope.clone(),
            child_scope.clone(),
            grandchild_scope.clone(),
            destination_scope.clone(),
        ],
        Some(2),
    )
    .unwrap();
    assert_eq!(
        tree.ancestors(grandchild_scope.identifier).unwrap(),
        vec![child_scope.identifier, root_scope.identifier]
    );
    assert_eq!(
        tree.subtree(child_scope.identifier).unwrap(),
        vec![child_scope.identifier, grandchild_scope.identifier]
    );
    assert_eq!(
        tree.move_scope(root_scope.identifier, Some(grandchild_scope.identifier)),
        Err(TreeError::Cycle)
    );
    assert_eq!(tree.get(root_scope.identifier).unwrap().parent, None);
    assert_eq!(
        tree.move_scope(child_scope.identifier, Some(destination_scope.identifier))
            .unwrap(),
        vec![child_scope.identifier, grandchild_scope.identifier]
    );
    assert_eq!(tree.children(root_scope.identifier), &[]);
    assert_eq!(
        tree.ancestors(grandchild_scope.identifier).unwrap(),
        vec![child_scope.identifier, destination_scope.identifier]
    );
}

/// Reject unknown parents, excessive depth, and duplicate scope identifiers.
#[test]
fn topology_validation() {
    let root_scope = scope(None);
    let child_scope = scope(Some(root_scope.identifier));
    assert!(matches!(
        ScopeTree::new([child_scope.clone()], None),
        Err(TreeError::Unknown(_))
    ));
    assert!(matches!(
        ScopeTree::new([root_scope.clone(), child_scope], Some(0)),
        Err(TreeError::Depth(0))
    ));
    assert!(matches!(
        ScopeTree::new([root_scope.clone(), root_scope], None),
        Err(TreeError::Duplicate(_))
    ));
}

/// Verify filter edits advance versions only in the affected subtree.
#[test]
fn policy_invalidation_versions_only_the_affected_subtree() {
    let root_scope = scope(None);
    let child_scope = scope(Some(root_scope.identifier));
    let unrelated_scope = scope(None);
    let mut tree = ScopeTree::new(
        [
            root_scope.clone(),
            child_scope.clone(),
            unrelated_scope.clone(),
        ],
        None,
    )
    .unwrap();
    assert_eq!(
        tree.set_filter(root_scope.identifier, "false".into())
            .unwrap(),
        vec![root_scope.identifier, child_scope.identifier]
    );
    assert_eq!(tree.get(root_scope.identifier).unwrap().policy_version, 2);
    assert_eq!(tree.get(child_scope.identifier).unwrap().policy_version, 2);
    assert_eq!(
        tree.get(unrelated_scope.identifier).unwrap().policy_version,
        1
    );
}

/// Reject disconnected cycles and traverse deep trees without recursion.
#[test]
fn disconnected_cycles_and_deep_trees() {
    let root = scope(None);
    let mut first_cycle_scope = scope(None);
    let second_cycle_scope = scope(Some(first_cycle_scope.identifier));
    first_cycle_scope.parent = Some(second_cycle_scope.identifier);
    assert!(matches!(
        ScopeTree::new([root.clone(), first_cycle_scope, second_cycle_scope], None),
        Err(TreeError::Cycle)
    ));
    let mut scopes = vec![root.clone()];
    let mut parent = root.identifier;
    for _ in 0..2000 {
        let next = scope(Some(parent));
        parent = next.identifier;
        scopes.push(next);
    }
    let tree = ScopeTree::new(scopes, None).unwrap();
    assert_eq!(tree.ancestors(parent).unwrap().len(), 2000);
    assert_eq!(tree.subtree(root.identifier).unwrap().len(), 2001);
}
