//! Captured session generations fence restored, dormant, and disabled view mutations.
#[allow(dead_code)]
mod common;

use sandpost_core::{
    GlobalRole, ImapAccount, ImapAccountMode, ImapMailboxIdentifier, ImapMailboxSource,
    ImapMessageFlags, MailAccess,
};
use sandpost_query::Expression;
use sandpost_storage::{
    ImapAccountSettings, ImapFlagChange, ImapFlagOperation, ImapLocalFolderSettings,
    ImapMembershipDefinition, ImapStorage, MessageStorage, NewImapAccount, NewImapLocalFolder,
    NewUser, StorageError, UserStorage,
};
use sandpost_storage_sqlite::SqliteStorage;
use sqlx::Connection;
use std::path::Path;

/// Define unrestricted membership with the generation captured by the caller.
fn definition(generation: u64) -> ImapMembershipDefinition {
    ImapMembershipDefinition {
        expected_mode_generation: Some(generation),
        filter: Expression::True,
        fingerprint: "generation-all".into(),
    }
}

/// Switch mode or enabled state through the production storage operation.
async fn configure(
    storage: &SqliteStorage,
    account: &ImapAccount,
    mode: ImapAccountMode,
    enabled: bool,
) -> ImapAccount {
    storage
        .update_imap_account(
            account.identifier,
            &ImapAccountSettings {
                name: account.name.clone(),
                enabled,
                mirror_views: mode == ImapAccountMode::Mirror,
                mode,
            },
        )
        .await
        .unwrap()
        .unwrap()
}

/// Capture all view organization and UID state so refused operations cannot silently change it.
async fn organization_snapshot(path: &Path) -> Vec<String> {
    let mut database = common::connection(path).await;
    let mut snapshot = Vec::new();
    for table in [
        "imap_accounts",
        "imap_local_folders",
        "imap_user_folders",
        "imap_user_folder_accounts",
        "web_folders",
        "web_folder_accounts",
        "imap_mailboxes",
        "imap_mailbox_messages",
        "imap_mailbox_exclusions",
        "imap_account_messages",
        "imap_message_placements",
        "imap_message_copies",
        "imap_message_flags",
        "user_message_placements",
        "user_message_copies",
        "user_message_trash",
        "user_message_deletions",
        "user_message_flags",
        "users",
        "search_outbox",
    ] {
        let columns: Vec<String> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT name FROM pragma_table_info('{table}') ORDER BY cid"
        )))
        .fetch_all(&mut database)
        .await
        .unwrap();
        let sql = format!(
            "SELECT coalesce(json_group_array(json_array({})), '[]') FROM (SELECT * FROM {table} ORDER BY rowid)",
            columns.join(",")
        );
        snapshot.push(
            sqlx::query_scalar(sqlx::AssertSqlSafe(sql))
                .fetch_one(&mut database)
                .await
                .unwrap(),
        );
    }
    database.close().await.unwrap();
    snapshot
}

/// Remove a closed file-backed fixture and its SQLite sidecars.
async fn close_fixture(storage: &SqliteStorage, path: &Path) {
    storage.close().await;
    for file in [
        path.to_path_buf(),
        path.with_extension("sqlite-wal"),
        path.with_extension("sqlite-shm"),
    ] {
        let _ = std::fs::remove_file(file);
    }
}

