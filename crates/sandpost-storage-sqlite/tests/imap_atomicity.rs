//! Atomic shared-view transfers, mode fences, and reference-safe physical cleanup.
#[allow(dead_code)]
mod common;
use common::{connection, message};
use sandpost_core::{
    ImapAccount, ImapAccountMode, ImapMailboxIdentifier, ImapMailboxSource, UserIdentifier,
};
use sandpost_query::Expression;
use sandpost_storage::{
    ImapAccountSettings, ImapMembershipDefinition, ImapStorage, MessageStorage, NewImapAccount,
    NewUser, SearchSynchronizationStorage, StorageError, UserStorage,
};
use sandpost_storage_sqlite::SqliteStorage;
use sqlx::Connection;

/// Create a primary owner whose retained references can be independently released.
async fn owner(storage: &SqliteStorage) -> UserIdentifier {
    storage
        .create_first_owner(&NewUser {
            name: "Owner".into(),
            email: "atomicity@example.test".into(),
            password_hash: "hash".into(),
            global_role: sandpost_core::GlobalRole::Owner,
            mail_access: sandpost_core::MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap()
        .identifier
}

/// Create an account initialized from the current primary state.
async fn account(
    storage: &SqliteStorage,
    owner: UserIdentifier,
    mode: ImapAccountMode,
) -> ImapAccount {
    storage
        .create_imap_account(&NewImapAccount {
            owner_identifier: owner,
            name: "Atomic view".into(),
            username: uuid::Uuid::new_v4().to_string(),
            password_hash: "hash".into(),
            mirror_views: mode == ImapAccountMode::Mirror,
            mode,
            initial_authorization: Expression::True,
            created_at: 1,
        })
        .await
        .unwrap()
}

/// A complete current authorization definition for the unrestricted fixture owner.
fn definition() -> ImapMembershipDefinition {
    ImapMembershipDefinition {
        expected_mode_generation: None,
        filter: Expression::True,
        fingerprint: "atomicity-all".into(),
    }
}

/// Synchronize an account's Inbox and shared Projects projection before a concurrent operation.
async fn mailboxes(storage: &SqliteStorage, account: &ImapAccount) -> [ImapMailboxIdentifier; 2] {
    let folder = storage
        .list_imap_local_folders(account.identifier)
        .await
        .unwrap()
        .into_iter()
        .find(|folder| folder.name == "Projects")
        .unwrap();
    let states = storage
        .reconcile_imap_mailboxes(
            account.identifier,
            &[
                ImapMailboxSource::Inbox,
                ImapMailboxSource::LocalFolder(folder.identifier),
            ],
            None,
        )
        .await
        .unwrap();
    for state in &states {
        storage
            .synchronize_imap_mailbox(state.identifier, &definition())
            .await
            .unwrap();
    }
    [states[0].identifier, states[1].identifier]
}

/// Two Mirror clients with stale destination projections serialize one canonical COPY occurrence.
#[tokio::test]
async fn concurrent_mirror_copies_refuse_the_duplicate_shared_occurrence() {
    let storage = SqliteStorage::memory().await.unwrap();
    let owner = owner(&storage).await;
    let message = message("One physical message");
    storage.insert_message(&message).await.unwrap();
    let first = account(&storage, owner, ImapAccountMode::Mirror).await;
    let second = account(&storage, owner, ImapAccountMode::Mirror).await;
    storage
        .create_web_folder(owner, "Projects", &[], 1)
        .await
        .unwrap();
    let first_mailboxes = mailboxes(&storage, &first).await;
    let second_mailboxes = mailboxes(&storage, &second).await;
    let authorization = definition();
    let (first_copy, second_copy) = tokio::join!(
        storage.copy_imap_members(
            first_mailboxes[0],
            &[1],
            first_mailboxes[1],
            Some(&authorization),
            false
        ),
        storage.copy_imap_members(
            second_mailboxes[0],
            &[1],
            second_mailboxes[1],
            Some(&authorization),
            false
        )
    );
    assert_ne!(
        first_copy.is_ok(),
        second_copy.is_ok(),
        "only one shared occurrence can be created: {first_copy:?}, {second_copy:?}"
    );
    let refused = if first_copy.is_err() {
        first_copy.unwrap_err()
    } else {
        second_copy.unwrap_err()
    };
    assert!(matches!(refused, StorageError::ConstraintViolation(_)));
    for mailbox in [first_mailboxes[1], second_mailboxes[1]] {
        storage
            .synchronize_imap_mailbox(mailbox, &authorization)
            .await
            .unwrap();
        assert_eq!(
            storage
                .imap_mailbox_members(mailbox, 0)
                .await
                .unwrap()
                .len(),
            1
        );
    }
    assert_eq!(
        storage
            .indexed_messages(&[message.identifier])
            .await
            .unwrap()
            .len(),
        1
    );
}

/// A batch that encounters an existing destination rolls back earlier memberships and UID allocation.
#[tokio::test]
async fn duplicate_in_copy_batch_rolls_back_every_new_occurrence() {
    let storage = SqliteStorage::memory().await.unwrap();
    let owner = owner(&storage).await;
    let first = message("First");
    let second = message("Second");
    storage.insert_message(&first).await.unwrap();
    storage.insert_message(&second).await.unwrap();
    let account = account(&storage, owner, ImapAccountMode::Mirror).await;
    storage
        .create_web_folder(owner, "Projects", &[], 1)
        .await
        .unwrap();
    let [inbox, destination] = mailboxes(&storage, &account).await;
    let authorization = definition();
    storage
        .copy_imap_members(inbox, &[2], destination, Some(&authorization), false)
        .await
        .unwrap();
    let before = storage
        .get_imap_mailbox(destination)
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(
        storage
            .copy_imap_members(inbox, &[1, 2], destination, Some(&authorization), false)
            .await,
        Err(StorageError::ConstraintViolation(_))
    ));
    let after = storage
        .get_imap_mailbox(destination)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.uid_next, before.uid_next);
    storage
        .synchronize_imap_mailbox(destination, &authorization)
        .await
        .unwrap();
    let members = storage.imap_mailbox_members(destination, 0).await.unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].message_identifier, second.identifier);
    assert_eq!(
        storage.imap_mailbox_members(inbox, 0).await.unwrap().len(),
        2
    );
}

