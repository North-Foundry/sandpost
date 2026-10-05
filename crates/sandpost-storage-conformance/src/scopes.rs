//! Scope and scope-membership conformance checks.
use sandpost_core::{
    EndpointMembership, EndpointRole, GlobalRole, MailAccess, Scope, ScopeIdentifier,
};
use sandpost_storage::{Storage, StorageError};

use crate::ConformanceFailure;
use crate::support::{
    create_user, failure, save_endpoint, save_scope, unique_email, unwrap_storage, verify,
    verify_equal,
};

/// Verify scope create, read, update, delete, and policy-version behaviour.
pub async fn scope_crud(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "scope_crud";
    let endpoint = save_endpoint(storage, CHECK, "scope crud endpoint").await?;
    let identifier = ScopeIdentifier::new();
    let root = Scope {
        identifier,
        endpoint_identifier: endpoint,
        parent: None,
        name: "root scope".to_owned(),
        description: Some("root description".to_owned()),
        filter: "subject == \"alpha\"".to_owned(),
        position: 0,
        policy_version: 1,
    };
    unwrap_storage(CHECK, "save_scope", storage.save_scope(&root).await)?;
    let stored = unwrap_storage(CHECK, "get_scope", storage.get_scope(identifier).await)?
        .ok_or_else(|| failure(CHECK, "get_scope returned none for a saved scope"))?;
    verify_scope(CHECK, &stored, &root)?;
    let listed = unwrap_storage(CHECK, "list_scopes", storage.list_scopes().await)?;
    verify(
        CHECK,
        listed.iter().any(|scope| scope.identifier == identifier),
        "list_scopes omitted a saved scope",
    )?;

    let conflicting = Scope {
        filter: "subject == \"beta\"".to_owned(),
        ..root.clone()
    };
    let conflict = storage.save_scope(&conflicting).await;
    verify(
        CHECK,
        matches!(
            &conflict,
            Err(StorageError::PolicyVersionConflict(scope)) if *scope == identifier
        ),
        format!(
            "changing a filter without advancing the policy version must fail, got {conflict:?}"
        ),
    )?;
    let bumped = Scope {
        filter: "subject == \"beta\"".to_owned(),
        policy_version: 2,
        ..root.clone()
    };
    unwrap_storage(
        CHECK,
        "save_scope with a new version",
        storage.save_scope(&bumped).await,
    )?;
    verify_equal(
        CHECK,
        "bumped filter",
        unwrap_storage(
            CHECK,
            "get_scope after bump",
            storage.get_scope(identifier).await,
        )?
        .ok_or_else(|| failure(CHECK, "bumped scope disappeared"))?
        .filter,
        "subject == \"beta\"".to_owned(),
    )?;

    let child = Scope {
        identifier: ScopeIdentifier::new(),
        endpoint_identifier: endpoint,
        parent: Some(identifier),
        name: "child scope".to_owned(),
        description: None,
        filter: String::new(),
        position: 1,
        policy_version: 1,
    };
    unwrap_storage(CHECK, "save child scope", storage.save_scope(&child).await)?;
    let parent_delete = storage.delete_scope(identifier).await;
    verify(
        CHECK,
        !matches!(&parent_delete, Ok(true)),
        format!("a scope referenced as a parent must not be deleted, got {parent_delete:?}"),
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "get_scope after refused delete",
            storage.get_scope(identifier).await,
        )?
        .is_some(),
        "a refused parent deletion must leave the scope in place",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "delete child scope",
            storage.delete_scope(child.identifier).await,
        )?,
        "deleting a leaf scope must report true",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "get_scope after child delete",
            storage.get_scope(child.identifier).await,
        )?
        .is_none(),
        "a deleted scope must be gone",
    )?;
    Ok(())
}

/// Compare every stored field of a scope against its expected value.
fn verify_scope(
    check: &'static str,
    actual: &Scope,
    expected: &Scope,
) -> Result<(), ConformanceFailure> {
    verify_equal(
        check,
        "scope identifier",
        actual.identifier,
        expected.identifier,
    )?;
    verify_equal(
        check,
        "scope endpoint",
        actual.endpoint_identifier,
        expected.endpoint_identifier,
    )?;
    verify_equal(check, "scope parent", actual.parent, expected.parent)?;
    verify_equal(
        check,
        "scope name",
        actual.name.clone(),
        expected.name.clone(),
    )?;
    verify_equal(
        check,
        "scope description",
        actual.description.clone(),
        expected.description.clone(),
    )?;
    verify_equal(
        check,
        "scope filter",
        actual.filter.clone(),
        expected.filter.clone(),
    )?;
    verify_equal(check, "scope position", actual.position, expected.position)?;
    verify_equal(
        check,
        "scope policy version",
        actual.policy_version,
        expected.policy_version,
    )?;
    Ok(())
}

