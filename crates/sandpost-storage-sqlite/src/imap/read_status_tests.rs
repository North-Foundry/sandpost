//! Shared read-status persistence checks.

use super::*;
use sandpost_core::{
    GlobalRole, ImapAccountMode, ImapMailboxSource, MailAccess, Message, MessageFacts,
};
use sandpost_storage::{
    ImapMembershipDefinition, MessageListQuery, MessageStorage, NewImapAccount, NewUser,
    UserStorage,
};

/// Shared Seen reaches every Mirror account, remains local to Independent accounts, and appears in Web summaries.
#[tokio::test]
async fn shared_seen_is_visible_to_mirror_accounts_and_web() {
    let storage = crate::SqliteStorage::memory().await.unwrap();
    let user = storage
        .create_user(&NewUser {
            name: "Read status".into(),
            email: format!("read-status-{}@example.test", uuid::Uuid::new_v4()),
            password_hash: "hash".into(),
            global_role: GlobalRole::Member,
            mail_access: MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    let message = Message {
        identifier: MessageIdentifier::new(),
        facts: MessageFacts {
            subject: "Shared Seen".into(),
            size: 4,
            ..Default::default()
        },
        raw_message: b"test".to_vec(),
        attachments: Vec::new(),
    };
    storage.insert_message(&message).await.unwrap();
    let mut inboxes = Vec::new();
    for mode in [
        ImapAccountMode::Mirror,
        ImapAccountMode::Mirror,
        ImapAccountMode::Independent,
    ] {
        let account = storage
            .create_imap_account(&NewImapAccount {
                owner_identifier: user.identifier,
                name: format!("{mode:?}"),
                username: format!("{}-{}", mode.as_str(), uuid::Uuid::new_v4()),
                password_hash: "hash".into(),
                mirror_views: true,
                mode,
                initial_authorization: sandpost_query::Expression::True,
                created_at: 1,
            })
            .await
            .unwrap();
        let mailbox = storage
            .reconcile_imap_mailboxes(account.identifier, &[ImapMailboxSource::Inbox], None)
            .await
            .unwrap()[0]
            .identifier;
        storage
            .synchronize_imap_mailbox(
                mailbox,
                &ImapMembershipDefinition {
                    expected_mode_generation: None,
                    filter: sandpost_query::Expression::True,
                    fingerprint: format!("read-status-{mode:?}"),
                },
            )
            .await
            .unwrap();
        inboxes.push(mailbox);
    }

    assert_eq!(
        storage
            .set_user_message_seen(
                user.identifier,
                &[message.identifier],
                true,
                &sandpost_query::Expression::True,
            )
            .await
            .unwrap(),
        1
    );
    for (index, mailbox) in inboxes.iter().enumerate() {
        let member = &storage.imap_mailbox_members(*mailbox, 0).await.unwrap()[0];
        assert_eq!(member.flags.seen, index < 2);
    }
    let summaries = storage
        .list_web_mailbox_messages(
            user.identifier,
            None,
            MessageListQuery {
                filter: Some(sandpost_query::Expression::True),
                before: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert!(summaries[0].seen);

    assert_eq!(
        storage
            .set_user_message_seen(
                user.identifier,
                &[message.identifier],
                false,
                &sandpost_query::Expression::False,
            )
            .await
            .unwrap(),
        0
    );
    storage
        .delete_user_messages(
            user.identifier,
            Some(&[message.identifier]),
            &sandpost_query::Expression::True,
        )
        .await
        .unwrap();
    assert_eq!(
        storage
            .set_user_message_seen(
                user.identifier,
                &[message.identifier],
                false,
                &sandpost_query::Expression::True,
            )
            .await
            .unwrap(),
        0
    );
    storage.close().await;
}
