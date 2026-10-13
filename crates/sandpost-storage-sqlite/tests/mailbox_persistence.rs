//! File-backed persistence tests for Web and IMAP mailbox organization.
#[allow(dead_code)]
mod common;

use common::message;
use sandpost_core::{GlobalRole, ImapMailboxSource, MailAccess};
use sandpost_query::Expression;
use sandpost_storage::{
    ImapMembershipDefinition, ImapStorage, MessageListQuery, MessageStorage, NewImapAccount,
    NewUser, UserStorage,
};
use sandpost_storage_sqlite::SqliteStorage;

/// Web filing, selected exposure, and account-local MOVE/COPY placements survive reopening.
#[tokio::test]
async fn web_folder_and_imap_placements_survive_file_reopen() {
    let path = std::env::temp_dir().join(format!(
        "sandpost-mailbox-persistence-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let storage = SqliteStorage::open(&path).await.unwrap();
    let owner = storage
        .create_first_owner(&NewUser {
            name: "Owner".into(),
            email: "mailbox-persistence@example.test".into(),
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
            username: format!("mailbox-persistence-{}", uuid::Uuid::new_v4()),
            password_hash: "hash".into(),
            mirror_views: false,
            created_at: 1,
        })
        .await
        .unwrap();
    let web_folder = storage
        .create_web_folder(owner.identifier, "Filed", &[account.identifier], 1)
        .await
        .unwrap();
    let private_override = storage
        .create_imap_private_folder(account.identifier, "Private override", 1)
        .await
        .unwrap();
    let copy_destination = storage
        .create_imap_private_folder(account.identifier, "Copy destination", 1)
        .await
        .unwrap();

    let web_message = message("Web filed message");
    let web_message_identifier = web_message.identifier;
    storage.insert_message(&web_message).await.unwrap();
    let moved_message = message("Private override message");
    let moved_message_identifier = moved_message.identifier;
    storage.insert_message(&moved_message).await.unwrap();
    let copied_message = message("Copied message");
    let copied_message_identifier = copied_message.identifier;
    storage.insert_message(&copied_message).await.unwrap();
    storage
        .place_user_messages(
            owner.identifier,
            &[web_message_identifier],
            Some(web_folder.identifier),
            &Expression::True,
        )
        .await
        .unwrap();

    let sources = [
        ImapMailboxSource::Inbox,
        ImapMailboxSource::LocalFolder(private_override.identifier),
        ImapMailboxSource::LocalFolder(copy_destination.identifier),
    ];
    let mailboxes = storage
        .reconcile_imap_mailboxes(account.identifier, &sources)
        .await
        .unwrap();
    let mailbox_for = |source| {
        mailboxes
            .iter()
            .find(|mailbox| mailbox.source == source)
            .unwrap()
            .identifier
    };
    let inbox = mailbox_for(ImapMailboxSource::Inbox);
    let private_mailbox = mailbox_for(ImapMailboxSource::LocalFolder(private_override.identifier));
    let copy_mailbox = mailbox_for(ImapMailboxSource::LocalFolder(copy_destination.identifier));
    let definition = ImapMembershipDefinition {
        filter: Expression::True,
        fingerprint: "mailbox-persistence:all".into(),
    };
    for mailbox in [inbox, private_mailbox, copy_mailbox] {
        storage
            .synchronize_imap_mailbox(mailbox, &definition)
            .await
            .unwrap();
    }
    let members = storage.imap_mailbox_members(inbox, 0).await.unwrap();
    let moved_uid = members
        .iter()
        .find(|member| member.message_identifier == moved_message_identifier)
        .unwrap()
        .uid;
    let copied_uid = members
        .iter()
        .find(|member| member.message_identifier == copied_message_identifier)
        .unwrap()
        .uid;
    storage
        .copy_imap_members(
            inbox,
            &[moved_uid],
            private_mailbox,
            Some(&definition),
            true,
        )
        .await
        .unwrap();
    storage
        .copy_imap_members(inbox, &[copied_uid], copy_mailbox, Some(&definition), false)
        .await
        .unwrap();
    storage.close().await;

    let storage = SqliteStorage::open(&path).await.unwrap();
    let folders = storage.list_web_folders(owner.identifier).await.unwrap();
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].identifier, web_folder.identifier);
    assert_eq!(folders[0].name, "Filed");
    assert_eq!(folders[0].exposed_accounts, [account.identifier]);
    let folder_messages = storage
        .list_web_mailbox_messages(
            owner.identifier,
            Some(web_folder.identifier),
            MessageListQuery {
                filter: Some(Expression::True),
                before: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert!(
        folder_messages
            .iter()
            .any(|summary| summary.identifier == web_message_identifier)
    );

    let mailboxes = storage
        .reconcile_imap_mailboxes(account.identifier, &sources)
        .await
        .unwrap();
    for mailbox in &mailboxes {
        storage
            .synchronize_imap_mailbox(mailbox.identifier, &definition)
            .await
            .unwrap();
    }
    let members_for = |source| {
        mailboxes
            .iter()
            .find(|mailbox| mailbox.source == source)
            .unwrap()
            .identifier
    };
    let inbox_members = storage
        .imap_mailbox_members(members_for(ImapMailboxSource::Inbox), 0)
        .await
        .unwrap();
    let private_members = storage
        .imap_mailbox_members(
            members_for(ImapMailboxSource::LocalFolder(private_override.identifier)),
            0,
        )
        .await
        .unwrap();
    let copy_members = storage
        .imap_mailbox_members(
            members_for(ImapMailboxSource::LocalFolder(copy_destination.identifier)),
            0,
        )
        .await
        .unwrap();
    assert!(
        !inbox_members
            .iter()
            .any(|member| member.message_identifier == moved_message_identifier)
    );
    assert!(
        private_members
            .iter()
            .any(|member| member.message_identifier == moved_message_identifier)
    );
    assert!(
        inbox_members
            .iter()
            .any(|member| member.message_identifier == copied_message_identifier)
    );
    assert!(
        copy_members
            .iter()
            .any(|member| member.message_identifier == copied_message_identifier)
    );

    storage.close().await;
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("sqlite-wal"));
    let _ = std::fs::remove_file(path.with_extension("sqlite-shm"));
}