/// Old generations cannot modify any message, definition, identity, or UID after a mode round trip.
#[tokio::test]
async fn stale_generations_cannot_mutate_restored_views() {
    for mode in [ImapAccountMode::Mirror, ImapAccountMode::Independent] {
        let path = std::env::temp_dir().join(format!(
            "sandpost-generation-{}.sqlite",
            uuid::Uuid::new_v4()
        ));
        let storage = SqliteStorage::open(&path).await.unwrap();
        let owner = storage
            .create_first_owner(&NewUser {
                name: "Owner".into(),
                email: "generation@example.test".into(),
                password_hash: "hash".into(),
                global_role: GlobalRole::Owner,
                mail_access: MailAccess::All,
                personal_filter: None,
            })
            .await
            .unwrap();
        let message = common::message("Retained physical content");
        storage.insert_message(&message).await.unwrap();
        let account = storage
            .create_imap_account(&NewImapAccount {
                owner_identifier: owner.identifier,
                name: "Generation".into(),
                username: "generation".into(),
                password_hash: "hash".into(),
                mirror_views: mode == ImapAccountMode::Mirror,
                mode,
                initial_authorization: Expression::True,
                created_at: 1,
            })
            .await
            .unwrap();
        let captured = Some(account.mode_generation);
        let folder = storage
            .create_imap_private_folder(account.identifier, "Projects", 1, captured)
            .await
            .unwrap();
        let smart = storage
            .create_imap_local_folder(&NewImapLocalFolder {
                expected_mode_generation: captured,
                account_identifier: account.identifier,
                name: "Dynamic".into(),
                filter: "true".into(),
                include_in_inbox: false,
                created_at: 1,
            })
            .await
            .unwrap();
        let sources = [
            ImapMailboxSource::Inbox,
            ImapMailboxSource::Trash,
            ImapMailboxSource::LocalFolder(folder.identifier),
            ImapMailboxSource::LocalFolder(smart.identifier),
        ];
        let states = storage
            .reconcile_imap_mailboxes(account.identifier, &sources, captured)
            .await
            .unwrap();
        let original_definition = definition(account.mode_generation);
        for state in &states {
            storage
                .synchronize_imap_mailbox(state.identifier, &original_definition)
                .await
                .unwrap();
        }
        let inbox = states[0].identifier;
        let destination = states[2].identifier;
        let deleted = ImapFlagChange {
            operation: ImapFlagOperation::Add,
            flags: ImapMessageFlags {
                deleted: true,
                ..ImapMessageFlags::default()
            },
        };
        storage
            .store_imap_flags(inbox, &[1], &deleted, captured)
            .await
            .unwrap();
        let away = if mode == ImapAccountMode::Mirror {
            ImapAccountMode::Independent
        } else {
            ImapAccountMode::Mirror
        };
        configure(&storage, &account, away, true).await;
        let restored = configure(&storage, &account, mode, true).await;
        assert!(restored.mode_generation > account.mode_generation);
        let before = organization_snapshot(&path).await;

        let seen = ImapFlagChange {
            operation: ImapFlagOperation::Add,
            flags: ImapMessageFlags {
                seen: true,
                ..ImapMessageFlags::default()
            },
        };
        assert!(matches!(
            storage.store_imap_flags(inbox, &[1], &seen, captured).await,
            Err(StorageError::NotFound)
        ));
        for remove_source in [false, true] {
            assert!(matches!(
                storage
                    .copy_imap_members(
                        inbox,
                        &[1],
                        destination,
                        Some(&original_definition),
                        remove_source
                    )
                    .await,
                Err(StorageError::NotFound)
            ));
        }
        assert!(matches!(
            storage
                .expunge_imap_members(inbox, Some(&[1]), captured)
                .await,
            Err(StorageError::NotFound)
        ));
        assert!(matches!(
            storage
                .reconcile_imap_mailboxes(account.identifier, &sources, captured)
                .await,
            Err(StorageError::NotFound)
        ));
        assert!(matches!(
            storage
                .synchronize_imap_mailbox(inbox, &original_definition)
                .await,
            Err(StorageError::NotFound)
        ));
        let changed_definition = ImapMembershipDefinition {
            fingerprint: "force-writer".into(),
            ..original_definition.clone()
        };
        assert!(matches!(
            storage
                .synchronize_imap_mailbox(inbox, &changed_definition)
                .await,
            Err(StorageError::NotFound)
        ));
        assert!(matches!(
            storage
                .create_imap_private_folder(account.identifier, "Stale normal", 2, captured)
                .await,
            Err(StorageError::NotFound)
        ));
        assert!(matches!(
            storage
                .create_imap_local_folder(&NewImapLocalFolder {
                    expected_mode_generation: captured,
                    account_identifier: account.identifier,
                    name: "Stale smart".into(),
                    filter: "true".into(),
                    include_in_inbox: false,
                    created_at: 2,
                })
                .await,
            Err(StorageError::NotFound)
        ));
        for folder in [folder.identifier, smart.identifier] {
            assert!(matches!(
                storage
                    .update_imap_local_folder(
                        folder,
                        &ImapLocalFolderSettings {
                            expected_mode_generation: captured,
                            name: "Stale rename".into(),
                            filter: "false".into(),
                            include_in_inbox: true,
                        }
                    )
                    .await,
                Err(StorageError::NotFound)
            ));
            assert!(matches!(
                storage.delete_imap_local_folder(folder, captured).await,
                Err(StorageError::NotFound)
            ));
        }
        assert_eq!(
            organization_snapshot(&path).await,
            before,
            "every refused mutation must leave {mode:?} state unchanged"
        );
        assert_eq!(
            storage
                .get_message(message.identifier)
                .await
                .unwrap()
                .unwrap()
                .raw_message,
            message.raw_message
        );

        // A freshly authenticated epoch can still use the preserved mailbox and update flags.
        storage
            .synchronize_imap_mailbox(inbox, &definition(restored.mode_generation))
            .await
            .unwrap();
        let updated = storage
            .store_imap_flags(inbox, &[1], &seen, Some(restored.mode_generation))
            .await
            .unwrap();
        assert!(updated[0].1.seen);
        close_fixture(&storage, &path).await;
    }
}

