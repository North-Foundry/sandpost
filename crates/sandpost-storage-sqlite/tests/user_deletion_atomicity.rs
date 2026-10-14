//! Atomic user-deletion regressions against public storage operations.
#[allow(dead_code)]
mod common;

use sandpost_core::{GlobalRole, ImapMailboxSource, MailAccess, MessageIdentifier};
use sandpost_query::Expression;
use sandpost_storage::{
    ImapMembershipDefinition, ImapStorage, MessageStorage, NewImapAccount, NewUser, UserStorage,
};
use sandpost_storage_sqlite::SqliteStorage;
use sqlx::Connection;

/// A failure midway through bulk tombstoning rolls back earlier tombstones and mailbox removals.
#[tokio::test]
async fn failed_bulk_user_deletion_preserves_messages_and_memberships() {
    let path = std::env::temp_dir().join(format!(
        "sandpost-deletion-atomicity-{}.sqlite",
        MessageIdentifier::new()
    ));
    let storage = SqliteStorage::open(&path).await.unwrap();
    let user = storage
        .create_user(&NewUser {
            name: "Reader".into(),
            email: "reader@example.test".into(),
            password_hash: "hash".into(),
            global_role: GlobalRole::Member,
            mail_access: MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    let account = storage
        .create_imap_account(&NewImapAccount {
            owner_identifier: user.identifier,
            name: "Reader".into(),
            username: "reader".into(),
            password_hash: "hash".into(),
            mirror_views: true,
            mode: sandpost_core::ImapAccountMode::Mirror,
            initial_authorization: sandpost_query::Expression::True,
            created_at: 0,
        })
        .await
        .unwrap();
    let first = common::message("first");
    let second = common::message("second");
    storage.insert_message(&first).await.unwrap();
    storage.insert_message(&second).await.unwrap();
    let mailbox = storage
        .reconcile_imap_mailboxes(account.identifier, &[ImapMailboxSource::Inbox], None)
        .await
        .unwrap()[0];
    let definition = ImapMembershipDefinition {
        expected_mode_generation: None,
        filter: Expression::True,
        fingerprint: "all".into(),
    };
    storage
        .synchronize_imap_mailbox(mailbox.identifier, &definition)
        .await
        .unwrap();
    let before = storage
        .imap_mailbox_members(mailbox.identifier, 0)
        .await
        .unwrap();
    assert_eq!(before.len(), 2);
    let mut connection = common::connection(&path).await;
    sqlx::query("CREATE TRIGGER fail_second_user_deletion BEFORE INSERT ON user_message_deletions WHEN (SELECT COUNT(*) FROM user_message_deletions) > 0 BEGIN SELECT RAISE(ABORT, 'injected deletion failure'); END")
        .execute(&mut connection).await.unwrap();
    assert!(
        storage
            .delete_user_messages(user.identifier, None, &Expression::True)
            .await
            .is_err()
    );
    for identifier in [first.identifier, second.identifier] {
        assert!(
            storage
                .get_user_message(user.identifier, identifier)
                .await
                .unwrap()
                .is_some()
        );
    }
    assert_eq!(
        storage
            .imap_mailbox_members(mailbox.identifier, 0)
            .await
            .unwrap(),
        before
    );
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_message_deletions")
        .fetch_one(&mut connection)
        .await
        .unwrap();
    assert_eq!(count, 0);
    sqlx::query("DROP TRIGGER fail_second_user_deletion")
        .execute(&mut connection)
        .await
        .unwrap();
    assert_eq!(
        storage
            .delete_user_messages(user.identifier, None, &Expression::True)
            .await
            .unwrap(),
        2
    );
    assert!(
        storage
            .imap_mailbox_members(mailbox.identifier, 0)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        storage
            .get_message(first.identifier)
            .await
            .unwrap()
            .is_some()
    );
    connection.close().await.unwrap();
    storage.close().await;
    std::fs::remove_file(path).unwrap();
}
