//! Web-managed folders and account-local manual placement contract checks.
use sandpost_core::{ImapFolderKind, ImapMailboxSource, ImapMessageFlags, MessageIdentifier};
use sandpost_storage::{
    ImapFlagChange, ImapFlagOperation, ImapMembershipDefinition, MessageListQuery, NewImapAccount,
    Storage, StorageError,
};

use super::support::{
    create_imap_account_fixture, create_imap_account_fixture_with_mode,
    insert_message_identifier_for_imap_check, mailbox_member_uids, membership_definition,
    reconcile_imap_mailbox,
};
use crate::ConformanceFailure;
use crate::support::{failure, unique_token, unwrap_storage, verify, verify_equal};

/// Build an unrestricted paged query using the supplied membership filter.
fn web_mailbox_query(source: &str) -> MessageListQuery {
    MessageListQuery {
        filter: Some(membership_definition(source).filter),
        before: None,
        limit: 100,
    }
}

/// List message identifiers from an owner's general or selected Web mailbox.
async fn list_web_mailbox_message_identifiers(
    storage: &dyn Storage,
    check: &'static str,
    owner: sandpost_core::UserIdentifier,
    folder: Option<sandpost_core::ImapFolderIdentifier>,
) -> Result<Vec<MessageIdentifier>, ConformanceFailure> {
    Ok(unwrap_storage(
        check,
        "list_web_mailbox_messages",
        storage
            .list_web_mailbox_messages(owner, folder, web_mailbox_query("true"))
            .await,
    )?
    .into_iter()
    .map(|message| message.identifier)
    .collect())
}

/// Find a message's current UID in a materialized IMAP mailbox, if present.
async fn find_imap_mailbox_uid_for_message(
    storage: &dyn Storage,
    check: &'static str,
    mailbox: sandpost_core::ImapMailboxIdentifier,
    message: MessageIdentifier,
) -> Result<Option<u32>, ConformanceFailure> {
    Ok(unwrap_storage(
        check,
        "imap_mailbox_members",
        storage.imap_mailbox_members(mailbox, 0).await,
    )?
    .into_iter()
    .find(|member| member.message_identifier == message)
    .map(|member| member.uid))
}

/// Recalculate a mailbox's materialized membership using the supplied definition.
async fn synchronize_imap_mailbox(
    storage: &dyn Storage,
    check: &'static str,
    mailbox: sandpost_core::ImapMailboxIdentifier,
    definition: &ImapMembershipDefinition,
) -> Result<(), ConformanceFailure> {
    unwrap_storage(
        check,
        "synchronize_imap_mailbox",
        storage.synchronize_imap_mailbox(mailbox, definition).await,
    )?;
    Ok(())
}

