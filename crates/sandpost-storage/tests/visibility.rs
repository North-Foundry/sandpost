//! Public API regression tests for versioned, materialized scope visibility.
mod common;

use common::scope;
use sandpost_core::{MessageSequence, ScopeIdentifier};
use sandpost_storage::{Storage, StorageError};

/// Preserve an existing visible set when replacement references an unknown message.
#[test]
fn failed_match_replacement_rolls_back_previous_matches() {
    let storage = Storage::memory().expect("create in-memory storage");
    let saved_scope = scope(1);
    let scope_identifier = saved_scope.identifier;
    storage.save_scope(&saved_scope).expect("save scope");
    let visible_sequence = common::insert_message(&storage, "visible before replacement");
    let other_sequence = common::insert_message(&storage, "candidate replacement");
    storage
        .replace_scope_matches(scope_identifier, 1, &[visible_sequence])
        .expect("set initial matches");

    assert!(
        storage
            .replace_scope_matches(scope_identifier, 1, &[other_sequence, MessageSequence(999)],)
            .is_err()
    );

    let visible = storage
        .list_visible_messages(&[scope_identifier], None, 100)
        .expect("read matches after failed replacement");
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].sequence, visible_sequence);
}

/// Require policy versions to increase for filter or parent changes and reject regressions.
#[test]
fn scope_policy_changes_require_increasing_versions() {
    let storage = Storage::memory().expect("create in-memory storage");
    let initial_scope = scope(2);
    let scope_identifier = initial_scope.identifier;
    storage
        .save_scope(&initial_scope)
        .expect("save initial scope");

    let mut regressed_version = initial_scope.clone();
    regressed_version.policy_version = 1;
    assert!(matches!(
        storage.save_scope(&regressed_version),
        Err(StorageError::PolicyVersionConflict(identifier)) if identifier == scope_identifier
    ));

    let mut changed_filter = initial_scope.clone();
    changed_filter.filter = "subject contains 'updated'".into();
    assert!(matches!(
        storage.save_scope(&changed_filter),
        Err(StorageError::PolicyVersionConflict(identifier)) if identifier == scope_identifier
    ));

    let mut changed_parent = initial_scope.clone();
    changed_parent.parent = Some(ScopeIdentifier::new());
    assert!(matches!(
        storage.save_scope(&changed_parent),
        Err(StorageError::PolicyVersionConflict(identifier)) if identifier == scope_identifier
    ));

    let persisted_scopes = storage.load_scopes().expect("load saved scope");
    assert_eq!(persisted_scopes.len(), 1);
    assert_eq!(persisted_scopes[0].policy_version, 2);
    assert_eq!(persisted_scopes[0].filter, initial_scope.filter);
    assert_eq!(persisted_scopes[0].parent, None);
}

/// Union scope matches without duplicates and hide matches from stale policy versions.
#[test]
fn visible_scope_union_deduplicates_and_filters_stale_matches() {
    let storage = Storage::memory().expect("create in-memory storage");
    let first_scope = scope(1);
    let second_scope = scope(1);
    storage.save_scope(&first_scope).expect("save first scope");
    storage
        .save_scope(&second_scope)
        .expect("save second scope");
    let shared_sequence = common::insert_message(&storage, "shared match");
    let first_only_sequence = common::insert_message(&storage, "first scope match");
    storage
        .replace_scope_matches(
            first_scope.identifier,
            1,
            &[shared_sequence, first_only_sequence],
        )
        .expect("set first scope matches");
    storage
        .replace_scope_matches(second_scope.identifier, 1, &[shared_sequence])
        .expect("set second scope matches");

    let union = storage
        .list_visible_messages(
            &[first_scope.identifier, second_scope.identifier],
            None,
            100,
        )
        .expect("list scope union");
    assert_eq!(
        union
            .iter()
            .map(|summary| summary.sequence)
            .collect::<Vec<_>>(),
        vec![first_only_sequence, shared_sequence]
    );

    let mut updated_scope = first_scope.clone();
    updated_scope.policy_version = 2;
    updated_scope.filter = "subject contains 'new policy'".into();
    storage
        .save_scope(&updated_scope)
        .expect("advance scope policy");
    let current_matches = storage
        .list_visible_messages(&[first_scope.identifier], None, 100)
        .expect("list current-policy matches");
    assert!(current_matches.is_empty());
}