/// Verify role-less scope memberships and their endpoint-membership invariant.
pub async fn scope_memberships(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "scope_memberships";
    let endpoint = save_endpoint(storage, CHECK, "scope membership endpoint").await?;
    let other_endpoint = save_endpoint(storage, CHECK, "scope membership other endpoint").await?;
    let scoped_user =
        create_user(storage, CHECK, GlobalRole::Member, &unique_email("scoped")).await?;
    let second_user =
        create_user(storage, CHECK, GlobalRole::Member, &unique_email("scoped")).await?;
    let without_membership =
        create_user(storage, CHECK, GlobalRole::Member, &unique_email("orphan")).await?;

    for user in [scoped_user.identifier, second_user.identifier] {
        unwrap_storage(
            CHECK,
            "set scoped endpoint membership",
            storage
                .set_endpoint_membership(&EndpointMembership {
                    user_identifier: user,
                    endpoint_identifier: endpoint,
                    role: EndpointRole::Member,
                    mail_access: MailAccess::Scoped,
                })
                .await,
        )?;
    }

    let first_scope = save_scope(storage, CHECK, endpoint, "first scope").await?;
    let second_scope = save_scope(storage, CHECK, endpoint, "second scope").await?;

    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "list_user_scopes before assignment",
            storage.list_user_scopes(scoped_user.identifier).await,
        )?
        .is_empty(),
        "a scoped member with no assignments must have zero derived access",
    )?;

    unwrap_storage(
        CHECK,
        "assign first scope",
        storage
            .assign_user_to_scope(scoped_user.identifier, first_scope)
            .await,
    )?;
    verify_equal(
        CHECK,
        "one assigned scope",
        unwrap_storage(
            CHECK,
            "list_user_scopes after one assignment",
            storage.list_user_scopes(scoped_user.identifier).await,
        )?,
        vec![first_scope],
    )?;
    verify_equal(
        CHECK,
        "scope members after one assignment",
        unwrap_storage(
            CHECK,
            "list_scope_members",
            storage.list_scope_members(first_scope).await,
        )?,
        vec![scoped_user.identifier],
    )?;

    unwrap_storage(
        CHECK,
        "assign second scope",
        storage
            .assign_user_to_scope(scoped_user.identifier, second_scope)
            .await,
    )?;
    let mut expected_scopes = vec![first_scope, second_scope];
    expected_scopes.sort();
    verify_equal(
        CHECK,
        "two assigned scopes ordered",
        unwrap_storage(
            CHECK,
            "list_user_scopes after two assignments",
            storage.list_user_scopes(scoped_user.identifier).await,
        )?,
        expected_scopes,
    )?;
    unwrap_storage(
        CHECK,
        "assign second user",
        storage
            .assign_user_to_scope(second_user.identifier, first_scope)
            .await,
    )?;
    let mut expected_members = vec![scoped_user.identifier, second_user.identifier];
    expected_members.sort();
    verify_equal(
        CHECK,
        "scope members ordered",
        unwrap_storage(
            CHECK,
            "list_scope_members with two users",
            storage.list_scope_members(first_scope).await,
        )?,
        expected_members,
    )?;

    unwrap_storage(
        CHECK,
        "reassign first scope",
        storage
            .assign_user_to_scope(scoped_user.identifier, first_scope)
            .await,
    )?;
    verify_equal(
        CHECK,
        "reassignment is idempotent",
        unwrap_storage(
            CHECK,
            "list_scope_members after reassignment",
            storage.list_scope_members(first_scope).await,
        )?
        .len(),
        2,
    )?;

    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "remove scope assignment",
            storage
                .remove_user_from_scope(scoped_user.identifier, first_scope)
                .await,
        )?,
        "remove_user_from_scope must report true for an existing assignment",
    )?;
    verify_equal(
        CHECK,
        "one remaining scope",
        unwrap_storage(
            CHECK,
            "list_user_scopes after removal",
            storage.list_user_scopes(scoped_user.identifier).await,
        )?,
        vec![second_scope],
    )?;
    verify(
        CHECK,
        !unwrap_storage(
            CHECK,
            "remove scope assignment twice",
            storage
                .remove_user_from_scope(scoped_user.identifier, first_scope)
                .await,
        )?,
        "remove_user_from_scope must report false when absent",
    )?;

    let orphan_scope = save_scope(storage, CHECK, other_endpoint, "orphan scope").await?;
    let orphan = storage
        .assign_user_to_scope(without_membership.identifier, orphan_scope)
        .await;
    verify(
        CHECK,
        matches!(&orphan, Err(StorageError::ConstraintViolation(_))),
        format!(
            "assigning a scope without an endpoint membership must fail with ConstraintViolation, got {orphan:?}"
        ),
    )?;

    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "remove endpoint membership",
            storage
                .remove_endpoint_membership(second_user.identifier, endpoint)
                .await,
        )?,
        "removing a member's endpoint membership must report true",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "list_user_scopes after endpoint removal",
            storage.list_user_scopes(second_user.identifier).await,
        )?
        .is_empty(),
        "removing an endpoint membership must remove dependent scope memberships",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "list_scope_members after endpoint removal",
            storage.list_scope_members(first_scope).await,
        )?
        .iter()
        .all(|member| *member != second_user.identifier),
        "scope members must drop a removed endpoint member",
    )?;

    let unknown = storage
        .assign_user_to_scope(scoped_user.identifier, ScopeIdentifier::new())
        .await;
    verify(
        CHECK,
        matches!(&unknown, Err(StorageError::NotFound)),
        format!("assigning an unknown scope must fail with NotFound, got {unknown:?}"),
    )?;
    Ok(())
}
