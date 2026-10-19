//! Durable remembered-session storage behavior.
use sandpost_core::{GlobalRole, MailAccess, MessageIdentifier};
use sandpost_storage::{NewUser, UserStorage};
use sandpost_storage_sqlite::SqliteStorage;

/**
 * Persist sessions across reopen and support explicit revocation and user-deletion cascade.
 */
#[tokio::test]
async fn remembered_sessions_survive_reopen_and_are_revoked() {
    let path = std::env::temp_dir().join(format!(
        "sandpost-persistent-sessions-{}.sqlite",
        MessageIdentifier::new()
    ));
    let storage = SqliteStorage::open(&path).await.unwrap();
    let first_user = storage
        .create_user(&NewUser {
            name: "First".into(),
            email: "first@example.test".into(),
            password_hash: "first-user-password-hash".into(),
            global_role: GlobalRole::Member,
            mail_access: MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    let second_user = storage
        .create_user(&NewUser {
            name: "Second".into(),
            email: "second@example.test".into(),
            password_hash: "second-user-password-hash".into(),
            global_role: GlobalRole::Member,
            mail_access: MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    storage
        .create_persistent_session(
            "sha256-token-digest-one",
            first_user.identifier,
            "verified-first-password-hash",
        )
        .await
        .unwrap();
    storage
        .create_persistent_session(
            "sha256-token-digest-two",
            second_user.identifier,
            "verified-second-password-hash",
        )
        .await
        .unwrap();
    storage.close().await;

    let storage = SqliteStorage::open(&path).await.unwrap();
    assert_eq!(
        storage
            .get_persistent_session("sha256-token-digest-one")
            .await
            .unwrap(),
        Some((first_user.identifier, "verified-first-password-hash".into()))
    );
    storage
        .delete_persistent_session("sha256-token-digest-one")
        .await
        .unwrap();
    assert_eq!(
        storage
            .get_persistent_session("sha256-token-digest-one")
            .await
            .unwrap(),
        None
    );

    storage.delete_user(second_user.identifier).await.unwrap();
    assert_eq!(
        storage
            .get_persistent_session("sha256-token-digest-two")
            .await
            .unwrap(),
        None
    );
    storage.close().await;
    let _ = std::fs::remove_file(path);
}
