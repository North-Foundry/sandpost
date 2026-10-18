//! Mirror sharing and independent retention, organization, identity, and collection regressions.
#[allow(dead_code)]
mod common;

use sandpost_core::{
    GlobalRole, ImapAccount, ImapAccountMode, ImapFolderIdentifier, ImapFolderKind,
    ImapMailboxIdentifier, ImapMailboxSource, ImapMessageFlags, MailAccess, MessageIdentifier,
    UserIdentifier,
};
use sandpost_query::Expression;
use sandpost_storage::{
    ImapAccountSettings, ImapFlagChange, ImapFlagOperation, ImapLocalFolderSettings,
    ImapMailboxState, ImapMember, ImapMembershipDefinition, ImapStorage, MessageListQuery,
    MessageStorage, NewImapAccount, NewImapLocalFolder, NewUser, StorageError, UserStorage,
};
use sandpost_storage_sqlite::SqliteStorage;

/// Create the only primary user so collection tests have an explicit retention boundary.
async fn create_owner(storage: &SqliteStorage) -> UserIdentifier {
    storage
        .create_first_owner(&NewUser {
            name: "Owner".into(),
            email: "owner@example.test".into(),
            password_hash: "hash".into(),
            global_role: GlobalRole::Owner,
            mail_access: MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap()
        .identifier
}

/// Create an account with an explicit initial history authorization snapshot.
async fn create_account(
    storage: &SqliteStorage,
    owner: UserIdentifier,
    mode: ImapAccountMode,
    authorization: &str,
) -> ImapAccount {
    storage
        .create_imap_account(&NewImapAccount {
            owner_identifier: owner,
            name: format!("{mode:?}"),
            username: uuid::Uuid::new_v4().to_string(),
            password_hash: "hash".into(),
            mirror_views: false,
            mode,
            initial_authorization: definition(authorization).filter,
            created_at: 1,
        })
        .await
        .unwrap()
}

/// Switch only the organization mode, preserving the account's other settings.
async fn switch_mode(
    storage: &SqliteStorage,
    account: &ImapAccount,
    mode: ImapAccountMode,
) -> ImapAccount {
    storage
        .update_imap_account(
            account.identifier,
            &ImapAccountSettings {
                name: account.name.clone(),
                enabled: account.enabled,
                mirror_views: account.mirror_views,
                mode,
            },
        )
        .await
        .unwrap()
        .unwrap()
}

/// Build a canonical membership definition for an authorization or smart result.
fn definition(source: &str) -> ImapMembershipDefinition {
    let (filter, fingerprint) = sandpost_query::compile(source)
        .unwrap()
        .into_expression()
        .into_canonical_with_fingerprint();
    ImapMembershipDefinition {
        expected_mode_generation: None,
        filter,
        fingerprint,
    }
}

/// Insert a message whose exact bytes can be read through retained independent membership.
async fn insert_message(storage: &SqliteStorage, subject: &str) -> MessageIdentifier {
    let mut message = common::message(subject);
    message.raw_message = format!("Subject: {subject}\r\n\r\nretained body\r\n").into_bytes();
    message.facts.size = message.raw_message.len() as u64;
    let identifier = message.identifier;
    storage.insert_message(&message).await.unwrap();
    identifier
}

/// Reconcile every currently visible folder plus the account's fixed mailbox sources.
async fn reconcile(storage: &SqliteStorage, account: &ImapAccount) -> Vec<ImapMailboxState> {
    let mut sources = vec![ImapMailboxSource::Inbox, ImapMailboxSource::Trash];
    sources.extend(
        storage
            .list_imap_local_folders(account.identifier)
            .await
            .unwrap()
            .into_iter()
            .map(|folder| ImapMailboxSource::LocalFolder(folder.identifier)),
    );
    storage
        .reconcile_imap_mailboxes(account.identifier, &sources, None)
        .await
        .unwrap()
}

/// Select one reconciled source without changing the other mailbox identities.
fn mailbox(states: &[ImapMailboxState], source: ImapMailboxSource) -> ImapMailboxState {
    *states.iter().find(|state| state.source == source).unwrap()
}

/// Synchronize a mailbox against current authorization and return its materialized members.
async fn synchronize(
    storage: &SqliteStorage,
    mailbox: ImapMailboxIdentifier,
    filter: &str,
) -> Vec<ImapMember> {
    storage
        .synchronize_imap_mailbox(mailbox, &definition(filter))
        .await
        .unwrap()
        .unwrap();
    storage.imap_mailbox_members(mailbox, 0).await.unwrap()
}

/// Return one message's current UID, failing when the expected occurrence is missing.
async fn member_uid(
    storage: &SqliteStorage,
    mailbox: ImapMailboxIdentifier,
    message: MessageIdentifier,
) -> u32 {
    storage
        .imap_mailbox_members(mailbox, 0)
        .await
        .unwrap()
        .into_iter()
        .find(|member| member.message_identifier == message)
        .unwrap()
        .uid
}

/// Remove exactly one occurrence through the public STORE and EXPUNGE contract.
async fn expunge_message(
    storage: &SqliteStorage,
    mailbox: ImapMailboxIdentifier,
    message: MessageIdentifier,
) {
    let uid = member_uid(storage, mailbox, message).await;
    storage
        .store_imap_flags(
            mailbox,
            &[uid],
            &ImapFlagChange {
                operation: ImapFlagOperation::Add,
                flags: ImapMessageFlags {
                    deleted: true,
                    ..ImapMessageFlags::default()
                },
            },
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        storage
            .expunge_imap_members(mailbox, Some(&[uid]), None)
            .await
            .unwrap(),
        [uid]
    );
}

/// Return visible primary message identifiers from one normal or Trash mailbox.
async fn primary_messages(
    storage: &SqliteStorage,
    owner: UserIdentifier,
    folder: Option<ImapFolderIdentifier>,
    trash: bool,
) -> Vec<MessageIdentifier> {
    let query = MessageListQuery {
        filter: Some(Expression::True),
        before: None,
        limit: 100,
    };
    let messages = if trash {
        storage.list_web_trash_messages(owner, query).await.unwrap()
    } else {
        storage
            .list_web_mailbox_messages(owner, folder, query)
            .await
            .unwrap()
    };
    messages
        .into_iter()
        .map(|message| message.identifier)
        .collect()
}

/// Find a primary Web folder's Mirror projection or a same-named independent snapshot.
async fn folder_identifier(
    storage: &SqliteStorage,
    account: &ImapAccount,
    name: &str,
) -> ImapFolderIdentifier {
    storage
        .list_imap_local_folders(account.identifier)
        .await
        .unwrap()
        .into_iter()
        .find(|folder| folder.name == name)
        .unwrap()
        .identifier
}

/// Seen, filing, and two-stage deletion share primary Mirror state while Independent accounts retain their own state.
#[tokio::test]
async fn mirror_sharing_and_independent_isolation() {
    let storage = SqliteStorage::memory().await.unwrap();
    let owner = create_owner(&storage).await;
    let message = insert_message(&storage, "shared history").await;
    let first = create_account(&storage, owner, ImapAccountMode::Mirror, "true").await;
    let second = create_account(&storage, owner, ImapAccountMode::Mirror, "true").await;
    let first_states = reconcile(&storage, &first).await;
    let second_states = reconcile(&storage, &second).await;
    let first_inbox = mailbox(&first_states, ImapMailboxSource::Inbox).identifier;
    let second_inbox = mailbox(&second_states, ImapMailboxSource::Inbox).identifier;
    let first_trash = mailbox(&first_states, ImapMailboxSource::Trash).identifier;
    let second_trash = mailbox(&second_states, ImapMailboxSource::Trash).identifier;
    synchronize(&storage, first_inbox, "true").await;
    synchronize(&storage, second_inbox, "true").await;
    storage
        .store_imap_flags(
            first_inbox,
            &[member_uid(&storage, first_inbox, message).await],
            &ImapFlagChange {
                operation: ImapFlagOperation::Add,
                flags: ImapMessageFlags {
                    seen: true,
                    flagged: true,
                    keywords: vec!["$Primary".into()],
                    ..ImapMessageFlags::default()
                },
            },
            None,
        )
        .await
        .unwrap();
    assert!(
        storage.imap_mailbox_members(second_inbox, 0).await.unwrap()[0]
            .flags
            .seen
    );
    let primary = storage
        .list_user_messages(
            owner,
            MessageListQuery {
                filter: Some(Expression::True),
                before: None,
                limit: 10,
            },
        )
        .await
        .unwrap();
    assert!(primary[0].seen);

    let independent = create_account(&storage, owner, ImapAccountMode::Independent, "true").await;
    let sibling = create_account(&storage, owner, ImapAccountMode::Independent, "true").await;
    let independent_states = reconcile(&storage, &independent).await;
    let sibling_states = reconcile(&storage, &sibling).await;
    let independent_inbox = mailbox(&independent_states, ImapMailboxSource::Inbox).identifier;
    let sibling_inbox = mailbox(&sibling_states, ImapMailboxSource::Inbox).identifier;
    let snapshot_flags = synchronize(&storage, independent_inbox, "true").await[0]
        .flags
        .clone();
    assert!(snapshot_flags.seen && snapshot_flags.flagged);
    assert_eq!(snapshot_flags.keywords, ["$Primary"]);
    synchronize(&storage, sibling_inbox, "true").await;
    storage
        .store_imap_flags(
            independent_inbox,
            &[member_uid(&storage, independent_inbox, message).await],
            &ImapFlagChange {
                operation: ImapFlagOperation::Remove,
                flags: ImapMessageFlags {
                    seen: true,
                    ..ImapMessageFlags::default()
                },
            },
            None,
        )
        .await
        .unwrap();
    assert!(
        !storage
            .imap_mailbox_members(independent_inbox, 0)
            .await
            .unwrap()[0]
            .flags
            .seen
    );
    assert!(
        storage
            .imap_mailbox_members(sibling_inbox, 0)
            .await
            .unwrap()[0]
            .flags
            .seen
    );
    assert!(
        storage.imap_mailbox_members(first_inbox, 0).await.unwrap()[0]
            .flags
            .seen
    );

    expunge_message(&storage, first_inbox, message).await;
    assert_eq!(
        primary_messages(&storage, owner, None, true).await,
        [message]
    );
    assert!(synchronize(&storage, second_inbox, "true").await.is_empty());
    assert_eq!(
        synchronize(&storage, second_trash, "true").await[0].message_identifier,
        message
    );
    assert_eq!(
        synchronize(&storage, independent_inbox, "true").await[0].message_identifier,
        message
    );
    assert_eq!(
        synchronize(&storage, sibling_inbox, "true").await[0].message_identifier,
        message
    );
    synchronize(&storage, first_trash, "true").await;
    expunge_message(&storage, first_trash, message).await;
    assert!(
        storage
            .get_user_message(owner, message)
            .await
            .unwrap()
            .is_none()
    );
    assert!(synchronize(&storage, second_trash, "true").await.is_empty());
    assert_eq!(
        synchronize(&storage, independent_inbox, "true").await[0].message_identifier,
        message
    );
    let content = storage
        .imap_member_content(
            independent_inbox,
            member_uid(&storage, independent_inbox, message).await,
        )
        .await
        .unwrap()
        .unwrap();
    assert!(content.raw_message.ends_with(b"retained body\r\n"));
    expunge_message(&storage, independent_inbox, message).await;
    assert_eq!(synchronize(&storage, sibling_inbox, "true").await.len(), 1);
    assert!(storage.get_message(message).await.unwrap().is_some());
}

/// Initial history honors authorization and primary tombstones, but snapshots every folder, placements, copies, Trash, and flags.
#[tokio::test]
async fn initial_history_and_primary_organization_are_snapshotted_once() {
    let storage = SqliteStorage::memory().await.unwrap();
    let owner = create_owner(&storage).await;
    let filed = insert_message(&storage, "allowed filed").await;
    let copied = insert_message(&storage, "allowed copied").await;
    let trashed = insert_message(&storage, "allowed trashed").await;
    let hidden = insert_message(&storage, "allowed hidden").await;
    let denied = insert_message(&storage, "denied history").await;
    let mirror = create_account(&storage, owner, ImapAccountMode::Mirror, "true").await;
    let filed_definition = storage
        .create_web_folder(owner, "Filed", None, &[], 1)
        .await
        .unwrap();
    storage
        .create_web_folder(owner, "Copies", None, &[], 1)
        .await
        .unwrap();
    storage
        .create_web_folder(owner, "Empty", None, &[], 1)
        .await
        .unwrap();
    storage
        .create_imap_local_folder(&NewImapLocalFolder {
            expected_mode_generation: None,
            account_identifier: mirror.identifier,
            name: "Legacy smart".into(),
            filter: "true".into(),
            include_in_inbox: true,
            created_at: 1,
        })
        .await
        .unwrap();
    storage
        .place_user_messages(
            owner,
            &[filed],
            Some(filed_definition.identifier),
            &Expression::True,
        )
        .await
        .unwrap();
    storage
        .trash_user_messages(owner, &[trashed], &Expression::True, 1)
        .await
        .unwrap();
    storage
        .delete_user_messages(owner, Some(&[hidden]), &Expression::True)
        .await
        .unwrap();
    let mirror_states = reconcile(&storage, &mirror).await;
    let inbox = mailbox(&mirror_states, ImapMailboxSource::Inbox).identifier;
    let copies_folder = folder_identifier(&storage, &mirror, "Copies").await;
    let copies_mailbox = mailbox(
        &mirror_states,
        ImapMailboxSource::LocalFolder(copies_folder),
    )
    .identifier;
    synchronize(&storage, inbox, "true").await;
    synchronize(&storage, copies_mailbox, "true").await;
    storage
        .copy_imap_members(
            inbox,
            &[member_uid(&storage, inbox, copied).await],
            copies_mailbox,
            Some(&definition("true")),
            false,
        )
        .await
        .unwrap();
    let independent = create_account(
        &storage,
        owner,
        ImapAccountMode::Independent,
        "subject contains \"allowed\"",
    )
    .await;
    let snapshots = storage
        .list_imap_local_folders(independent.identifier)
        .await
        .unwrap();
    assert_eq!(snapshots.len(), 3);
    assert!(
        snapshots
            .iter()
            .all(|folder| folder.kind == ImapFolderKind::Private
                && folder.definition_identifier.is_none())
    );
    assert!(snapshots.iter().any(|folder| folder.name == "Empty"));
    let states = reconcile(&storage, &independent).await;
    let independent_inbox = mailbox(&states, ImapMailboxSource::Inbox).identifier;
    assert_eq!(
        synchronize(&storage, independent_inbox, "true")
            .await
            .iter()
            .map(|member| member.message_identifier)
            .collect::<Vec<_>>(),
        [copied]
    );
    for (name, expected) in [("Filed", filed), ("Copies", copied)] {
        let folder = folder_identifier(&storage, &independent, name).await;
        assert_eq!(
            synchronize(
                &storage,
                mailbox(&states, ImapMailboxSource::LocalFolder(folder)).identifier,
                "true"
            )
            .await[0]
                .message_identifier,
            expected
        );
    }
    assert_eq!(
        synchronize(
            &storage,
            mailbox(&states, ImapMailboxSource::Trash).identifier,
            "true"
        )
        .await[0]
            .message_identifier,
        trashed
    );
    for state in &states {
        assert!(
            storage
                .imap_mailbox_members(state.identifier, 0)
                .await
                .unwrap()
                .iter()
                .all(|member| member.message_identifier != hidden
                    && member.message_identifier != denied)
        );
    }
    storage
        .update_web_folder(filed_definition.identifier, "Primary renamed", None, &[])
        .await
        .unwrap();
    assert!(
        storage
            .list_imap_local_folders(independent.identifier)
            .await
            .unwrap()
            .iter()
            .any(|folder| folder.name == "Filed")
    );
    let future = insert_message(&storage, "future arrival outside initial authorization").await;
    storage
        .delete_user_messages(owner, Some(&[future]), &Expression::True)
        .await
        .unwrap();
    let members = synchronize(&storage, independent_inbox, "true").await;
    assert!(
        members
            .iter()
            .any(|member| member.message_identifier == future)
    );
    assert!(
        synchronize(&storage, independent_inbox, "false")
            .await
            .is_empty()
    );
    let members = synchronize(&storage, independent_inbox, "true").await;
    assert!(
        members
            .iter()
            .any(|member| member.message_identifier == future)
    );
}

/// Dormant independent state retains arrivals and resumes its prior mailbox identities, flags, and filing after reopen.
#[tokio::test]
async fn mode_switches_preserve_dormant_arrivals_and_identity_across_reopen() {
    let path = std::env::temp_dir().join(format!(
        "sandpost-imap-modes-{}.sqlite",
        uuid::Uuid::new_v4()
    ));
    let storage = SqliteStorage::open(&path).await.unwrap();
    let owner = create_owner(&storage).await;
    let original = insert_message(&storage, "original").await;
    let mut account = create_account(&storage, owner, ImapAccountMode::Independent, "true").await;
    let private = storage
        .create_imap_private_folder(account.identifier, "Private", 1, None)
        .await
        .unwrap();
    let states = reconcile(&storage, &account).await;
    let independent_inbox = mailbox(&states, ImapMailboxSource::Inbox);
    let independent_folder = mailbox(&states, ImapMailboxSource::LocalFolder(private.identifier));
    synchronize(&storage, independent_inbox.identifier, "true").await;
    synchronize(&storage, independent_folder.identifier, "true").await;
    let moved = storage
        .copy_imap_members(
            independent_inbox.identifier,
            &[member_uid(&storage, independent_inbox.identifier, original).await],
            independent_folder.identifier,
            Some(&definition("true")),
            true,
        )
        .await
        .unwrap();
    storage
        .store_imap_flags(
            independent_folder.identifier,
            &moved.destination_uids,
            &ImapFlagChange {
                operation: ImapFlagOperation::Add,
                flags: ImapMessageFlags {
                    seen: true,
                    answered: true,
                    keywords: vec!["$Independent".into()],
                    ..ImapMessageFlags::default()
                },
            },
            None,
        )
        .await
        .unwrap();
    let prior_generation = account.mode_generation;
    account = switch_mode(&storage, &account, ImapAccountMode::Mirror).await;
    assert_eq!(account.mode_generation, prior_generation + 1);
    assert!(matches!(
        storage
            .store_imap_flags(
                independent_folder.identifier,
                &[1],
                &ImapFlagChange {
                    operation: ImapFlagOperation::Add,
                    flags: ImapMessageFlags::default()
                },
                None
            )
            .await,
        Err(StorageError::NotFound)
    ));
    let mirror_states = reconcile(&storage, &account).await;
    let mirror_inbox = mailbox(&mirror_states, ImapMailboxSource::Inbox);
    assert_ne!(mirror_inbox.identifier, independent_inbox.identifier);
    assert_ne!(mirror_inbox.uid_validity, independent_inbox.uid_validity);
    assert!(
        !synchronize(&storage, mirror_inbox.identifier, "true").await[0]
            .flags
            .seen
    );
    let new_primary = storage
        .create_web_folder(owner, "Added while dormant", None, &[], 2)
        .await
        .unwrap();
    let dormant_arrival = insert_message(&storage, "arrival while exposing Mirror").await;
    storage
        .delete_user_messages(owner, Some(&[original, dormant_arrival]), &Expression::True)
        .await
        .unwrap();
    assert!(
        synchronize(&storage, mirror_inbox.identifier, "true")
            .await
            .is_empty()
    );
    storage.close().await;

    let storage = SqliteStorage::open(&path).await.unwrap();
    account = storage
        .get_imap_account(account.identifier)
        .await
        .unwrap()
        .unwrap();
    account = switch_mode(&storage, &account, ImapAccountMode::Independent).await;
    let resumed = reconcile(&storage, &account).await;
    assert_eq!(
        mailbox(&resumed, ImapMailboxSource::Inbox).identifier,
        independent_inbox.identifier
    );
    assert_eq!(
        mailbox(&resumed, ImapMailboxSource::Inbox).uid_validity,
        independent_inbox.uid_validity
    );
    assert_eq!(
        mailbox(&resumed, ImapMailboxSource::LocalFolder(private.identifier)).identifier,
        independent_folder.identifier
    );
    assert_eq!(
        mailbox(&resumed, ImapMailboxSource::LocalFolder(private.identifier)).uid_validity,
        independent_folder.uid_validity
    );
    let filed = synchronize(&storage, independent_folder.identifier, "true").await;
    assert_eq!(filed[0].uid, moved.destination_uids[0]);
    assert!(filed[0].flags.seen && filed[0].flags.answered);
    assert_eq!(filed[0].flags.keywords, ["$Independent"]);
    let arrivals = synchronize(&storage, independent_inbox.identifier, "true").await;
    assert_eq!(arrivals.len(), 1);
    assert_eq!(arrivals[0].message_identifier, dormant_arrival);
    assert!(arrivals[0].uid > 1);
    assert!(
        storage
            .list_imap_local_folders(account.identifier)
            .await
            .unwrap()
            .iter()
            .all(|folder| folder.name != "Added while dormant")
    );
    account = switch_mode(&storage, &account, ImapAccountMode::Mirror).await;
    let mirror_again = reconcile(&storage, &account).await;
    assert_eq!(
        mailbox(&mirror_again, ImapMailboxSource::Inbox).identifier,
        mirror_inbox.identifier
    );
    let projection_count = storage
        .list_imap_local_folders(account.identifier)
        .await
        .unwrap()
        .len();
    let projection = folder_identifier(&storage, &account, "Added while dormant").await;
    let generation = account.mode_generation;
    account = switch_mode(&storage, &account, ImapAccountMode::Mirror).await;
    assert_eq!(account.mode_generation, generation);
    assert_eq!(
        storage
            .list_imap_local_folders(account.identifier)
            .await
            .unwrap()
            .len(),
        projection_count
    );
    assert_eq!(
        folder_identifier(&storage, &account, "Added while dormant").await,
        projection
    );
    assert!(
        storage
            .get_web_folder(new_primary.identifier)
            .await
            .unwrap()
            .is_some()
    );
    storage.close().await;
    for path in [
        &path,
        &path.with_extension("sqlite-wal"),
        &path.with_extension("sqlite-shm"),
    ] {
        let _ = std::fs::remove_file(path);
    }
}

/// Physical collection waits for every independent reference, including unauthorized and dormant state.
#[tokio::test]
async fn garbage_collection_waits_for_all_retention_references() {
    let storage = SqliteStorage::memory().await.unwrap();
    let owner = create_owner(&storage).await;
    let message = insert_message(&storage, "last retained content").await;
    let first = create_account(&storage, owner, ImapAccountMode::Independent, "true").await;
    let mut last = create_account(&storage, owner, ImapAccountMode::Independent, "true").await;
    let first_states = reconcile(&storage, &first).await;
    let inbox = mailbox(&first_states, ImapMailboxSource::Inbox).identifier;
    let trash = mailbox(&first_states, ImapMailboxSource::Trash).identifier;
    synchronize(&storage, inbox, "true").await;
    let last_states = reconcile(&storage, &last).await;
    let last_inbox = mailbox(&last_states, ImapMailboxSource::Inbox).identifier;
    assert!(synchronize(&storage, last_inbox, "false").await.is_empty());
    last = switch_mode(&storage, &last, ImapAccountMode::Mirror).await;
    storage
        .delete_user_messages(owner, Some(&[message]), &Expression::True)
        .await
        .unwrap();
    expunge_message(&storage, inbox, message).await;
    synchronize(&storage, trash, "true").await;
    expunge_message(&storage, trash, message).await;
    assert!(synchronize(&storage, inbox, "true").await.is_empty());
    assert!(storage.get_message(message).await.unwrap().is_some());
    assert_eq!(storage.indexed_messages(&[message]).await.unwrap().len(), 1);
    storage.delete_imap_account(first.identifier).await.unwrap();
    assert!(storage.get_message(message).await.unwrap().is_some());
    last = switch_mode(&storage, &last, ImapAccountMode::Independent).await;
    assert_eq!(
        synchronize(
            &storage,
            mailbox(&reconcile(&storage, &last).await, ImapMailboxSource::Inbox).identifier,
            "true"
        )
        .await[0]
            .message_identifier,
        message
    );
    storage.delete_imap_account(last.identifier).await.unwrap();
    assert!(storage.get_message(message).await.unwrap().is_none());
    assert!(
        storage
            .indexed_messages(&[message])
            .await
            .unwrap()
            .is_empty()
    );
    assert_eq!(storage.max_message_sequence().await.unwrap().0, 1);
}

/// A surviving primary user keeps physical mail after an independent account permanently releases it.
#[tokio::test]
async fn primary_retention_survives_independent_permanent_deletion() {
    let storage = SqliteStorage::memory().await.unwrap();
    let owner = create_owner(&storage).await;
    let message = insert_message(&storage, "primary retained").await;
    let account = create_account(&storage, owner, ImapAccountMode::Independent, "true").await;
    let states = reconcile(&storage, &account).await;
    let inbox = mailbox(&states, ImapMailboxSource::Inbox).identifier;
    let trash = mailbox(&states, ImapMailboxSource::Trash).identifier;
    synchronize(&storage, inbox, "true").await;
    expunge_message(&storage, inbox, message).await;
    synchronize(&storage, trash, "true").await;
    expunge_message(&storage, trash, message).await;
    assert!(synchronize(&storage, inbox, "true").await.is_empty());
    assert!(
        storage
            .get_user_message(owner, message)
            .await
            .unwrap()
            .is_some()
    );
    assert!(storage.get_message(message).await.unwrap().is_some());
    storage
        .delete_user_messages(owner, Some(&[message]), &Expression::True)
        .await
        .unwrap();
    assert!(storage.get_message(message).await.unwrap().is_none());
}

/// Independent smart results follow retained mail and authorization without altering normal folder membership.
#[tokio::test]
async fn smart_results_follow_each_organization_and_do_not_grant_retention() {
    let storage = SqliteStorage::memory().await.unwrap();
    let owner = create_owner(&storage).await;
    let account = create_account(&storage, owner, ImapAccountMode::Independent, "true").await;
    let mirror = create_account(&storage, owner, ImapAccountMode::Mirror, "true").await;
    let sibling = create_account(&storage, owner, ImapAccountMode::Mirror, "true").await;
    let smart = storage
        .create_imap_local_folder(&NewImapLocalFolder {
            expected_mode_generation: None,
            account_identifier: account.identifier,
            name: "Local matches".into(),
            filter: "subject contains \"match\"".into(),
            include_in_inbox: false,
            created_at: 1,
        })
        .await
        .unwrap();
    let legacy = storage
        .create_imap_local_folder(&NewImapLocalFolder {
            expected_mode_generation: None,
            account_identifier: mirror.identifier,
            name: "Legacy matches".into(),
            filter: "subject contains \"match\"".into(),
            include_in_inbox: false,
            created_at: 1,
        })
        .await
        .unwrap();
    let sibling_smart = folder_identifier(&storage, &sibling, "Legacy matches").await;
    assert!(
        storage
            .list_imap_local_folders(account.identifier)
            .await
            .unwrap()
            .iter()
            .all(|folder| folder.name != "Legacy matches")
    );
    assert!(
        storage
            .list_imap_local_folders(mirror.identifier)
            .await
            .unwrap()
            .iter()
            .all(|folder| folder.name != "Local matches")
    );
    let private = storage
        .create_imap_private_folder(account.identifier, "Filed", 1, None)
        .await
        .unwrap();
    let message = insert_message(&storage, "match filed").await;
    let nonmatching = insert_message(&storage, "unrelated").await;
    let mirror_states = reconcile(&storage, &mirror).await;
    let mirror_smart = mailbox(
        &mirror_states,
        ImapMailboxSource::LocalFolder(legacy.identifier),
    )
    .identifier;
    assert_eq!(
        synchronize(&storage, mirror_smart, "subject contains \"match\"").await[0]
            .message_identifier,
        message
    );
    let primary_filed = storage
        .create_web_folder(owner, "Primary filed", None, &[], 1)
        .await
        .unwrap();
    storage
        .place_user_messages(
            owner,
            &[message],
            Some(primary_filed.identifier),
            &Expression::True,
        )
        .await
        .unwrap();
    assert_eq!(
        synchronize(&storage, mirror_smart, "subject contains \"match\"").await[0]
            .message_identifier,
        message,
        "smart results include matching primary mail filed in a normal folder"
    );
    let states = reconcile(&storage, &account).await;
    let inbox = mailbox(&states, ImapMailboxSource::Inbox).identifier;
    let trash = mailbox(&states, ImapMailboxSource::Trash).identifier;
    let manual = mailbox(&states, ImapMailboxSource::LocalFolder(private.identifier)).identifier;
    let smart_mailbox =
        mailbox(&states, ImapMailboxSource::LocalFolder(smart.identifier)).identifier;
    synchronize(&storage, inbox, "true").await;
    synchronize(&storage, manual, "true").await;
    synchronize(&storage, smart_mailbox, "subject contains \"match\"").await;
    let before = storage
        .imap_mailbox_members(smart_mailbox, 0)
        .await
        .unwrap();
    assert_eq!(before[0].message_identifier, message);
    for source in [message, nonmatching] {
        assert!(matches!(
            storage
                .copy_imap_members(
                    inbox,
                    &[member_uid(&storage, inbox, source).await],
                    smart_mailbox,
                    Some(&definition("subject contains \"match\"")),
                    false,
                )
                .await,
            Err(StorageError::ConstraintViolation(_))
        ));
    }
    assert_eq!(
        storage
            .imap_mailbox_members(smart_mailbox, 0)
            .await
            .unwrap(),
        before
    );
    storage
        .copy_imap_members(
            inbox,
            &[member_uid(&storage, inbox, message).await],
            manual,
            Some(&definition("true")),
            true,
        )
        .await
        .unwrap();
    assert_eq!(
        synchronize(&storage, smart_mailbox, "subject contains \"match\"").await[0].uid,
        before[0].uid
    );
    storage
        .delete_user_messages(owner, Some(&[message]), &Expression::True)
        .await
        .unwrap();
    assert!(
        synchronize(&storage, mirror_smart, "subject contains \"match\"")
            .await
            .is_empty()
    );
    assert_eq!(
        synchronize(&storage, smart_mailbox, "subject contains \"match\"").await[0]
            .message_identifier,
        message
    );
    storage
        .delete_imap_local_folder(private.identifier, None)
        .await
        .unwrap();
    assert!(
        synchronize(&storage, smart_mailbox, "subject contains \"match\"")
            .await
            .is_empty()
    );
    assert_eq!(
        synchronize(&storage, trash, "true").await[0].message_identifier,
        message
    );
    expunge_message(&storage, trash, message).await;
    assert!(
        synchronize(&storage, smart_mailbox, "subject contains \"match\"")
            .await
            .is_empty()
    );
    storage
        .update_imap_local_folder(
            sibling_smart,
            &ImapLocalFolderSettings {
                expected_mode_generation: None,
                name: "Shared legacy changed".into(),
                filter: "false".into(),
                include_in_inbox: true,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        storage
            .get_imap_local_folder(legacy.identifier)
            .await
            .unwrap()
            .unwrap()
            .filter,
        "false"
    );
    assert_eq!(
        storage
            .get_imap_local_folder(smart.identifier)
            .await
            .unwrap()
            .unwrap()
            .filter,
        "subject contains \"match\""
    );
    let sibling_states = reconcile(&storage, &sibling).await;
    let legacy_mailbox = mailbox(
        &sibling_states,
        ImapMailboxSource::LocalFolder(sibling_smart),
    )
    .identifier;
    assert!(
        synchronize(&storage, legacy_mailbox, "false")
            .await
            .is_empty()
    );
}

/// Independent folder deletion promotes surviving normal occurrences, sends sole occurrences to local Trash, and leaves smart results out of survivor selection.
#[tokio::test]
async fn independent_folder_deletion_preserves_normal_survivors() {
    let storage = SqliteStorage::memory().await.unwrap();
    let owner = create_owner(&storage).await;
    let account = create_account(&storage, owner, ImapAccountMode::Independent, "true").await;
    let first = storage
        .create_imap_private_folder(account.identifier, "Removed", 1, None)
        .await
        .unwrap();
    let second = storage
        .create_imap_private_folder(account.identifier, "Survivor", 1, None)
        .await
        .unwrap();
    let sole = insert_message(&storage, "sole").await;
    let survivor = insert_message(&storage, "survivor").await;
    let inbox_copy = insert_message(&storage, "inbox copy").await;
    let implicit_inbox = insert_message(&storage, "implicit Inbox").await;
    let states = reconcile(&storage, &account).await;
    let inbox = mailbox(&states, ImapMailboxSource::Inbox).identifier;
    let trash = mailbox(&states, ImapMailboxSource::Trash).identifier;
    let removed = mailbox(&states, ImapMailboxSource::LocalFolder(first.identifier)).identifier;
    let surviving = mailbox(&states, ImapMailboxSource::LocalFolder(second.identifier));
    for mailbox in [inbox, trash, removed, surviving.identifier] {
        synchronize(&storage, mailbox, "true").await;
    }
    for message in [sole, survivor, inbox_copy] {
        storage
            .copy_imap_members(
                inbox,
                &[member_uid(&storage, inbox, message).await],
                removed,
                Some(&definition("true")),
                true,
            )
            .await
            .unwrap();
    }
    let survivor_copy = storage
        .copy_imap_members(
            removed,
            &[member_uid(&storage, removed, survivor).await],
            surviving.identifier,
            Some(&definition("true")),
            false,
        )
        .await
        .unwrap();
    storage
        .copy_imap_members(
            removed,
            &[member_uid(&storage, removed, inbox_copy).await],
            inbox,
            Some(&definition("true")),
            false,
        )
        .await
        .unwrap();
    storage
        .copy_imap_members(
            inbox,
            &[member_uid(&storage, inbox, implicit_inbox).await],
            removed,
            Some(&definition("true")),
            false,
        )
        .await
        .unwrap();
    assert!(
        storage
            .delete_imap_local_folder(first.identifier, None)
            .await
            .unwrap()
    );
    assert!(storage.get_imap_mailbox(removed).await.unwrap().is_none());
    let inbox_members = synchronize(&storage, inbox, "true").await;
    assert_eq!(inbox_members.len(), 2);
    assert!(
        inbox_members
            .iter()
            .any(|member| member.message_identifier == inbox_copy)
    );
    assert!(
        inbox_members
            .iter()
            .any(|member| member.message_identifier == implicit_inbox)
    );
    let survivors = synchronize(&storage, surviving.identifier, "true").await;
    assert_eq!(survivors.len(), 1);
    assert_eq!(survivors[0].message_identifier, survivor);
    assert_eq!(survivors[0].uid, survivor_copy.destination_uids[0]);
    assert_eq!(
        synchronize(&storage, trash, "true").await[0].message_identifier,
        sole
    );
    assert_eq!(
        primary_messages(&storage, owner, None, false).await.len(),
        4
    );
}

/// Primary folder deletion works without active Mirror accounts and preserves private snapshots while the primary message is hidden.
#[tokio::test]
async fn primary_folder_deletion_preserves_independent_snapshots_without_mirrors() {
    let storage = SqliteStorage::memory().await.unwrap();
    let owner = create_owner(&storage).await;
    let primary_folder = storage
        .create_web_folder(owner, "Primary", None, &[], 1)
        .await
        .unwrap();
    let sole = insert_message(&storage, "sole primary").await;
    let hidden = insert_message(&storage, "hidden primary").await;
    storage
        .place_user_messages(
            owner,
            &[sole, hidden],
            Some(primary_folder.identifier),
            &Expression::True,
        )
        .await
        .unwrap();
    let account = create_account(&storage, owner, ImapAccountMode::Independent, "true").await;
    let snapshot = folder_identifier(&storage, &account, "Primary").await;
    let states = reconcile(&storage, &account).await;
    let snapshot_mailbox = mailbox(&states, ImapMailboxSource::LocalFolder(snapshot));
    let before = synchronize(&storage, snapshot_mailbox.identifier, "true").await;
    assert_eq!(before.len(), 2);
    storage
        .delete_user_messages(owner, Some(&[hidden]), &Expression::True)
        .await
        .unwrap();
    // Create dormant Mirror projections, then return to the previously materialized independent view.
    let account = switch_mode(&storage, &account, ImapAccountMode::Mirror).await;
    reconcile(&storage, &account).await;
    let account = switch_mode(&storage, &account, ImapAccountMode::Independent).await;
    storage
        .delete_web_folder(primary_folder.identifier)
        .await
        .unwrap();
    assert_eq!(primary_messages(&storage, owner, None, true).await, [sole]);
    assert_eq!(
        synchronize(&storage, snapshot_mailbox.identifier, "true").await,
        before
    );
    assert!(
        storage
            .get_imap_local_folder(snapshot)
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        storage
            .get_user_message(owner, hidden)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        storage
            .imap_member_content(
                snapshot_mailbox.identifier,
                member_uid(&storage, snapshot_mailbox.identifier, hidden).await
            )
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        storage
            .delete_imap_local_folder(snapshot, None)
            .await
            .unwrap()
    );
    let resumed = reconcile(&storage, &account).await;
    let trash = mailbox(&resumed, ImapMailboxSource::Trash).identifier;
    let trashed = synchronize(&storage, trash, "true").await;
    assert_eq!(trashed.len(), 2);
    expunge_message(&storage, trash, hidden).await;
    assert!(storage.get_message(hidden).await.unwrap().is_none());
    assert!(storage.get_message(sole).await.unwrap().is_some());
}

/// Parent and child renames preserve mailbox identities, UIDs, placements, and independent snapshots.
#[tokio::test]
async fn subtree_rename_keeps_uids_and_snapshot_names() {
    let storage = SqliteStorage::memory().await.unwrap();
    let owner = create_owner(&storage).await;
    let mirror = create_account(&storage, owner, ImapAccountMode::Mirror, "true").await;
    let parent = storage
        .create_imap_private_folder(mirror.identifier, "Projects", 1, None)
        .await
        .unwrap();
    let child = storage
        .create_imap_private_folder(mirror.identifier, "Projects/Rust", 1, None)
        .await
        .unwrap();
    assert_eq!(parent.kind, ImapFolderKind::Manual);
    assert_eq!(child.kind, ImapFolderKind::Manual);
    let message = insert_message(&storage, "child filed").await;
    storage
        .place_user_messages(
            owner,
            &[message],
            child.definition_identifier,
            &Expression::True,
        )
        .await
        .unwrap();
    let independent = create_account(&storage, owner, ImapAccountMode::Independent, "true").await;
    let states = reconcile(&storage, &mirror).await;
    let child_mailbox = mailbox(&states, ImapMailboxSource::LocalFolder(child.identifier));
    let before = synchronize(&storage, child_mailbox.identifier, "true").await;
    storage
        .update_imap_local_folder(
            parent.identifier,
            &ImapLocalFolderSettings {
                expected_mode_generation: None,
                name: "Archive".into(),
                filter: "true".into(),
                include_in_inbox: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        storage
            .get_imap_local_folder(child.identifier)
            .await
            .unwrap()
            .unwrap()
            .name,
        "Archive/Rust"
    );
    assert_eq!(
        synchronize(&storage, child_mailbox.identifier, "true").await,
        before
    );
    let after = reconcile(&storage, &mirror).await;
    assert_eq!(
        mailbox(&after, ImapMailboxSource::LocalFolder(child.identifier)).identifier,
        child_mailbox.identifier
    );
    assert_eq!(
        mailbox(&after, ImapMailboxSource::LocalFolder(child.identifier)).uid_validity,
        child_mailbox.uid_validity
    );
    assert!(
        storage
            .list_imap_local_folders(independent.identifier)
            .await
            .unwrap()
            .iter()
            .any(|folder| folder.name == "Projects/Rust")
    );
    let private_parent = folder_identifier(&storage, &independent, "Projects").await;
    let private_child = folder_identifier(&storage, &independent, "Projects/Rust").await;
    let independent_states = reconcile(&storage, &independent).await;
    let private_child_mailbox = mailbox(
        &independent_states,
        ImapMailboxSource::LocalFolder(private_child),
    );
    let private_before = synchronize(&storage, private_child_mailbox.identifier, "true").await;
    storage
        .update_imap_local_folder(
            private_parent,
            &ImapLocalFolderSettings {
                expected_mode_generation: None,
                name: "Local".into(),
                filter: "true".into(),
                include_in_inbox: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        storage
            .get_imap_local_folder(private_child)
            .await
            .unwrap()
            .unwrap()
            .name,
        "Local/Rust"
    );
    assert_eq!(
        synchronize(&storage, private_child_mailbox.identifier, "true").await,
        private_before
    );
    assert_eq!(
        storage
            .get_imap_local_folder(child.identifier)
            .await
            .unwrap()
            .unwrap()
            .name,
        "Archive/Rust"
    );
}

/// Deleting a Web definition without any Mirror projection still routes primary occurrences to Trash.
#[tokio::test]
async fn primary_folder_delete_without_mirror_projections_routes_to_trash() {
    let storage = SqliteStorage::memory().await.unwrap();
    let owner = create_owner(&storage).await;
    let folder = storage
        .create_web_folder(owner, "Unprojected", None, &[], 1)
        .await
        .unwrap();
    let message = insert_message(&storage, "unprojected primary folder").await;
    storage
        .place_user_messages(
            owner,
            &[message],
            Some(folder.identifier),
            &Expression::True,
        )
        .await
        .unwrap();
    let account = create_account(&storage, owner, ImapAccountMode::Independent, "true").await;
    let private = folder_identifier(&storage, &account, "Unprojected").await;
    let states = reconcile(&storage, &account).await;
    let snapshot = mailbox(&states, ImapMailboxSource::LocalFolder(private));
    let before = synchronize(&storage, snapshot.identifier, "true").await;
    assert!(
        storage
            .get_web_folder(folder.identifier)
            .await
            .unwrap()
            .unwrap()
            .exposed_accounts
            .is_empty()
    );
    storage.delete_web_folder(folder.identifier).await.unwrap();
    assert!(
        primary_messages(&storage, owner, None, false)
            .await
            .is_empty()
    );
    assert_eq!(
        primary_messages(&storage, owner, None, true).await,
        [message]
    );
    assert_eq!(
        synchronize(&storage, snapshot.identifier, "true").await,
        before
    );
    assert_eq!(
        mailbox(
            &reconcile(&storage, &account).await,
            ImapMailboxSource::LocalFolder(private)
        )
        .uid_validity,
        snapshot.uid_validity
    );
}