/// Verify shared Mirror folders, exposure compatibility, isolated snapshots, and deletion routing.
pub async fn web_manual_folders_and_placements(
    storage: &dyn Storage,
) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "web_manual_folders_and_placements";
    let first = create_imap_account_fixture_with_mode(
        storage,
        CHECK,
        sandpost_core::ImapAccountMode::Mirror,
    )
    .await?;
    let owner = first.owner_identifier;
    let second = unwrap_storage(
        CHECK,
        "create sibling Mirror",
        storage
            .create_imap_account(&NewImapAccount {
                owner_identifier: owner,
                name: "Second Mirror".into(),
                username: format!("imap-{}-{}", std::process::id(), unique_token()),
                password_hash: "hash".into(),
                mirror_views: false,
                mode: sandpost_core::ImapAccountMode::Mirror,
                initial_authorization: sandpost_query::Expression::True,
                created_at: 1,
            })
            .await,
    )?;
    let other = create_imap_account_fixture(storage, CHECK).await?;
    let all = membership_definition("true");
    let folder = unwrap_storage(
        CHECK,
        "create primary folder",
        storage
            .create_web_folder(owner, "Manual", None, &[first.identifier], 1)
            .await,
    )?;
    verify(
        CHECK,
        folder.exposed_accounts.contains(&first.identifier)
            && folder.exposed_accounts.contains(&second.identifier),
        "every Mirror account projects a primary folder regardless of requested exposure",
    )?;
    let invalid = storage
        .create_web_folder(owner, "Invalid", None, &[other.identifier], 1)
        .await;
    verify(
        CHECK,
        matches!(invalid, Err(StorageError::NotFound)),
        "cross-owner folder exposure is refused",
    )?;
    let invalid = storage
        .update_web_folder(folder.identifier, "Invalid", None, &[other.identifier])
        .await;
    verify(
        CHECK,
        matches!(invalid, Err(StorageError::NotFound)),
        "cross-owner exposure update is refused",
    )?;
    let unchanged = unwrap_storage(
        CHECK,
        "read after refused update",
        storage.get_web_folder(folder.identifier).await,
    )?
    .ok_or_else(|| failure(CHECK, "folder disappeared"))?;
    verify_equal(
        CHECK,
        "refused update is atomic",
        unchanged.name.as_str(),
        "Manual",
    )?;
    let invalid = storage
        .create_web_folder(owner, "Manual", None, &[], 1)
        .await;
    verify(
        CHECK,
        matches!(invalid, Err(StorageError::DuplicateImapFolderName)),
        "owner folder names remain unique",
    )?;

    let mut projections = Vec::new();
    for account in [&first, &second] {
        projections.push(
            unwrap_storage(
                CHECK,
                "list Mirror projection",
                storage.list_imap_local_folders(account.identifier).await,
            )?
            .into_iter()
            .find(|local| local.definition_identifier == Some(folder.identifier))
            .ok_or_else(|| failure(CHECK, "automatic Mirror projection missing"))?,
        );
    }
    let mut inboxes = Vec::new();
    let mut manuals = Vec::new();
    let mut trashes = Vec::new();
    for (account, projection) in [&first, &second].into_iter().zip(&projections) {
        let sources = [
            ImapMailboxSource::Inbox,
            ImapMailboxSource::Trash,
            ImapMailboxSource::LocalFolder(projection.identifier),
        ];
        inboxes.push(
            reconcile_imap_mailbox(storage, CHECK, account, &sources, ImapMailboxSource::Inbox)
                .await?,
        );
        manuals.push(
            reconcile_imap_mailbox(
                storage,
                CHECK,
                account,
                &sources,
                ImapMailboxSource::LocalFolder(projection.identifier),
            )
            .await?,
        );
        trashes.push(
            reconcile_imap_mailbox(storage, CHECK, account, &sources, ImapMailboxSource::Trash)
                .await?,
        );
    }
    let copied =
        insert_message_identifier_for_imap_check(storage, CHECK, "manual copied survivor").await?;
    let sole =
        insert_message_identifier_for_imap_check(storage, CHECK, "manual sole occurrence").await?;
    unwrap_storage(
        CHECK,
        "place primary messages",
        storage
            .place_user_messages(owner, &[copied, sole], Some(folder.identifier), &all.filter)
            .await,
    )?;
    let invalid = storage
        .place_user_messages(
            other.owner_identifier,
            &[sole],
            Some(folder.identifier),
            &all.filter,
        )
        .await;
    verify(
        CHECK,
        matches!(invalid, Err(StorageError::NotFound)),
        "cross-owner placement is refused",
    )?;
    let invalid = storage
        .list_web_mailbox_messages(
            other.owner_identifier,
            Some(folder.identifier),
            web_mailbox_query("true"),
        )
        .await;
    verify(
        CHECK,
        matches!(invalid, Err(StorageError::NotFound)),
        "cross-owner folder listing is refused",
    )?;
    for mailbox in inboxes.iter().chain(&manuals).chain(&trashes) {
        synchronize_imap_mailbox(storage, CHECK, *mailbox, &all).await?;
    }
    for mailbox in &manuals {
        verify(
            CHECK,
            find_imap_mailbox_uid_for_message(storage, CHECK, *mailbox, sole)
                .await?
                .is_some(),
            "Web placement is shared by all Mirror accounts",
        )?;
    }
    let independent = unwrap_storage(
        CHECK,
        "create independent snapshot",
        storage
            .create_imap_account(&NewImapAccount {
                owner_identifier: owner,
                name: "Independent".into(),
                username: format!("imap-{}-{}", std::process::id(), unique_token()),
                password_hash: "hash".into(),
                mirror_views: false,
                mode: sandpost_core::ImapAccountMode::Independent,
                initial_authorization: sandpost_query::Expression::True,
                created_at: 2,
            })
            .await,
    )?;
    let snapshot = unwrap_storage(
        CHECK,
        "list snapshot folders",
        storage
            .list_imap_local_folders(independent.identifier)
            .await,
    )?
    .into_iter()
    .find(|local| local.name == "Manual")
    .ok_or_else(|| failure(CHECK, "all primary folders must be snapshotted"))?;
    verify(
        CHECK,
        snapshot.kind == ImapFolderKind::Private && snapshot.definition_identifier.is_none(),
        "snapshots are independent private definitions",
    )?;
    let independent_sources = [
        ImapMailboxSource::Inbox,
        ImapMailboxSource::Trash,
        ImapMailboxSource::LocalFolder(snapshot.identifier),
    ];
    let snapshot_mailbox = reconcile_imap_mailbox(
        storage,
        CHECK,
        &independent,
        &independent_sources,
        ImapMailboxSource::LocalFolder(snapshot.identifier),
    )
    .await?;
    synchronize_imap_mailbox(storage, CHECK, snapshot_mailbox, &all).await?;
    let copied_uid = find_imap_mailbox_uid_for_message(storage, CHECK, manuals[0], copied)
        .await?
        .ok_or_else(|| failure(CHECK, "COPY source missing"))?;
    unwrap_storage(
        CHECK,
        "copy primary folder occurrence to Inbox",
        storage
            .copy_imap_members(manuals[0], &[copied_uid], inboxes[0], Some(&all), false)
            .await,
    )?;
    synchronize_imap_mailbox(storage, CHECK, inboxes[1], &all).await?;
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, inboxes[1], copied)
            .await?
            .is_some(),
        "Mirror COPY adds a shared primary occurrence",
    )?;
    let refused = storage
        .copy_imap_members(manuals[0], &[copied_uid], inboxes[0], Some(&all), false)
        .await;
    verify(
        CHECK,
        matches!(refused, Err(StorageError::ConstraintViolation(_))),
        "duplicate occurrence COPY is refused",
    )?;
    unwrap_storage(
        CHECK,
        "rename without selected exposure",
        storage
            .update_web_folder(folder.identifier, "Renamed", None, &[])
            .await,
    )?;
    for (account, projection) in [&first, &second].into_iter().zip(&projections) {
        let current = unwrap_storage(
            CHECK,
            "Mirror projection after rename",
            storage.list_imap_local_folders(account.identifier).await,
        )?;
        verify(
            CHECK,
            current
                .iter()
                .any(|local| local.identifier == projection.identifier && local.name == "Renamed"),
            "Mirror projections retain their identity when exposure selection is empty",
        )?;
    }
    verify_equal(
        CHECK,
        "snapshot name is independent",
        unwrap_storage(
            CHECK,
            "read private snapshot",
            storage.get_imap_local_folder(snapshot.identifier).await,
        )?
        .map(|local| local.name),
        Some("Manual".into()),
    )?;
    unwrap_storage(
        CHECK,
        "delete primary folder",
        storage.delete_web_folder(folder.identifier).await,
    )?;
    for mailbox in inboxes.iter().chain(&trashes) {
        synchronize_imap_mailbox(storage, CHECK, *mailbox, &all).await?;
    }
    verify(
        CHECK,
        list_web_trash_message_identifiers(storage, CHECK, owner)
            .await?
            .contains(&sole),
        "deleting the sole primary folder occurrence sends it to primary Trash",
    )?;
    verify(
        CHECK,
        list_web_mailbox_message_identifiers(storage, CHECK, owner, None)
            .await?
            .contains(&copied),
        "folder deletion promotes a surviving normal occurrence",
    )?;
    for mailbox in &trashes {
        verify(
            CHECK,
            find_imap_mailbox_uid_for_message(storage, CHECK, *mailbox, sole)
                .await?
                .is_some(),
            "primary folder deletion reaches every Mirror Trash",
        )?;
    }
    synchronize_imap_mailbox(storage, CHECK, snapshot_mailbox, &all).await?;
    verify_equal(
        CHECK,
        "primary deletion preserves independent snapshot contents",
        mailbox_member_uids(storage, CHECK, snapshot_mailbox)
            .await?
            .len(),
        2,
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "read preserved independent snapshot",
            storage.get_imap_local_folder(snapshot.identifier).await,
        )?
        .is_some(),
        "Web deletion preserves independent folder definitions",
    )?;
    Ok(())
}