/// A stale authenticated CREATE cannot mutate the newly active mode even after switching away and back.
#[tokio::test]
async fn mode_generation_fences_stale_folder_creation_atomically() {
    let storage = SqliteStorage::memory().await.unwrap();
    let owner = owner(&storage).await;
    let account = account(&storage, owner, ImapAccountMode::Independent).await;
    for mode in [
        ImapAccountMode::Mirror,
        ImapAccountMode::Independent,
        ImapAccountMode::Mirror,
    ] {
        storage
            .update_imap_account(
                account.identifier,
                &ImapAccountSettings {
                    name: account.name.clone(),
                    enabled: true,
                    mirror_views: mode == ImapAccountMode::Mirror,
                    mode,
                },
            )
            .await
            .unwrap();
    }
    assert!(matches!(
        storage
            .create_imap_private_folder(
                account.identifier,
                "Stale CREATE",
                1,
                Some(account.mode_generation)
            )
            .await,
        Err(StorageError::NotFound)
    ));
    assert!(storage.list_web_folders(owner).await.unwrap().is_empty());
    assert!(
        storage
            .list_imap_local_folders(account.identifier)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Final-reference collection removes attachment bytes/facts and queues derived index deletion together.
#[tokio::test]
async fn garbage_collection_removes_related_metadata_and_queues_search_deletion() {
    let path = std::env::temp_dir().join(format!(
        "sandpost-imap-garbage-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let storage = SqliteStorage::open(&path).await.unwrap();
    let owner = owner(&storage).await;
    let mut message = message("Attachment retention");
    message.raw_message = b"Subject: Attachment retention\r\n\r\nattachment bytes".to_vec();
    message.facts.size = message.raw_message.len() as u64;
    message.facts.attachment_count = 1;
    message.facts.from.push(sandpost_core::Mailbox {
        address: "dev@example.test".into(),
        domain: "example.test".into(),
    });
    message
        .facts
        .headers
        .insert("subject".into(), vec![message.facts.subject.clone()]);
    message.attachments.push(sandpost_core::Attachment {
        filename: Some("data.txt".into()),
        content_type: "text/plain".into(),
        size: 16,
        content_hash: "0".repeat(64),
    });
    storage.insert_message(&message).await.unwrap();
    let account = account(&storage, owner, ImapAccountMode::Independent).await;
    storage
        .delete_user_messages(owner, Some(&[message.identifier]), &Expression::True)
        .await
        .unwrap();
    assert_eq!(
        storage
            .get_message(message.identifier)
            .await
            .unwrap()
            .unwrap()
            .raw_message,
        message.raw_message
    );
    let before = storage.search_status().await.unwrap();
    storage
        .delete_imap_account(account.identifier)
        .await
        .unwrap();
    assert!(
        storage
            .get_message(message.identifier)
            .await
            .unwrap()
            .is_none()
    );
    let after = storage.search_status().await.unwrap();
    assert!(after.latest_sequence > before.latest_sequence);
    let mut database = connection(&path).await;
    for table in [
        "mail",
        "mail_recipients",
        "mail_headers",
        "mail_attachments",
        "user_message_deletions",
        "imap_account_messages",
        "imap_mailbox_messages",
        "imap_message_flags",
        "user_message_flags",
    ] {
        let sql = format!("SELECT count(*) FROM {table}");
        let count: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
            .fetch_one(&mut database)
            .await
            .unwrap();
        assert_eq!(count, 0, "no orphaned rows in {table}");
    }
    let queued: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM search_outbox WHERE message_identifier=?1 AND sequence>?2",
    )
    .bind(message.identifier.to_string())
    .bind(i64::try_from(before.latest_sequence).unwrap())
    .fetch_one(&mut database)
    .await
    .unwrap();
    assert!(queued > 0);
    database.close().await.unwrap();
    storage.close().await;
    for file in [
        &path,
        &path.with_extension("sqlite-wal"),
        &path.with_extension("sqlite-shm"),
    ] {
        let _ = std::fs::remove_file(file);
    }
}

/// Legacy manual exclusions cannot hide matching retained messages from dynamic smart results.
#[tokio::test]
async fn legacy_smart_exclusions_do_not_override_query_membership() {
    use sandpost_storage::ViewStorage;
    for mode in [ImapAccountMode::Mirror, ImapAccountMode::Independent] {
        let path = std::env::temp_dir().join(format!(
            "sandpost-imap-exclusions-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let storage = SqliteStorage::open(&path).await.unwrap();
        let owner = owner(&storage).await;
        let account = account(&storage, owner, mode).await;
        let source = if mode == ImapAccountMode::Mirror {
            let identifier = sandpost_core::ViewIdentifier::new();
            storage
                .save_view(&sandpost_core::View {
                    identifier,
                    owner_identifier: Some(owner),
                    name: "Dynamic".into(),
                    filter: "true".into(),
                })
                .await
                .unwrap();
            ImapMailboxSource::View(identifier)
        } else {
            let folder = storage
                .create_imap_local_folder(&sandpost_storage::NewImapLocalFolder {
                    expected_mode_generation: None,
                    account_identifier: account.identifier,
                    name: "Dynamic".into(),
                    filter: "true".into(),
                    include_in_inbox: false,
                    created_at: 1,
                })
                .await
                .unwrap();
            ImapMailboxSource::LocalFolder(folder.identifier)
        };
        let mail = message("Always matches");
        let sequence = storage.insert_message(&mail).await.unwrap();
        let mailbox = storage
            .reconcile_imap_mailboxes(account.identifier, &[source], None)
            .await
            .unwrap()[0]
            .identifier;
        storage
            .synchronize_imap_mailbox(mailbox, &definition())
            .await
            .unwrap();
        let old_uid = storage.imap_mailbox_members(mailbox, 0).await.unwrap()[0].uid;
        let mut database = connection(&path).await;
        sqlx::query(
            "INSERT INTO imap_mailbox_exclusions(mailbox_identifier, mail_sequence) VALUES(?1,?2)",
        )
        .bind(mailbox.to_string())
        .bind(i64::try_from(sequence.0).unwrap())
        .execute(&mut database)
        .await
        .unwrap();
        sqlx::query("DELETE FROM imap_mailbox_messages WHERE mailbox_identifier=?1")
            .bind(mailbox.to_string())
            .execute(&mut database)
            .await
            .unwrap();
        database.close().await.unwrap();
        let refreshed = ImapMembershipDefinition {
            expected_mode_generation: None,
            filter: Expression::True,
            fingerprint: "refresh-legacy-smart".into(),
        };
        storage
            .synchronize_imap_mailbox(mailbox, &refreshed)
            .await
            .unwrap();
        let members = storage.imap_mailbox_members(mailbox, 0).await.unwrap();
        assert_eq!(members.len(), 1, "{mode:?} smart query is authoritative");
        assert_eq!(members[0].message_identifier, mail.identifier);
        assert!(members[0].uid > old_uid);
        storage.close().await;
        for file in [
            &path,
            &path.with_extension("sqlite-wal"),
            &path.with_extension("sqlite-shm"),
        ] {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// A plan loaded before folder creation does not erase an already selected authoritative mailbox.
#[tokio::test]
async fn stale_catalog_omission_preserves_live_folder_uids_and_pending_flags() {
    let storage = SqliteStorage::memory().await.unwrap();
    let owner = owner(&storage).await;
    let mail = message("Kept through a stale plan");
    storage.insert_message(&mail).await.unwrap();
    let account = account(&storage, owner, ImapAccountMode::Mirror).await;
    storage
        .create_web_folder(owner, "Projects", &[], 1)
        .await
        .unwrap();
    let [inbox, projects] = mailboxes(&storage, &account).await;
    storage
        .copy_imap_members(inbox, &[1], projects, Some(&definition()), false)
        .await
        .unwrap();
    storage
        .store_imap_flags(
            projects,
            &[1],
            &sandpost_storage::ImapFlagChange {
                operation: sandpost_storage::ImapFlagOperation::Add,
                flags: sandpost_core::ImapMessageFlags {
                    deleted: true,
                    ..Default::default()
                },
            },
            Some(account.mode_generation),
        )
        .await
        .unwrap();
    let before = storage.get_imap_mailbox(projects).await.unwrap().unwrap();
    storage
        .reconcile_imap_mailboxes(
            account.identifier,
            &[ImapMailboxSource::Inbox],
            Some(account.mode_generation),
        )
        .await
        .unwrap();
    let after = storage.get_imap_mailbox(projects).await.unwrap().unwrap();
    assert_eq!(after.uid_validity, before.uid_validity);
    assert_eq!(after.uid_next, before.uid_next);
    let members = storage.imap_mailbox_members(projects, 0).await.unwrap();
    assert_eq!(members.len(), 1);
    assert_eq!(members[0].uid, 1);
    assert!(members[0].flags.deleted);
}

/// Deleting the user owning the final Independent reference collects mail without disturbing peers.
#[tokio::test]
async fn user_deletion_collects_exclusively_retained_independent_content() {
    let storage = SqliteStorage::memory().await.unwrap();
    let owner = owner(&storage).await;
    let retainer = storage
        .create_user(&NewUser {
            name: "Retainer".into(),
            email: "retainer@example.test".into(),
            password_hash: "hash".into(),
            global_role: sandpost_core::GlobalRole::Member,
            mail_access: sandpost_core::MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    let mail = message("Retained only by a subordinate view");
    storage.insert_message(&mail).await.unwrap();
    let account = account(&storage, retainer.identifier, ImapAccountMode::Independent).await;
    for primary in [owner, retainer.identifier] {
        storage
            .delete_user_messages(primary, Some(&[mail.identifier]), &Expression::True)
            .await
            .unwrap();
    }
    assert!(
        storage
            .get_message(mail.identifier)
            .await
            .unwrap()
            .is_some()
    );
    storage.delete_user(retainer.identifier).await.unwrap();
    assert!(
        storage
            .get_imap_account(account.identifier)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .get_message(mail.identifier)
            .await
            .unwrap()
            .is_none()
    );
    assert!(storage.get_user(owner).await.unwrap().is_some());
}