/// Matching generations do not authorize dormant handles or disabled credentials.
#[tokio::test]
async fn current_generations_still_reject_inactive_or_disabled_views() {
    for mode in [ImapAccountMode::Mirror, ImapAccountMode::Independent] {
        let storage = SqliteStorage::memory().await.unwrap();
        let owner = storage
            .create_first_owner(&NewUser {
                name: "Owner".into(),
                email: "disabled@example.test".into(),
                password_hash: "hash".into(),
                global_role: GlobalRole::Owner,
                mail_access: MailAccess::All,
                personal_filter: None,
            })
            .await
            .unwrap();
        storage
            .insert_message(&common::message("Hidden handle"))
            .await
            .unwrap();
        let account = storage
            .create_imap_account(&NewImapAccount {
                owner_identifier: owner.identifier,
                name: "Generation".into(),
                username: "generation".into(),
                password_hash: "hash".into(),
                mirror_views: mode == ImapAccountMode::Mirror,
                mode,
                initial_authorization: Expression::True,
                created_at: 1,
            })
            .await
            .unwrap();
        let states = storage
            .reconcile_imap_mailboxes(
                account.identifier,
                &[ImapMailboxSource::Inbox, ImapMailboxSource::Trash],
                Some(account.mode_generation),
            )
            .await
            .unwrap();
        let inbox = states[0].identifier;
        storage
            .synchronize_imap_mailbox(inbox, &definition(account.mode_generation))
            .await
            .unwrap();
        let away = if mode == ImapAccountMode::Mirror {
            ImapAccountMode::Independent
        } else {
            ImapAccountMode::Mirror
        };
        let inactive = configure(&storage, &account, away, true).await;
        assert!(matches!(
            storage
                .synchronize_imap_mailbox(inbox, &definition(inactive.mode_generation))
                .await,
            Err(StorageError::NotFound)
        ));
        let unfenced = ImapMembershipDefinition {
            expected_mode_generation: None,
            ..definition(inactive.mode_generation)
        };
        assert!(matches!(
            storage.synchronize_imap_mailbox(inbox, &unfenced).await,
            Err(StorageError::NotFound)
        ));
        configure(&storage, &account, mode, true).await;
        let disabled = configure(&storage, &account, mode, false).await;
        let generation = Some(disabled.mode_generation);
        let seen = ImapFlagChange {
            operation: ImapFlagOperation::Add,
            flags: ImapMessageFlags {
                seen: true,
                ..ImapMessageFlags::default()
            },
        };
        assert!(matches!(
            storage
                .reconcile_imap_mailboxes(
                    account.identifier,
                    &[ImapMailboxSource::Inbox],
                    generation
                )
                .await,
            Err(StorageError::NotFound)
        ));
        assert!(matches!(
            storage
                .synchronize_imap_mailbox(inbox, &definition(disabled.mode_generation))
                .await,
            Err(StorageError::NotFound)
        ));
        assert!(matches!(
            storage
                .store_imap_flags(inbox, &[1], &seen, generation)
                .await,
            Err(StorageError::NotFound)
        ));
        assert!(matches!(
            storage.expunge_imap_members(inbox, None, generation).await,
            Err(StorageError::NotFound)
        ));
        assert!(matches!(
            storage
                .copy_imap_members(
                    inbox,
                    &[1],
                    states[1].identifier,
                    Some(&definition(disabled.mode_generation)),
                    false
                )
                .await,
            Err(StorageError::NotFound)
        ));
        assert!(matches!(
            storage
                .create_imap_private_folder(account.identifier, "Disabled CREATE", 2, generation)
                .await,
            Err(StorageError::NotFound)
        ));
        storage.close().await;
    }
}

/// Trusted unfenced callers retain missing-mailbox no-op results while fenced calls fail closed.
#[tokio::test]
async fn missing_mailbox_results_preserve_unfenced_contract() {
    let storage = SqliteStorage::memory().await.unwrap();
    let missing = ImapMailboxIdentifier::new();
    let unfenced = ImapMembershipDefinition {
        expected_mode_generation: None,
        ..definition(0)
    };
    let seen = ImapFlagChange {
        operation: ImapFlagOperation::Add,
        flags: ImapMessageFlags {
            seen: true,
            ..ImapMessageFlags::default()
        },
    };
    assert!(
        storage
            .synchronize_imap_mailbox(missing, &unfenced)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .store_imap_flags(missing, &[1], &seen, None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        storage
            .expunge_imap_members(missing, None, None)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(matches!(
        storage
            .synchronize_imap_mailbox(missing, &definition(0))
            .await,
        Err(StorageError::NotFound)
    ));
    assert!(matches!(
        storage
            .store_imap_flags(missing, &[1], &seen, Some(0))
            .await,
        Err(StorageError::NotFound)
    ));
    assert!(matches!(
        storage.expunge_imap_members(missing, None, Some(0)).await,
        Err(StorageError::NotFound)
    ));
    storage.close().await;
}
