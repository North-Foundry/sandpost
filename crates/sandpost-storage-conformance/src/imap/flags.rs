//! IMAP flags, recent state, copies, expunges, and exclusions checks.
use sandpost_core::{ImapMailboxSource, ImapMessageFlags};
use sandpost_storage::{
    ImapFlagChange, ImapFlagOperation, NewImapAccount, NewImapLocalFolder, Storage, StorageError,
};

use super::support::{
    create_imap_account_fixture, insert_message_for_imap_check, mailbox_member_uids,
    membership_definition, reconcile_imap_mailbox,
};
use crate::ConformanceFailure;
use crate::support::{failure, unique_token, unwrap_storage, verify, verify_equal};

/// Verify independent flags, staged expunge, sibling isolation, recent claims, and smart-folder COPY refusal.
pub async fn imap_flags_and_expunge(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "imap_flags_and_expunge";
    let token = format!("flags-{}-{}", std::process::id(), unique_token());
    let account = create_imap_account_fixture(storage, CHECK).await?;
    let owner = unwrap_storage(
        CHECK,
        "load IMAP account",
        storage.get_imap_account(account.identifier).await,
    )?
    .ok_or_else(|| failure(CHECK, "IMAP account missing"))?
    .owner_identifier;
    let second_account = unwrap_storage(
        CHECK,
        "create second account for owner",
        storage
            .create_imap_account(&NewImapAccount {
                owner_identifier: owner,
                name: "Second conformance IMAP".to_owned(),
                username: format!("imap-{}-{}", std::process::id(), unique_token()),
                password_hash: "hash-two".to_owned(),
                mirror_views: true,
                mode: sandpost_core::ImapAccountMode::Independent,
                initial_authorization: sandpost_query::Expression::True,
                created_at: 1_700_000_000,
            })
            .await,
    )?;
    let second_inbox = reconcile_imap_mailbox(
        storage,
        CHECK,
        &second_account,
        &[ImapMailboxSource::Inbox],
        ImapMailboxSource::Inbox,
    )
    .await?;
    let folder = unwrap_storage(
        CHECK,
        "create_imap_local_folder",
        storage
            .create_imap_local_folder(&NewImapLocalFolder {
                expected_mode_generation: None,
                account_identifier: account.identifier,
                name: "Only one".to_owned(),
                filter: format!("subject contains \"{token} one\""),
                include_in_inbox: true,
                created_at: 1,
            })
            .await,
    )?;
    let other_account = create_imap_account_fixture(storage, CHECK).await?;
    let other_inbox = reconcile_imap_mailbox(
        storage,
        CHECK,
        &other_account,
        &[ImapMailboxSource::Inbox],
        ImapMailboxSource::Inbox,
    )
    .await?;
    verify(
        CHECK,
        matches!(
            storage
                .reconcile_imap_mailboxes(
                    other_account.identifier,
                    &[ImapMailboxSource::LocalFolder(folder.identifier)],
                    None,
                )
                .await,
            Err(StorageError::NotFound)
        ),
        "reconciliation must reject another account's folder",
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "mailbox after refused reconciliation",
            storage.get_imap_mailbox(other_inbox).await,
        )?
        .is_some(),
        "a refused reconciliation must preserve existing mailbox identities",
    )?;
    unwrap_storage(
        CHECK,
        "delete other IMAP account",
        storage.delete_imap_account(other_account.identifier).await,
    )?;
    let sources = [
        ImapMailboxSource::Inbox,
        ImapMailboxSource::Trash,
        ImapMailboxSource::LocalFolder(folder.identifier),
    ];
    let inbox =
        reconcile_imap_mailbox(storage, CHECK, &account, &sources, ImapMailboxSource::Inbox)
            .await?;
    let trash =
        reconcile_imap_mailbox(storage, CHECK, &account, &sources, ImapMailboxSource::Trash)
            .await?;
    let local = reconcile_imap_mailbox(
        storage,
        CHECK,
        &account,
        &sources,
        ImapMailboxSource::LocalFolder(folder.identifier),
    )
    .await?;
    insert_message_for_imap_check(storage, CHECK, &format!("{token} one")).await?;
    insert_message_for_imap_check(storage, CHECK, &format!("{token} two")).await?;
    let everything = membership_definition(&format!("subject contains \"{token}\""));
    let only_one = membership_definition(&format!("subject contains \"{token} one\""));
    for (mailbox, membership) in [
        (inbox, &everything),
        (local, &only_one),
        (second_inbox, &everything),
    ] {
        unwrap_storage(
            CHECK,
            "synchronize",
            storage.synchronize_imap_mailbox(mailbox, membership).await,
        )?;
    }
    verify_equal(
        CHECK,
        "second account starts with both messages",
        mailbox_member_uids(storage, CHECK, second_inbox).await?,
        vec![1, 2],
    )?;
    let previous = unwrap_storage(
        CHECK,
        "claim_imap_recent",
        storage.claim_imap_recent(inbox).await,
    )?;
    verify_equal(CHECK, "first recent watermark", previous, Some(0))?;
    let counts = unwrap_storage(
        CHECK,
        "imap_mailbox_counts",
        storage.imap_mailbox_counts(inbox).await,
    )?
    .ok_or_else(|| failure(CHECK, "counts missing"))?;
    verify(
        CHECK,
        counts.messages == 2 && counts.unseen == 2 && counts.recent == 0,
        format!("counts after claiming recent: {counts:?}"),
    )?;
    let cursor = unwrap_storage(
        CHECK,
        "imap_flag_cursor",
        storage.imap_flag_cursor(account.identifier).await,
    )?;
    let stored = unwrap_storage(
        CHECK,
        "store_imap_flags",
        storage
            .store_imap_flags(
                inbox,
                &[1, 2, 99],
                &ImapFlagChange {
                    operation: ImapFlagOperation::Add,
                    flags: ImapMessageFlags {
                        seen: true,
                        deleted: true,
                        keywords: vec!["$Work".to_owned()],
                        ..ImapMessageFlags::default()
                    },
                },
                None,
            )
            .await,
    )?;
    verify_equal(
        CHECK,
        "stored UIDs",
        stored.iter().map(|(uid, _)| *uid).collect::<Vec<_>>(),
        vec![1, 2],
    )?;
    let local_members = unwrap_storage(
        CHECK,
        "local members",
        storage.imap_mailbox_members(local, 0).await,
    )?;
    verify(
        CHECK,
        local_members.len() == 1
            && local_members[0].flags.seen
            && !local_members[0].flags.deleted
            && local_members[0].flags.keywords == ["$Work"],
        format!("shared flags apply to every mailbox but \\Deleted does not: {local_members:?}"),
    )?;
    let changes = unwrap_storage(
        CHECK,
        "imap_flag_changes",
        storage.imap_flag_changes(local, cursor, u32::MAX).await,
    )?;
    verify(
        CHECK,
        changes.changes.len() == 1 && changes.cursor > cursor,
        format!("flag changes after the cursor: {changes:?}"),
    )?;
    let unchanged = unwrap_storage(
        CHECK,
        "imap_flag_changes at cursor",
        storage
            .imap_flag_changes(local, changes.cursor, u32::MAX)
            .await,
    )?;
    verify(
        CHECK,
        unchanged.changes.is_empty(),
        "no changes after the latest cursor",
    )?;
    for definition in [Some(&only_one), None] {
        let refused = storage
            .copy_imap_members(inbox, &[2], local, definition, false)
            .await;
        verify(
            CHECK,
            matches!(refused, Err(StorageError::ConstraintViolation(_))),
            format!("a dynamic copy requires a matching definition: {refused:?}"),
        )?;
    }
    verify_equal(
        CHECK,
        "destination UIDs after refused copy",
        mailbox_member_uids(storage, CHECK, local).await?,
        vec![1],
    )?;
    let expunged = unwrap_storage(
        CHECK,
        "expunge_imap_members",
        storage.expunge_imap_members(inbox, Some(&[2]), None).await,
    )?;
    verify_equal(CHECK, "expunged UIDs", expunged, vec![2])?;
    verify_equal(
        CHECK,
        "Independent EXPUNGE preserves sibling retention",
        mailbox_member_uids(storage, CHECK, second_inbox).await?,
        vec![1, 2],
    )?;
    unwrap_storage(
        CHECK,
        "synchronize after expunge",
        storage.synchronize_imap_mailbox(inbox, &everything).await,
    )?;
    verify_equal(
        CHECK,
        "an expunged Inbox occurrence stays in account Trash",
        mailbox_member_uids(storage, CHECK, inbox).await?,
        vec![1],
    )?;
    unwrap_storage(
        CHECK,
        "materialize the first-stage Trash occurrence",
        storage.synchronize_imap_mailbox(trash, &everything).await,
    )?;
    let moved = unwrap_storage(
        CHECK,
        "move to trash",
        storage
            .copy_imap_members(inbox, &[1], trash, Some(&everything), true)
            .await,
    )?;
    verify_equal(CHECK, "moved UIDs", moved.removed_uids, vec![1])?;
    verify(
        CHECK,
        mailbox_member_uids(storage, CHECK, inbox).await?.is_empty()
            && mailbox_member_uids(storage, CHECK, trash).await? == [1, 2],
        "a move leaves the message only in the destination",
    )?;
    let cleared = unwrap_storage(
        CHECK,
        "clear_imap_exclusions",
        storage.clear_imap_exclusions(account.identifier).await,
    )?;
    verify_equal(CHECK, "forgotten mailbox exclusions", cleared, 0)?;
    unwrap_storage(
        CHECK,
        "synchronize Inbox after clearing exclusions",
        storage.synchronize_imap_mailbox(inbox, &everything).await,
    )?;
    verify_equal(
        CHECK,
        "a Trash MOVE remains out of Inbox after synchronization",
        mailbox_member_uids(storage, CHECK, inbox).await?,
        vec![],
    )?;
    unwrap_storage(
        CHECK,
        "synchronize Trash after clearing exclusions",
        storage.synchronize_imap_mailbox(trash, &everything).await,
    )?;
    verify_equal(
        CHECK,
        "Trash placements remain after clearing exclusions",
        mailbox_member_uids(storage, CHECK, trash).await?,
        vec![1, 2],
    )?;
    let metadata = unwrap_storage(
        CHECK,
        "imap_member_metadata",
        storage.imap_member_metadata(trash, &[1, 2, 3]).await,
    )?;
    verify(
        CHECK,
        metadata.len() == 2
            && metadata
                .iter()
                .all(|item| item.size > 0 && item.received_at == 1_700_000_100),
        format!("metadata: {metadata:?}"),
    )?;
    unwrap_storage(
        CHECK,
        "delete_imap_account",
        storage.delete_imap_account(account.identifier).await,
    )?;
    unwrap_storage(
        CHECK,
        "delete second IMAP account",
        storage.delete_imap_account(second_account.identifier).await,
    )?;
    Ok(())
}