/// Flag one mailbox member `\Deleted` so a later expunge selects it.
async fn flag_imap_member_deleted(
    storage: &dyn Storage,
    check: &'static str,
    mailbox: sandpost_core::ImapMailboxIdentifier,
    uid: u32,
) -> Result<(), ConformanceFailure> {
    unwrap_storage(
        check,
        "flag member deleted",
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
            .await,
    )?;
    Ok(())
}

/// Verify COPY, `\Deleted`, and EXPUNGE of a primary occurrence transfers it like MOVE.
///
/// Clients without MOVE (Apple Mail) organize and delete this way. The surviving copy becomes
/// the account placement and no tombstone is recorded; expunging a copy or the last occurrence
/// removes only that occurrence; deleting the final Trash occurrence releases only this account.
pub async fn imap_copy_then_expunge_transfers(
    storage: &dyn Storage,
) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "imap_copy_then_expunge_transfers";
    let token = format!("transfer-{}-{}", std::process::id(), unique_token());
    let account = create_imap_account_fixture(storage, CHECK).await?;
    let owner = account.owner_identifier;
    let all = membership_definition("true");
    let empty = membership_definition("false");
    let folder = unwrap_storage(
        CHECK,
        "create private folder",
        storage
            .create_imap_private_folder(account.identifier, "Pippo", 1_700_000_010, None)
            .await,
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
    let folder_mailbox = reconcile_imap_mailbox(
        storage,
        CHECK,
        &account,
        &sources,
        ImapMailboxSource::LocalFolder(folder.identifier),
    )
    .await?;
    let filed_message =
        insert_message_identifier_for_imap_check(storage, CHECK, &format!("{token} filed")).await?;
    let trashed_message =
        insert_message_identifier_for_imap_check(storage, CHECK, &format!("{token} trashed"))
            .await?;
    let copied_message =
        insert_message_identifier_for_imap_check(storage, CHECK, &format!("{token} copied"))
            .await?;
    for mailbox in [inbox, trash, folder_mailbox] {
        synchronize_imap_mailbox(storage, CHECK, mailbox, &all).await?;
    }
    let inbox_uid = |message| async move {
        find_imap_mailbox_uid_for_message(storage, CHECK, inbox, message)
            .await?
            .ok_or_else(|| failure(CHECK, "transfer source is absent from Inbox"))
    };
    let filed_uid = inbox_uid(filed_message).await?;
    let trashed_uid = inbox_uid(trashed_message).await?;

    let filed_copy = unwrap_storage(
        CHECK,
        "copy into private folder",
        storage
            .copy_imap_members(inbox, &[filed_uid], folder_mailbox, Some(&all), false)
            .await,
    )?;
    let copied_uid = inbox_uid(copied_message).await?;
    let destination_before = unwrap_storage(
        CHECK,
        "destination before refused batch",
        storage.get_imap_mailbox(folder_mailbox).await,
    )?;
    for remove_from_source in [false, true] {
        let refused = storage
            .copy_imap_members(
                inbox,
                &[copied_uid, filed_uid],
                folder_mailbox,
                Some(&all),
                remove_from_source,
            )
            .await;
        verify(
            CHECK,
            matches!(refused, Err(StorageError::ConstraintViolation(_))),
            "a batch containing a duplicate destination is refused atomically",
        )?;
        verify_equal(
            CHECK,
            "failed batch does not consume destination UIDs",
            unwrap_storage(
                CHECK,
                "destination after refused batch",
                storage.get_imap_mailbox(folder_mailbox).await,
            )?,
            destination_before,
        )?;
        verify_equal(
            CHECK,
            "failed batch preserves source occurrence",
            inbox_uid(copied_message).await?,
            copied_uid,
        )?;
        verify_equal(
            CHECK,
            "failed batch rolls back new destination occurrences",
            mailbox_member_uids(storage, CHECK, folder_mailbox).await?,
            filed_copy.destination_uids.clone(),
        )?;
        let refused = storage
            .copy_imap_members(
                inbox,
                &[copied_uid],
                folder_mailbox,
                Some(&empty),
                remove_from_source,
            )
            .await;
        verify(
            CHECK,
            matches!(refused, Err(StorageError::ConstraintViolation(_))),
            "destination authorization is required for COPY and MOVE",
        )?;
    }
    let refused = storage
        .copy_imap_members(inbox, &[copied_uid], inbox, Some(&all), false)
        .await;
    verify(
        CHECK,
        matches!(refused, Err(StorageError::ConstraintViolation(_))),
        "same-mailbox COPY cannot create a duplicate occurrence",
    )?;
    let trash_copy = unwrap_storage(
        CHECK,
        "copy into Trash",
        storage
            .copy_imap_members(inbox, &[trashed_uid], trash, Some(&all), false)
            .await,
    )?;
    let repeated_trash_copy = unwrap_storage(
        CHECK,
        "coalesce existing Trash copy",
        storage
            .copy_imap_members(inbox, &[trashed_uid], trash, Some(&all), false)
            .await,
    )?;
    verify(
        CHECK,
        !repeated_trash_copy.all_new,
        "retained Trash UIDs must not be advertised as newly assigned",
    )?;
    verify_equal(
        CHECK,
        "repeated Trash COPY preserves destination UIDs",
        repeated_trash_copy.destination_uids,
        trash_copy.destination_uids,
    )?;
    flag_imap_member_deleted(storage, CHECK, inbox, filed_uid).await?;
    flag_imap_member_deleted(storage, CHECK, inbox, trashed_uid).await?;
    verify_equal(
        CHECK,
        "expunged transfer source UIDs",
        unwrap_storage(
            CHECK,
            "expunge transferred sources",
            storage.expunge_imap_members(inbox, None, None).await,
        )?,
        {
            let mut expected = vec![filed_uid, trashed_uid];
            expected.sort_unstable();
            expected
        },
    )?;
    let web_messages = list_web_mailbox_message_identifiers(storage, CHECK, owner, None).await?;
    verify(
        CHECK,
        web_messages.contains(&filed_message) && web_messages.contains(&trashed_message),
        "COPY then EXPUNGE of the source does not create a user deletion tombstone",
    )?;
    for definition in [&empty, &all] {
        for mailbox in [inbox, trash, folder_mailbox] {
            synchronize_imap_mailbox(storage, CHECK, mailbox, definition).await?;
        }
    }
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, inbox, filed_message)
            .await?
            .is_none()
            && find_imap_mailbox_uid_for_message(storage, CHECK, inbox, trashed_message)
                .await?
                .is_none(),
        "transferred messages do not return to the Inbox fallback",
    )?;
    let filed_restored_uid =
        find_imap_mailbox_uid_for_message(storage, CHECK, folder_mailbox, filed_message)
            .await?
            .ok_or_else(|| failure(CHECK, "the surviving folder copy did not become placement"))?;
    verify(
        CHECK,
        filed_restored_uid > filed_copy.destination_uids[0],
        "the promoted placement survives authorization loss and regain with a fresh UID",
    )?;
    let trashed_trash_uid =
        find_imap_mailbox_uid_for_message(storage, CHECK, trash, trashed_message)
            .await?
            .ok_or_else(|| failure(CHECK, "the surviving Trash copy did not become placement"))?;

    flag_imap_member_deleted(storage, CHECK, trash, trashed_trash_uid).await?;
    unwrap_storage(
        CHECK,
        "expunge the last occurrence from Trash",
        storage.expunge_imap_members(trash, None, None).await,
    )?;
    // Authorization loss and regain assigned the Inbox occurrence a fresh UID.
    let copied_uid = inbox_uid(copied_message).await?;
    let copy_result = unwrap_storage(
        CHECK,
        "copy before expunging the copy",
        storage
            .copy_imap_members(inbox, &[copied_uid], folder_mailbox, Some(&all), false)
            .await,
    )?;
    flag_imap_member_deleted(
        storage,
        CHECK,
        folder_mailbox,
        copy_result.destination_uids[0],
    )
    .await?;
    unwrap_storage(
        CHECK,
        "expunge an additional COPY occurrence",
        storage
            .expunge_imap_members(folder_mailbox, None, None)
            .await,
    )?;
    let web_messages = list_web_mailbox_message_identifiers(storage, CHECK, owner, None).await?;
    verify(
        CHECK,
        web_messages.contains(&trashed_message) && web_messages.contains(&copied_message),
        "Independent Trash deletion and COPY removal preserve the primary view",
    )?;
    verify(
        CHECK,
        web_messages.contains(&filed_message),
        "an unrelated transferred message is unaffected",
    )?;
    let copied_again = unwrap_storage(
        CHECK,
        "copy before moving an additional occurrence",
        storage
            .copy_imap_members(inbox, &[copied_uid], folder_mailbox, Some(&all), false)
            .await,
    )?;
    unwrap_storage(
        CHECK,
        "move additional copy to Trash",
        storage
            .copy_imap_members(
                folder_mailbox,
                &copied_again.destination_uids,
                trash,
                Some(&all),
                true,
            )
            .await,
    )?;
    for mailbox in [inbox, trash, folder_mailbox] {
        synchronize_imap_mailbox(storage, CHECK, mailbox, &all).await?;
    }
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, inbox, copied_message)
            .await?
            .is_some()
            && find_imap_mailbox_uid_for_message(storage, CHECK, folder_mailbox, copied_message)
                .await?
                .is_none(),
        "moving an additional copy preserves the independent Inbox occurrence",
    )?;
    let trash_copy = find_imap_mailbox_uid_for_message(storage, CHECK, trash, copied_message)
        .await?
        .ok_or_else(|| failure(CHECK, "moved copy missing from Trash"))?;
    flag_imap_member_deleted(storage, CHECK, trash, trash_copy).await?;
    unwrap_storage(
        CHECK,
        "remove additional Trash copy",
        storage
            .expunge_imap_members(trash, Some(&[trash_copy]), None)
            .await,
    )?;
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, inbox, copied_message)
            .await?
            .is_some(),
        "expunging an additional Trash occurrence preserves the normal primary occurrence",
    )?;
    Ok(())
}

