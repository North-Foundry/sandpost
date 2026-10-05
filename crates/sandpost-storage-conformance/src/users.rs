//! User, global-role, and bootstrap conformance checks.
use sandpost_core::{GlobalRole, User, UserIdentifier};
use sandpost_storage::{NewUser, Storage, StorageError, UpdateUser};

use crate::ConformanceFailure;
use crate::support::{
    create_user, failure, new_user, unique_email, unwrap_storage, verify, verify_equal,
};

/// Verify first-run bootstrap: the first owner is created once and only once.
pub async fn bootstrap(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "bootstrap";
    verify_equal(
        CHECK,
        "count_users on an empty storage",
        unwrap_storage(CHECK, "count_users", storage.count_users().await)?,
        0,
    )?;
    let email = unique_email("bootstrap-owner");
    let requested = NewUser {
        global_role: GlobalRole::Member,
        ..new_user(GlobalRole::Member, &email)
    };
    let owner = unwrap_storage(
        CHECK,
        "create_first_owner",
        storage.create_first_owner(&requested).await,
    )?;
    verify_equal(
        CHECK,
        "create_first_owner forces the owner role",
        owner.global_role,
        GlobalRole::Owner,
    )?;
    verify_equal(
        CHECK,
        "create_first_owner stores the requested email",
        owner.email,
        email,
    )?;
    let repeated = storage
        .create_first_owner(&new_user(
            GlobalRole::Owner,
            &unique_email("bootstrap-again"),
        ))
        .await;
    verify(
        CHECK,
        matches!(&repeated, Err(StorageError::InstanceAlreadyInitialized)),
        format!(
            "a second create_first_owner must fail with InstanceAlreadyInitialized, got {repeated:?}"
        ),
    )?;
    Ok(())
}

/// Verify user create, read, update, and delete behaviour.
pub async fn user_crud(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "user_crud";
    let email = unique_email("crud");
    let created = unwrap_storage(
        CHECK,
        "create_user",
        storage
            .create_user(&NewUser {
                name: "Original name".to_owned(),
                email: email.clone(),
                password_hash: "hash-one".to_owned(),
                global_role: GlobalRole::Member,
                personal_filter: Some("subject == \"one\"".to_owned()),
            })
            .await,
    )?;

    let by_identifier = unwrap_storage(
        CHECK,
        "get_user",
        storage.get_user(created.identifier).await,
    )?
    .ok_or_else(|| failure(CHECK, "get_user returned none for a created user"))?;
    verify_equal(CHECK, "get_user result", by_identifier, created.clone())?;
    let by_email = unwrap_storage(
        CHECK,
        "get_user_by_email",
        storage.get_user_by_email(&email).await,
    )?
    .ok_or_else(|| failure(CHECK, "get_user_by_email returned none for a created user"))?;
    verify_equal(
        CHECK,
        "get_user_by_email result",
        by_email.identifier,
        created.identifier,
    )?;
    let listed = unwrap_storage(CHECK, "list_users", storage.list_users().await)?;
    verify(
        CHECK,
        listed
            .iter()
            .any(|user| user.identifier == created.identifier),
        "list_users omitted a created user",
    )?;

    let changed_email = unique_email("crud-updated");
    let changes = UpdateUser {
        name: "Updated name".to_owned(),
        email: changed_email.clone(),
        password_hash: "hash-two".to_owned(),
        global_role: GlobalRole::Admin,
        personal_filter: None,
    };
    let updated = unwrap_storage(
        CHECK,
        "update_user",
        storage.update_user(created.identifier, &changes).await,
    )?
    .ok_or_else(|| failure(CHECK, "update_user returned none for a known user"))?;
    verify_equal(
        CHECK,
        "updated name",
        updated.name.clone(),
        "Updated name".to_owned(),
    )?;
    verify_equal(CHECK, "updated email", updated.email.clone(), changed_email)?;
    verify_equal(
        CHECK,
        "updated global role",
        updated.global_role,
        GlobalRole::Admin,
    )?;
    verify_equal(
        CHECK,
        "updated personal filter",
        updated.personal_filter.clone(),
        None,
    )?;
    verify(
        CHECK,
        updated.updated_at >= updated.created_at,
        "updated_at must not precede created_at",
    )?;

    let unknown = UserIdentifier::new();
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "update_user unknown",
            storage.update_user(unknown, &changes).await,
        )?
        .is_none(),
        "update_user on an unknown identifier must return none",
    )?;

    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "delete_user",
            storage.delete_user(created.identifier).await,
        )?,
        "delete_user must report true for an existing user",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "get_user after delete",
            storage.get_user(created.identifier).await,
        )?
        .is_none(),
        "a deleted user must not be readable",
    )?;
    verify(
        CHECK,
        !unwrap_storage(
            CHECK,
            "delete_user twice",
            storage.delete_user(created.identifier).await,
        )?,
        "delete_user must report false for a missing user",
    )?;
    Ok(())
}

