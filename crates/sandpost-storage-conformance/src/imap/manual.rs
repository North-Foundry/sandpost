//! Web-managed folders and account-local manual placement contract checks.
use sandpost_core::{ImapFolderKind, ImapMailboxSource, ImapMessageFlags, MessageIdentifier};
use sandpost_storage::{
    ImapFlagChange, ImapFlagOperation, ImapMembershipDefinition, MessageListQuery, NewImapAccount,
    Storage, StorageError,
};

use super::support::{
    create_imap_account_fixture, insert_message_identifier_for_imap_check, mailbox_member_uids,
    membership_definition, reconcile_imap_mailbox,
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

/// Verify Web folders, selected exposures, transactional placement, and IMAP-local overrides.
pub async fn web_manual_folders_and_placements(
    storage: &dyn Storage,
) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "web_manual_folders_and_placements";
    let token = format!("manual-{}-{}", std::process::id(), unique_token());
    let first = create_imap_account_fixture(storage, CHECK).await?;
    let owner = first.owner_identifier;
    let second = unwrap_storage(
        CHECK,
        "create second account for owner",
        storage
            .create_imap_account(&NewImapAccount {
                owner_identifier: owner,
                name: "Second account".into(),
                username: format!("imap-{}-{}", std::process::id(), unique_token()),
                password_hash: "hash-two".into(),
                mirror_views: false,
                created_at: 1_700_000_000,
            })
            .await,
    )?;
    let other = create_imap_account_fixture(storage, CHECK).await?;
    let all = membership_definition("true");
    let empty = membership_definition("false");

    let folder = unwrap_storage(
        CHECK,
        "create Web folder",
        storage
            .create_web_folder(owner, "Manual", &[first.identifier], 1_700_000_001)
            .await,
    )?;
    verify_equal(
        CHECK,
        "selected Web exposure",
        folder.exposed_accounts.clone(),
        vec![first.identifier],
    )?;
    verify_equal(
        CHECK,
        "folder listed only for its owner",
        unwrap_storage(
            CHECK,
            "list owner folders",
            storage.list_web_folders(owner).await,
        )?
        .len(),
        1,
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "list other user's folders",
            storage.list_web_folders(other.owner_identifier).await,
        )?
        .is_empty(),
        "Web folder definitions are private to their owner",
    )?;
    let first_projection = unwrap_storage(
        CHECK,
        "list first account folders",
        storage.list_imap_local_folders(first.identifier).await,
    )?;
    verify(
        CHECK,
        first_projection.iter().any(|projection| {
            projection.kind == ImapFolderKind::Manual
                && projection.definition_identifier == Some(folder.identifier)
        }),
        format!("selected account is missing its manual projection: {first_projection:?}"),
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "list unselected account folders",
            storage.list_imap_local_folders(second.identifier).await,
        )?
        .is_empty(),
        "unselected accounts do not receive a Web projection",
    )?;

    let invalid_create = storage
        .create_web_folder(
            owner,
            "Rolled back",
            &[first.identifier, other.identifier],
            1,
        )
        .await;
    verify(
        CHECK,
        matches!(invalid_create, Err(StorageError::NotFound)),
        format!("cross-owner exposure must fail: {invalid_create:?}"),
    )?;
    verify_equal(
        CHECK,
        "failed create leaves the folder list unchanged",
        unwrap_storage(
            CHECK,
            "list after failed create",
            storage.list_web_folders(owner).await,
        )?
        .len(),
        1,
    )?;
    let invalid_update = storage
        .update_web_folder(folder.identifier, "Must roll back", &[other.identifier])
        .await;
    verify(
        CHECK,
        matches!(invalid_update, Err(StorageError::NotFound)),
        format!("cross-owner update must fail: {invalid_update:?}"),
    )?;
    let unchanged = unwrap_storage(
        CHECK,
        "load folder after failed update",
        storage.get_web_folder(folder.identifier).await,
    )?
    .ok_or_else(|| failure(CHECK, "folder disappeared after failed update"))?;
    verify(
        CHECK,
        unchanged.name == "Manual" && unchanged.exposed_accounts == [first.identifier],
        format!("failed update changed the definition: {unchanged:?}"),
    )?;
    let foreign_create = storage
        .create_web_folder(other.owner_identifier, "Foreign", &[first.identifier], 1)
        .await;
    verify(
        CHECK,
        matches!(foreign_create, Err(StorageError::NotFound)),
        format!("a user cannot expose another user's account: {foreign_create:?}"),
    )?;

    let manual_a = first_projection
        .iter()
        .find(|projection| projection.definition_identifier == Some(folder.identifier))
        .ok_or_else(|| failure(CHECK, "manual account projection missing"))?;
    let private_a = unwrap_storage(
        CHECK,
        "create first private folder",
        storage
            .create_imap_private_folder(first.identifier, "Private A", 1_700_000_002)
            .await,
    )?;
    let private_b = unwrap_storage(
        CHECK,
        "create second private folder",
        storage
            .create_imap_private_folder(first.identifier, "Private B", 1_700_000_003)
            .await,
    )?;
    let mut sources_a = vec![
        ImapMailboxSource::Inbox,
        ImapMailboxSource::Trash,
        ImapMailboxSource::LocalFolder(manual_a.identifier),
        ImapMailboxSource::LocalFolder(private_a.identifier),
        ImapMailboxSource::LocalFolder(private_b.identifier),
    ];
    let sources_b = [ImapMailboxSource::Inbox, ImapMailboxSource::Trash];
    let inbox_a =
        reconcile_imap_mailbox(storage, CHECK, &first, &sources_a, ImapMailboxSource::Inbox)
            .await?;
    let trash_a =
        reconcile_imap_mailbox(storage, CHECK, &first, &sources_a, ImapMailboxSource::Trash)
            .await?;
    let manual_mailbox_a = reconcile_imap_mailbox(
        storage,
        CHECK,
        &first,
        &sources_a,
        ImapMailboxSource::LocalFolder(manual_a.identifier),
    )
    .await?;
    let private_mailbox_a = reconcile_imap_mailbox(
        storage,
        CHECK,
        &first,
        &sources_a,
        ImapMailboxSource::LocalFolder(private_a.identifier),
    )
    .await?;
    let copy_mailbox_a = reconcile_imap_mailbox(
        storage,
        CHECK,
        &first,
        &sources_a,
        ImapMailboxSource::LocalFolder(private_b.identifier),
    )
    .await?;
    let inbox_b = reconcile_imap_mailbox(
        storage,
        CHECK,
        &second,
        &sources_b,
        ImapMailboxSource::Inbox,
    )
    .await?;
    let trash_b = reconcile_imap_mailbox(
        storage,
        CHECK,
        &second,
        &sources_b,
        ImapMailboxSource::Trash,
    )
    .await?;

    let web_message =
        insert_message_identifier_for_imap_check(storage, CHECK, &format!("{token} web")).await?;
    let fallback_message =
        insert_message_identifier_for_imap_check(storage, CHECK, &format!("{token} fallback"))
            .await?;
    let moved_message =
        insert_message_identifier_for_imap_check(storage, CHECK, &format!("{token} move")).await?;
    let copied_message =
        insert_message_identifier_for_imap_check(storage, CHECK, &format!("{token} copy")).await?;
    let trash_message =
        insert_message_identifier_for_imap_check(storage, CHECK, &format!("{token} trash")).await?;
    let deleted_message =
        insert_message_identifier_for_imap_check(storage, CHECK, &format!("{token} tombstone"))
            .await?;
    for mailbox in [
        inbox_a,
        manual_mailbox_a,
        private_mailbox_a,
        copy_mailbox_a,
        inbox_b,
    ] {
        synchronize_imap_mailbox(storage, CHECK, mailbox, &all).await?;
    }
    for mailbox in [trash_a, trash_b] {
        synchronize_imap_mailbox(storage, CHECK, mailbox, &all).await?;
    }

    let wrong_owner_placement = storage
        .place_user_messages(
            other.owner_identifier,
            &[web_message],
            Some(folder.identifier),
            &all.filter,
        )
        .await;
    verify(
        CHECK,
        matches!(wrong_owner_placement, Err(StorageError::NotFound)),
        format!("another user cannot place mail in this folder: {wrong_owner_placement:?}"),
    )?;
    let foreign_folder_query = storage
        .list_web_mailbox_messages(
            other.owner_identifier,
            Some(folder.identifier),
            web_mailbox_query("true"),
        )
        .await;
    verify(
        CHECK,
        matches!(foreign_folder_query, Err(StorageError::NotFound)),
        format!("cross-owner folder listing must fail: {foreign_folder_query:?}"),
    )?;
    verify_equal(
        CHECK,
        "placing Web messages reports the changed count",
        unwrap_storage(
            CHECK,
            "place Web messages",
            storage
                .place_user_messages(
                    owner,
                    &[web_message, fallback_message],
                    Some(folder.identifier),
                    &all.filter,
                )
                .await,
        )?,
        2,
    )?;
    verify_equal(
        CHECK,
        "placed messages leave the Web general mailbox",
        list_web_mailbox_message_identifiers(storage, CHECK, owner, None)
            .await?
            .contains(&web_message),
        false,
    )?;
    verify_equal(
        CHECK,
        "placed messages appear in their Web folder",
        list_web_mailbox_message_identifiers(storage, CHECK, owner, Some(folder.identifier))
            .await?
            .len(),
        2,
    )?;
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, manual_mailbox_a, web_message)
            .await?
            .is_none(),
        "the manual IMAP projection changes only after synchronization",
    )?;
    synchronize_imap_mailbox(storage, CHECK, manual_mailbox_a, &all).await?;
    synchronize_imap_mailbox(storage, CHECK, inbox_a, &all).await?;
    synchronize_imap_mailbox(storage, CHECK, inbox_b, &all).await?;
    let manual_uid =
        find_imap_mailbox_uid_for_message(storage, CHECK, manual_mailbox_a, web_message)
            .await?
            .ok_or_else(|| failure(CHECK, "exposed Web mail did not enter its manual mailbox"))?;
    let copied_to_inbox = unwrap_storage(
        CHECK,
        "copy Web mail into Inbox",
        storage
            .copy_imap_members(manual_mailbox_a, &[manual_uid], inbox_a, Some(&all), false)
            .await,
    )?;
    verify(
        CHECK,
        copied_to_inbox.all_new && copied_to_inbox.destination_uids.len() == 1,
        format!("Inbox COPY should create a fresh UID: {copied_to_inbox:?}"),
    )?;
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, manual_mailbox_a, web_message)
            .await?
            .is_some(),
        "COPY into Inbox preserves its source membership",
    )?;
    synchronize_imap_mailbox(storage, CHECK, inbox_a, &empty).await?;
    synchronize_imap_mailbox(storage, CHECK, inbox_a, &all).await?;
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, inbox_a, web_message)
            .await?
            .is_some(),
        "an Inbox COPY survives membership recalculation",
    )?;

    let copy_uid = find_imap_mailbox_uid_for_message(storage, CHECK, inbox_a, copied_message)
        .await?
        .ok_or_else(|| failure(CHECK, "copy source is absent from Inbox"))?;
    let copied = unwrap_storage(
        CHECK,
        "copy into account folder",
        storage
            .copy_imap_members(inbox_a, &[copy_uid], copy_mailbox_a, Some(&all), false)
            .await,
    )?;
    verify(
        CHECK,
        copied.all_new && copied.destination_uids[0] > 0,
        format!("COPY should assign a fresh destination UID: {copied:?}"),
    )?;
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, inbox_a, copied_message)
            .await?
            .is_some(),
        "COPY preserves source membership",
    )?;
    let duplicate_copy = storage
        .copy_imap_members(inbox_a, &[copy_uid], copy_mailbox_a, Some(&all), false)
        .await;
    verify(
        CHECK,
        matches!(duplicate_copy, Err(StorageError::ConstraintViolation(_))),
        format!("duplicate destination COPY is refused: {duplicate_copy:?}"),
    )?;
    verify_equal(
        CHECK,
        "duplicate refusal leaves destination UIDs unchanged",
        mailbox_member_uids(storage, CHECK, copy_mailbox_a).await?,
        copied.destination_uids.clone(),
    )?;

    let move_uid = find_imap_mailbox_uid_for_message(storage, CHECK, inbox_a, moved_message)
        .await?
        .ok_or_else(|| failure(CHECK, "move source is absent from Inbox"))?;
    unwrap_storage(
        CHECK,
        "copy message before moving it",
        storage
            .copy_imap_members(inbox_a, &[move_uid], copy_mailbox_a, Some(&all), false)
            .await,
    )?;
    let moved = unwrap_storage(
        CHECK,
        "move into private folder",
        storage
            .copy_imap_members(inbox_a, &[move_uid], private_mailbox_a, Some(&all), true)
            .await,
    )?;
    verify_equal(
        CHECK,
        "MOVE source UIDs",
        moved.removed_uids,
        vec![move_uid],
    )?;
    synchronize_imap_mailbox(storage, CHECK, inbox_a, &all).await?;
    synchronize_imap_mailbox(storage, CHECK, private_mailbox_a, &all).await?;
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, inbox_a, moved_message)
            .await?
            .is_none()
            && find_imap_mailbox_uid_for_message(storage, CHECK, private_mailbox_a, moved_message)
                .await?
                .is_some(),
        "MOVE changes only placement within the selected account",
    )?;
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, inbox_b, moved_message)
            .await?
            .is_some(),
        "another account's placement is unchanged",
    )?;
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, copy_mailbox_a, moved_message)
            .await?
            .is_some(),
        "MOVE preserves an unrelated COPY membership in the account",
    )?;
    verify(
        CHECK,
        list_web_mailbox_message_identifiers(storage, CHECK, owner, None)
            .await?
            .contains(&moved_message),
        "MOVE does not create a user deletion tombstone",
    )?;
    let first_move_uid =
        find_imap_mailbox_uid_for_message(storage, CHECK, private_mailbox_a, moved_message)
            .await?
            .unwrap();
    synchronize_imap_mailbox(storage, CHECK, private_mailbox_a, &empty).await?;
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, private_mailbox_a, moved_message)
            .await?
            .is_none(),
        "lost authorization removes the materialized override membership",
    )?;
    synchronize_imap_mailbox(storage, CHECK, private_mailbox_a, &all).await?;
    let restored_move_uid =
        find_imap_mailbox_uid_for_message(storage, CHECK, private_mailbox_a, moved_message)
            .await?
            .ok_or_else(|| failure(CHECK, "account placement did not survive scope regain"))?;
    verify(
        CHECK,
        restored_move_uid > first_move_uid
            && find_imap_mailbox_uid_for_message(storage, CHECK, inbox_a, moved_message)
                .await?
                .is_none(),
        "the account override survives false-to-true authorization without returning to Inbox",
    )?;

    let trash_uid = find_imap_mailbox_uid_for_message(storage, CHECK, inbox_a, trash_message)
        .await?
        .ok_or_else(|| failure(CHECK, "Trash test source is absent from Inbox"))?;
    let trash_move = unwrap_storage(
        CHECK,
        "move into Trash",
        storage
            .copy_imap_members(inbox_a, &[trash_uid], trash_a, Some(&all), true)
            .await,
    )?;
    verify_equal(
        CHECK,
        "Trash MOVE source UIDs",
        trash_move.removed_uids,
        vec![trash_uid],
    )?;
    synchronize_imap_mailbox(storage, CHECK, trash_a, &empty).await?;
    synchronize_imap_mailbox(storage, CHECK, trash_a, &all).await?;
    let trash_restored_uid =
        find_imap_mailbox_uid_for_message(storage, CHECK, trash_a, trash_message)
            .await?
            .ok_or_else(|| failure(CHECK, "Trash placement did not survive scope regain"))?;
    verify(
        CHECK,
        trash_restored_uid > trash_move.destination_uids[0]
            && find_imap_mailbox_uid_for_message(storage, CHECK, inbox_a, trash_message)
                .await?
                .is_none()
            && find_imap_mailbox_uid_for_message(storage, CHECK, inbox_b, trash_message)
                .await?
                .is_some(),
        "Trash placement restores only in its account after authorization returns",
    )?;

    let deleted_uid = find_imap_mailbox_uid_for_message(storage, CHECK, inbox_a, deleted_message)
        .await?
        .ok_or_else(|| failure(CHECK, "tombstone test message is absent from Inbox"))?;
    unwrap_storage(
        CHECK,
        "flag message deleted",
        storage
            .store_imap_flags(
                inbox_a,
                &[deleted_uid],
                &ImapFlagChange {
                    operation: ImapFlagOperation::Add,
                    flags: ImapMessageFlags {
                        deleted: true,
                        ..ImapMessageFlags::default()
                    },
                },
            )
            .await,
    )?;
    verify_equal(
        CHECK,
        "expunged UIDs",
        unwrap_storage(
            CHECK,
            "expunge deleted message",
            storage
                .expunge_imap_members(inbox_a, Some(&[deleted_uid]))
                .await,
        )?,
        vec![deleted_uid],
    )?;
    unwrap_storage(
        CHECK,
        "clear account A exclusions",
        storage.clear_imap_exclusions(first.identifier).await,
    )?;
    unwrap_storage(
        CHECK,
        "clear account B exclusions",
        storage.clear_imap_exclusions(second.identifier).await,
    )?;
    for mailbox in [inbox_a, inbox_b] {
        synchronize_imap_mailbox(storage, CHECK, mailbox, &all).await?;
        verify(
            CHECK,
            find_imap_mailbox_uid_for_message(storage, CHECK, mailbox, deleted_message)
                .await?
                .is_none(),
            "a tombstoned message stays hidden after restore and synchronization",
        )?;
    }
    verify(
        CHECK,
        !list_web_mailbox_message_identifiers(storage, CHECK, owner, None)
            .await?
            .contains(&deleted_message),
        "IMAP exclusion restore never clears a user tombstone",
    )?;

    let renamed = unwrap_storage(
        CHECK,
        "rename folder and change selected accounts",
        storage
            .update_web_folder(folder.identifier, "Renamed", &[second.identifier])
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "folder update returned no row"))?;
    verify(
        CHECK,
        renamed.name == "Renamed" && renamed.exposed_accounts == [second.identifier],
        format!("folder update did not replace its exposure: {renamed:?}"),
    )?;
    sources_a.retain(|source| *source != ImapMailboxSource::LocalFolder(manual_a.identifier));
    let inbox_a_state = unwrap_storage(
        CHECK,
        "reconcile account A after exposure removal",
        storage
            .reconcile_imap_mailboxes(first.identifier, &sources_a)
            .await,
    )?;
    let inbox_a = inbox_a_state
        .iter()
        .find(|state| state.source == ImapMailboxSource::Inbox)
        .ok_or_else(|| failure(CHECK, "account A Inbox disappeared"))?
        .identifier;
    synchronize_imap_mailbox(storage, CHECK, inbox_a, &all).await?;
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, inbox_a, fallback_message)
            .await?
            .is_some(),
        "removing exposure returns Web-placed mail to account A's general fallback",
    )?;
    let projection_b = unwrap_storage(
        CHECK,
        "find newly exposed projection",
        storage.list_imap_local_folders(second.identifier).await,
    )?
    .into_iter()
    .find(|projection| projection.definition_identifier == Some(folder.identifier))
    .ok_or_else(|| failure(CHECK, "newly selected account lacks its folder projection"))?;
    let mut sources_b = vec![
        ImapMailboxSource::Inbox,
        ImapMailboxSource::Trash,
        ImapMailboxSource::LocalFolder(projection_b.identifier),
    ];
    let inbox_b_state = unwrap_storage(
        CHECK,
        "reconcile account B after exposure change",
        storage
            .reconcile_imap_mailboxes(second.identifier, &sources_b)
            .await,
    )?;
    let inbox_b = inbox_b_state
        .iter()
        .find(|state| state.source == ImapMailboxSource::Inbox)
        .unwrap()
        .identifier;
    let manual_mailbox_b = inbox_b_state
        .iter()
        .find(|state| state.source == ImapMailboxSource::LocalFolder(projection_b.identifier))
        .unwrap()
        .identifier;
    synchronize_imap_mailbox(storage, CHECK, inbox_b, &all).await?;
    synchronize_imap_mailbox(storage, CHECK, manual_mailbox_b, &all).await?;
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, inbox_b, fallback_message)
            .await?
            .is_none()
            && find_imap_mailbox_uid_for_message(
                storage,
                CHECK,
                manual_mailbox_b,
                fallback_message,
            )
            .await?
            .is_some(),
        "only the selected account inherits Web placement",
    )?;

    let unexposed = unwrap_storage(
        CHECK,
        "remove all folder exposures",
        storage
            .update_web_folder(folder.identifier, "Renamed", &[])
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "folder disappeared when exposures were removed"))?;
    verify(
        CHECK,
        unexposed.exposed_accounts.is_empty(),
        "exposures were not removed",
    )?;
    sources_b.retain(|source| *source != ImapMailboxSource::LocalFolder(projection_b.identifier));
    let inbox_b_state = unwrap_storage(
        CHECK,
        "reconcile account B after removing exposure",
        storage
            .reconcile_imap_mailboxes(second.identifier, &sources_b)
            .await,
    )?;
    let inbox_b = inbox_b_state[0].identifier;
    synchronize_imap_mailbox(storage, CHECK, inbox_b, &all).await?;
    verify(
        CHECK,
        find_imap_mailbox_uid_for_message(storage, CHECK, inbox_b, fallback_message)
            .await?
            .is_some(),
        "removing the final exposure returns placed mail to Inbox fallback",
    )?;

    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "delete Web folder",
            storage.delete_web_folder(folder.identifier).await,
        )?,
        "an owned Web folder can be deleted",
    )?;
    verify(
        CHECK,
        list_web_mailbox_message_identifiers(storage, CHECK, owner, None)
            .await?
            .contains(&web_message),
        "folder deletion clears Web placement and restores the general mailbox",
    )?;

    for account in [first.identifier, second.identifier] {
        unwrap_storage(
            CHECK,
            "delete conformance account",
            storage.delete_imap_account(account).await,
        )?;
    }
    unwrap_storage(
        CHECK,
        "delete foreign conformance account",
        storage.delete_imap_account(other.identifier).await,
    )?;
    Ok(())
}
