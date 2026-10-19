//! Concurrent owner deletion and demotion invariants.
use sandpost_core::{GlobalRole, MailAccess};
use sandpost_storage::{NewUser, StorageError, UpdateUser, UserStorage};
use sandpost_storage_sqlite::SqliteStorage;

/**
 * Serialize concurrent deletion and demotion so exactly one can remove an owner.
 */
#[tokio::test]
async fn deleting_and_demoting_distinct_owners_concurrently_keeps_one_owner() {
    let path = std::env::temp_dir().join(format!(
        "sandpost-owner-concurrency-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let storage = SqliteStorage::open(&path).await.unwrap();
    let other_storage = SqliteStorage::open(&path).await.unwrap();
    let create = |email: &str| NewUser {
        name: email.into(),
        email: email.into(),
        password_hash: "hash".into(),
        global_role: GlobalRole::Owner,
        mail_access: MailAccess::All,
        personal_filter: None,
    };
    let first = storage
        .create_first_owner(&create("first@example.test"))
        .await
        .unwrap();
    let second = storage
        .create_user(&create("second@example.test"))
        .await
        .unwrap();
    let demotion = UpdateUser {
        name: second.name.clone(),
        email: second.email.clone(),
        password_hash: second.password_hash.clone(),
        global_role: GlobalRole::Member,
        mail_access: second.mail_access,
        personal_filter: None,
    };

    let (deleted, demoted) = tokio::join!(
        storage.delete_user(first.identifier),
        other_storage.update_user(second.identifier, &demotion)
    );
    assert!(matches!(deleted, Ok(true)) ^ matches!(demoted, Ok(Some(_))));
    assert!(
        matches!(deleted, Err(StorageError::LastOwner))
            ^ matches!(demoted, Err(StorageError::LastOwner))
    );
    assert_eq!(
        storage
            .list_users()
            .await
            .unwrap()
            .iter()
            .filter(|user| user.global_role == GlobalRole::Owner)
            .count(),
        1
    );
    storage.close().await;
    other_storage.close().await;
    let _ = std::fs::remove_file(path);
}

/**
 * Verify two concurrent deletions or demotions cannot remove both owners.
 */
#[tokio::test]
async fn concurrent_same_kind_owner_changes_keep_one_owner() {
    for demote in [false, true] {
        let path = std::env::temp_dir().join(format!(
            "sandpost-owner-same-kind-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let first_storage = SqliteStorage::open(&path).await.unwrap();
        let second_storage = SqliteStorage::open(&path).await.unwrap();
        let first = first_storage
            .create_first_owner(&owner_input("first"))
            .await
            .unwrap();
        let second = first_storage
            .create_user(&owner_input("second"))
            .await
            .unwrap();
        let first_changes = owner_demotion(&first);
        let second_changes = owner_demotion(&second);

        let (first_result, second_result) = if demote {
            let (first_result, second_result) = tokio::join!(
                first_storage.update_user(first.identifier, &first_changes),
                second_storage.update_user(second.identifier, &second_changes)
            );
            (
                first_result.map(|user| user.is_some()),
                second_result.map(|user| user.is_some()),
            )
        } else {
            tokio::join!(
                first_storage.delete_user(first.identifier),
                second_storage.delete_user(second.identifier)
            )
        };
        assert!(matches!(first_result, Ok(true)) ^ matches!(second_result, Ok(true)));
        assert!(
            matches!(first_result, Err(StorageError::LastOwner))
                ^ matches!(second_result, Err(StorageError::LastOwner))
        );
        assert_eq!(
            first_storage
                .list_users()
                .await
                .unwrap()
                .iter()
                .filter(|user| user.global_role == GlobalRole::Owner)
                .count(),
            1
        );
        first_storage.close().await;
        second_storage.close().await;
        let _ = std::fs::remove_file(path);
    }
}

/**
 * Refuse both deletion and demotion of the only Owner across independent connections.
 */
#[tokio::test]
async fn concurrent_final_owner_delete_and_demotion_return_last_owner() {
    let path = std::env::temp_dir().join(format!(
        "sandpost-final-owner-concurrency-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let storage = SqliteStorage::open(&path).await.unwrap();
    let other_storage = SqliteStorage::open(&path).await.unwrap();
    let owner = storage
        .create_first_owner(&NewUser {
            name: "Owner".into(),
            email: "owner@example.test".into(),
            password_hash: "hash".into(),
            global_role: GlobalRole::Owner,
            mail_access: MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    let changes = UpdateUser {
        name: owner.name.clone(),
        email: owner.email.clone(),
        password_hash: owner.password_hash.clone(),
        global_role: GlobalRole::Member,
        mail_access: owner.mail_access,
        personal_filter: None,
    };
    let (deleted, demoted) = tokio::join!(
        storage.delete_user(owner.identifier),
        other_storage.update_user(owner.identifier, &changes)
    );
    assert!(matches!(deleted, Err(StorageError::LastOwner)));
    assert!(matches!(demoted, Err(StorageError::LastOwner)));
    assert!(storage.get_user(owner.identifier).await.unwrap().is_some());
    storage.close().await;
    other_storage.close().await;
    let _ = std::fs::remove_file(path);
}

/**
 * Create one Owner with deterministic test fields and a unique email.
 */
fn owner_input(label: &str) -> NewUser {
    NewUser {
        name: label.into(),
        email: format!("{label}-{}@example.test", uuid::Uuid::new_v4()),
        password_hash: "hash".into(),
        global_role: GlobalRole::Owner,
        mail_access: MailAccess::All,
        personal_filter: None,
    }
}

/**
 * Build a role demotion that preserves every other Owner field.
 */
fn owner_demotion(owner: &sandpost_core::User) -> UpdateUser {
    UpdateUser {
        name: owner.name.clone(),
        email: owner.email.clone(),
        password_hash: owner.password_hash.clone(),
        global_role: GlobalRole::Member,
        mail_access: owner.mail_access,
        personal_filter: owner.personal_filter.clone(),
    }
}
