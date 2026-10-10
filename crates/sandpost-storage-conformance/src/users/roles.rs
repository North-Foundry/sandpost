//! User bootstrap, global role, and last-owner invariant checks.
use sandpost_core::{GlobalRole, MailAccess, User};
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

/// Verify every global role round-trips through storage.
pub async fn global_roles(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "global_roles";
    for role in [
        GlobalRole::Owner,
        GlobalRole::Admin,
        GlobalRole::Member,
        GlobalRole::Viewer,
    ] {
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
        mail_access: last.mail_access,
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
                    mail_access: MailAccess::All,
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