/// Verify that duplicate emails are rejected on create and update.
pub async fn duplicate_user_email(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "duplicate_user_email";
    let email = unique_email("duplicate");
    let first = create_user(storage, CHECK, GlobalRole::Member, &email).await?;
    verify_equal(CHECK, "first user email", first.email, email.clone())?;

    let repeated = storage
        .create_user(&new_user(GlobalRole::Member, &email))
        .await;
    verify(
        CHECK,
        matches!(&repeated, Err(StorageError::DuplicateUserEmail)),
        format!(
            "creating a second user with the same email must fail with DuplicateUserEmail, got {repeated:?}"
        ),
    )?;

    let other_email = unique_email("duplicate-other");
    let other = create_user(storage, CHECK, GlobalRole::Member, &other_email).await?;
    let moved = storage
        .update_user(
            other.identifier,
            &UpdateUser {
                name: other.name.clone(),
                email: email.clone(),
                password_hash: other.password_hash.clone(),
                global_role: other.global_role,
                personal_filter: None,
            },
        )
        .await;
    verify(
        CHECK,
        matches!(&moved, Err(StorageError::DuplicateUserEmail)),
        format!(
            "updating a user onto another user's email must fail with DuplicateUserEmail, got {moved:?}"
        ),
    )?;

    let kept = unwrap_storage(
        CHECK,
        "update_user with own email",
        storage
            .update_user(
                other.identifier,
                &UpdateUser {
                    name: other.name.clone(),
                    email: other_email,
                    password_hash: other.password_hash.clone(),
                    global_role: other.global_role,
                    personal_filter: other.personal_filter.clone(),
                },
            )
            .await,
    )?;
    verify(
        CHECK,
        kept.is_some(),
        "updating a user while keeping its own email must succeed",
    )?;
    Ok(())
}

/// Verify every global role round-trips through storage.
pub async fn global_roles(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "global_roles";
    for role in [GlobalRole::Owner, GlobalRole::Admin, GlobalRole::Member] {
        let created = create_user(storage, CHECK, role, &unique_email("role")).await?;
        verify_equal(CHECK, "created global role", created.global_role, role)?;
        let stored = unwrap_storage(
            CHECK,
            "get_user",
            storage.get_user(created.identifier).await,
        )?
        .ok_or_else(|| failure(CHECK, "get_user returned none for a created user"))?;
        verify_equal(CHECK, "stored global role", stored.global_role, role)?;
    }
    Ok(())
}

/// Verify that the final owner cannot be demoted or deleted.
pub async fn last_owner_invariant(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "last_owner_invariant";
    let mut owners: Vec<User> = unwrap_storage(CHECK, "list_users", storage.list_users().await)?
        .into_iter()
        .filter(|user| user.global_role == GlobalRole::Owner)
        .collect();
    verify(
        CHECK,
        !owners.is_empty(),
        "an initialized store must have at least one owner",
    )?;

    // Guarantee at least two owners so a successful owner deletion can be observed.
    owners.push(
        create_user(
            storage,
            CHECK,
            GlobalRole::Owner,
            &unique_email("owner-extra"),
        )
        .await?,
    );
    let last = owners
        .pop()
        .ok_or_else(|| failure(CHECK, "expected a final owner candidate"))?;
    for owner in owners {
        verify(
            CHECK,
            unwrap_storage(
                CHECK,
                "delete redundant owner",
                storage.delete_user(owner.identifier).await,
            )?,
            "deleting an owner while another owner exists must succeed",
        )?;
    }

    let demotion = UpdateUser {
        name: last.name.clone(),
        email: last.email.clone(),
        password_hash: last.password_hash.clone(),
        global_role: GlobalRole::Member,
        personal_filter: last.personal_filter.clone(),
    };
    let demoted = storage.update_user(last.identifier, &demotion).await;
    verify(
        CHECK,
        matches!(&demoted, Err(StorageError::LastOwner)),
        format!("demoting the final owner must fail with LastOwner, got {demoted:?}"),
    )?;
    let deleted = storage.delete_user(last.identifier).await;
    verify(
        CHECK,
        matches!(&deleted, Err(StorageError::LastOwner)),
        format!("deleting the final owner must fail with LastOwner, got {deleted:?}"),
    )?;

    let retained = unwrap_storage(
        CHECK,
        "update final owner without demotion",
        storage
            .update_user(
                last.identifier,
                &UpdateUser {
                    name: "Renamed final owner".to_owned(),
                    email: last.email.clone(),
                    password_hash: last.password_hash.clone(),
                    global_role: GlobalRole::Owner,
                    personal_filter: None,
                },
            )
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "updating the final owner returned none"))?;
    verify_equal(
        CHECK,
        "final owner name",
        retained.name,
        "Renamed final owner".to_owned(),
    )?;
    verify_equal(
        CHECK,
        "final owner role",
        retained.global_role,
        GlobalRole::Owner,
    )?;
    Ok(())
}
