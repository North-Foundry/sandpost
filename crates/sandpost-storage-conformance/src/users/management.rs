//! User creation, lookup, update, deletion, and uniqueness checks.
use sandpost_core::{GlobalRole, MailAccess, UserIdentifier};
use sandpost_storage::{NewUser, Storage, StorageError, UpdateUser};

use crate::ConformanceFailure;
use crate::support::{
    create_user, failure, new_user, unique_email, unwrap_storage, verify, verify_equal,
};

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
                mail_access: MailAccess::Scoped,
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
        mail_access: MailAccess::All,
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
        "created mail access",
        created.mail_access,
        MailAccess::Scoped,
    )?;
    verify_equal(
        CHECK,
        "updated mail access",
        updated.mail_access,
        MailAccess::All,
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
                mail_access: other.mail_access,
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
                    mail_access: other.mail_access,
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
