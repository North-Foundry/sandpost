//! Subscription retention regressions against the public SQLite storage contract.
use sandpost_core::{GlobalRole, MailAccess};
use sandpost_storage::{ImapStorage, NewImapAccount, NewUser, UserStorage};
use sandpost_storage_sqlite::SqliteStorage;

/// Retaining discovered default names cannot override an explicit unsubscribe.
#[tokio::test]
async fn retain_default_subscriptions_preserves_unsubscribe_exceptions() {
    let path = std::env::temp_dir().join(format!(
        "sandpost-subscriptions-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let storage = SqliteStorage::open(&path).await.unwrap();
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
    let account = storage
        .create_imap_account(&NewImapAccount {
            owner_identifier: owner.identifier,
            name: "Account".into(),
            username: "owner".into(),
            password_hash: "hash".into(),
            mirror_views: true,
            mode: sandpost_core::ImapAccountMode::Independent,
            initial_authorization: sandpost_query::Expression::True,
            created_at: 0,
        })
        .await
        .unwrap();

    storage
        .set_imap_subscription(account.identifier, "INBOX", false)
        .await
        .unwrap();
    storage
        .retain_imap_subscriptions(
            account.identifier,
            &["INBOX".into(), "Private".into(), "Private".into()],
        )
        .await
        .unwrap();

    assert_eq!(
        storage
            .list_imap_subscribed(account.identifier)
            .await
            .unwrap(),
        vec!["Private"]
    );
    assert_eq!(
        storage
            .list_imap_unsubscribed(account.identifier)
            .await
            .unwrap(),
        vec!["INBOX"]
    );
    storage.close().await;
    let _ = std::fs::remove_file(path);
}