/// List message identifiers in an owner's Web Trash.
async fn list_web_trash_message_identifiers(
    storage: &dyn Storage,
    check: &'static str,
    owner: sandpost_core::UserIdentifier,
) -> Result<Vec<MessageIdentifier>, ConformanceFailure> {
    Ok(unwrap_storage(
        check,
        "list_web_trash_messages",
        storage
            .list_web_trash_messages(owner, web_mailbox_query("true"))
            .await,
    )?
    .into_iter()
    .map(|message| message.identifier)
    .collect())
}

/// Verify Web Trash and Mirror share placement, restoration, and permanent deletion state.
pub async fn web_trash(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "web_trash";
    let token = format!("trash-{}-{}", std::process::id(), unique_token());
    let account = create_imap_account_fixture_with_mode(
        storage,
        CHECK,
        sandpost_core::ImapAccountMode::Mirror,
    )
    .await?;
    let owner = account.owner_identifier;
    let all = membership_definition("true");
    let archive = unwrap_storage(
        CHECK,
        "create Web folder",
        storage
            .create_web_folder(owner, "Archive", None, &[], 1_700_000_020)
            .await,
    )?;
    let kept_definition = unwrap_storage(
        CHECK,
        "create primary kept folder",
        storage
            .create_web_folder(owner, "Kept", None, &[], 1_700_000_021)
            .await,
    )?;
    let kept = unwrap_storage(
        CHECK,
        "find kept Mirror projection",
        storage.list_imap_local_folders(account.identifier).await,
    )?
    .into_iter()
    .find(|folder| folder.definition_identifier == Some(kept_definition.identifier))
    .ok_or_else(|| failure(CHECK, "kept projection missing"))?;
    let sources = [
        ImapMailboxSource::Inbox,
        ImapMailboxSource::Trash,
        ImapMailboxSource::LocalFolder(kept.identifier),
    ];
    let inbox =
        reconcile_imap_mailbox(storage, CHECK, &account, &sources, ImapMailboxSource::Inbox)
            .await?;
    let trash =
        reconcile_imap_mailbox(storage, CHECK, &account, &sources, ImapMailboxSource::Trash)
            .await?;
    let kept_mailbox = reconcile_imap_mailbox(
        storage,
        CHECK,
        &account,
        &sources,
        ImapMailboxSource::LocalFolder(kept.identifier),
    )
    .await?;
    let mut fixture = Vec::new();
    for name in ["trashed", "filed", "restored", "overridden", "unaffected"] {
        fixture.push(
            insert_message_identifier_for_imap_check(storage, CHECK, &format!("{token} {name}"))
                .await?,
        );
    }
    let [trashed, filed, restored, overridden, unaffected] = fixture[..] else {
        unreachable!("the fixture has five messages");
    };
    for mailbox in [inbox, trash, kept_mailbox] {
        synchronize_imap_mailbox(storage, CHECK, mailbox, &all).await?;
    }
    unwrap_storage(
        CHECK,
        "file message in Web folder",
        storage
            .place_user_messages(owner, &[filed], Some(archive.identifier), &all.filter)
            .await,
    )?;
    let overridden_uid = find_imap_mailbox_uid_for_message(storage, CHECK, inbox, overridden)
        .await?
        .ok_or_else(|| failure(CHECK, "override source is absent from Inbox"))?;
    unwrap_storage(
        CHECK,
        "move into private folder",
        storage
            .copy_imap_members(inbox, &[overridden_uid], kept_mailbox, Some(&all), true)
            .await,
    )?;

    let oversized = vec![trashed; 257];
    let refused = storage
        .trash_user_messages(owner, &oversized, &all.filter, 1_700_000_022)
        .await;
    verify(
        CHECK,
        matches!(refused, Err(StorageError::ConstraintViolation(_))),
        format!("a Trash batch above 256 messages is refused: {refused:?}"),
    )?;
    verify_equal(
        CHECK,
        "trashed message count",
        unwrap_storage(
            CHECK,
            "trash messages",
            storage
                .trash_user_messages(
                    owner,
                    &[trashed, filed, restored, overridden, trashed],
                    &all.filter,
                    1_700_000_022,
                )
                .await,
        )?,
        4,
    )?;
    let inbox_messages = list_web_mailbox_message_identifiers(storage, CHECK, owner, None).await?;
    verify(
        CHECK,
        inbox_messages.contains(&unaffected)
            && [trashed, filed, restored, overridden]
                .iter()
                .all(|message| !inbox_messages.contains(message)),
        "trashed mail leaves the general Web mailbox",
    )?;
    verify(
        CHECK,
        list_web_mailbox_message_identifiers(storage, CHECK, owner, Some(archive.identifier))
            .await?
            .is_empty(),
        "Trash replaces Web folder placement",
    )?;
    let mut trash_listing = list_web_trash_message_identifiers(storage, CHECK, owner).await?;
    trash_listing.sort_unstable();
    let mut expected_trash = vec![trashed, filed, restored, overridden];
    expected_trash.sort_unstable();
    verify_equal(CHECK, "Web Trash listing", trash_listing, expected_trash)?;
    let indexed = unwrap_storage(
        CHECK,
        "load current revisions",
        storage.indexed_messages(&[trashed, unaffected]).await,
    )?;
    let requested = indexed
        .iter()
        .map(|message| (message.identifier, message.revision))
        .collect::<Vec<_>>();
    verify_equal(
        CHECK,
        "search hydration omits trashed mail",
        unwrap_storage(
            CHECK,
            "hydrate current user messages",
            storage
                .hydrate_current_user_messages(owner, &requested, &all.filter)
                .await,
        )?
        .into_iter()
        .map(|summary| summary.identifier)
        .collect::<Vec<_>>(),
        vec![unaffected],
    )?;

    for mailbox in [inbox, trash, kept_mailbox] {
        synchronize_imap_mailbox(storage, CHECK, mailbox, &all).await?;
    }
    for message in [trashed, filed, restored] {
        verify(
            CHECK,
            find_imap_mailbox_uid_for_message(storage, CHECK, inbox, message)
                .await?
                .is_none()
                && find_imap_mailbox_uid_for_message(storage, CHECK, trash, message)
                    .await?
                    .is_some(),
            "Web Trash moves mail from IMAP INBOX to every account's Trash",
        )?;
    }
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, kept_mailbox, overridden)
            .await?
            .is_none()
            && find_imap_mailbox_uid_for_message(storage, CHECK, trash, overridden)
                .await?
                .is_some(),
        "Mirror placement follows the primary user's Web Trash",
    )?;

    unwrap_storage(
        CHECK,
        "restore from Trash by placement",
        storage
            .place_user_messages(owner, &[restored], None, &all.filter)
            .await,
    )?;
    verify(
        CHECK,
        list_web_mailbox_message_identifiers(storage, CHECK, owner, None)
            .await?
            .contains(&restored)
            && !list_web_trash_message_identifiers(storage, CHECK, owner)
                .await?
                .contains(&restored),
        "placement restores a message from the Web Trash",
    )?;
    for mailbox in [inbox, trash] {
        synchronize_imap_mailbox(storage, CHECK, mailbox, &all).await?;
    }
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, inbox, restored)
            .await?
            .is_some()
            && find_imap_mailbox_uid_for_message(storage, CHECK, trash, restored)
                .await?
                .is_none(),
        "a restored message returns to IMAP INBOX",
    )?;

    let trashed_uid = find_imap_mailbox_uid_for_message(storage, CHECK, trash, trashed)
        .await?
        .ok_or_else(|| failure(CHECK, "trashed message is absent from IMAP Trash"))?;
    flag_imap_member_deleted(storage, CHECK, trash, trashed_uid).await?;
    unwrap_storage(
        CHECK,
        "expunge from IMAP Trash",
        storage.expunge_imap_members(trash, None, None).await,
    )?;
    verify(
        CHECK,
        !list_web_trash_message_identifiers(storage, CHECK, owner)
            .await?
            .contains(&trashed),
        "expunging from IMAP Trash permanently deletes the Web Trash entry",
    )?;
    verify_equal(
        CHECK,
        "emptied Trash count",
        unwrap_storage(
            CHECK,
            "empty Trash",
            storage.empty_user_trash(owner, &all.filter).await,
        )?,
        2,
    )?;
    verify(
        CHECK,
        list_web_trash_message_identifiers(storage, CHECK, owner)
            .await?
            .is_empty(),
        "emptying the Trash leaves it empty",
    )?;
    let inbox_messages = list_web_mailbox_message_identifiers(storage, CHECK, owner, None).await?;
    verify(
        CHECK,
        !inbox_messages.contains(&filed)
            && !inbox_messages.contains(&overridden)
            && inbox_messages.contains(&unaffected)
            && inbox_messages.contains(&restored),
        "emptying the Trash tombstones only trashed mail",
    )?;
    synchronize_imap_mailbox(storage, CHECK, kept_mailbox, &all).await?;
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, kept_mailbox, overridden)
            .await?
            .is_none(),
        "the emptied message leaves every IMAP mailbox of its owner",
    )?;
    Ok(())
}
