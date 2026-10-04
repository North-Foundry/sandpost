use super::*;

/// Verify pagination, match materialization, scope unions, and policy version guards.
#[test]
fn pagination_scope_materialization_union_and_policy_guard() {
    let storage = Storage::memory().unwrap();
    let scope_one = scope(3);
    let scope_two = scope(8);
    storage.save_scope(&scope_one).unwrap();
    storage.save_scope(&scope_two).unwrap();
    assert_eq!(storage.load_scopes().unwrap().len(), 2);
    let first_message = message();
    let first_sequence = storage
        .insert_message(
            &first_message,
            &[(scope_one.identifier, 3), (scope_two.identifier, 8)],
        )
        .unwrap();
    let mut second_message = message();
    second_message.identifier = MessageIdentifier::new();
    let second_sequence = storage
        .insert_message(&second_message, &[(scope_one.identifier, 3)])
        .unwrap();
    let connection = storage.connection().unwrap();
    let count: i64 = connection
        .query_row("SELECT count(*) FROM message_scope", [], |database_row| {
            database_row.get(0)
        })
        .unwrap();
    assert_eq!(count, 3);
    drop(connection);
    assert_eq!(
        storage.list_messages(None, 1).unwrap()[0].sequence,
        second_sequence
    );
    assert_eq!(
        storage.list_messages(Some(second_sequence), 10).unwrap()[0].sequence,
        first_sequence
    );
    let visible = storage
        .list_visible_messages(&[scope_one.identifier], None, 10)
        .unwrap();
    assert_eq!(
        visible.iter().map(|row| row.sequence).collect::<Vec<_>>(),
        vec![second_sequence, first_sequence]
    );
    assert!(
        matches!(storage.replace_scope_matches(scope_one.identifier, 2, &[first_sequence]), Err(StorageError::StalePolicy(identifier)) if identifier == scope_one.identifier)
    );
    storage
        .replace_scope_matches(scope_one.identifier, 3, &[first_sequence])
        .unwrap();
    let connection = storage.connection().unwrap();
    let rows: i64 = connection
        .query_row(
            "SELECT count(*) FROM message_scope WHERE scope_id=?1 AND policy_version=3",
            [scope_one.identifier.to_string()],
            |database_row| database_row.get(0),
        )
        .unwrap();
    assert_eq!(rows, 1);
    drop(connection);
    let mut changed = scope_one.clone();
    changed.filter = "subject contains 'changed'".into();
    assert!(
        matches!(storage.save_scope(&changed), Err(StorageError::PolicyVersionConflict(identifier)) if identifier == scope_one.identifier)
    );
    changed.policy_version = 4;
    storage.save_scope(&changed).unwrap();
    assert!(
        storage
            .list_visible_messages(&[scope_one.identifier], None, 10)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        storage
            .list_visible_messages(&[scope_one.identifier, scope_two.identifier], None, 10)
            .unwrap()
            .len(),
        1
    );
    let mut reparented = changed.clone();
    reparented.parent = Some(scope_two.identifier);
    assert!(matches!(
        storage.save_scope(&reparented),
        Err(StorageError::PolicyVersionConflict(identifier)) if identifier == scope_one.identifier
    ));
    storage
        .replace_scope_matches(scope_one.identifier, 4, &[first_sequence])
        .unwrap();
    assert_eq!(
        storage
            .list_visible_messages(&[scope_one.identifier], None, 10)
            .unwrap()
            .len(),
        1
    );
}
