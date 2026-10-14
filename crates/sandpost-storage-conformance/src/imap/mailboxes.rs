//! IMAP mailbox identity and membership synchronization checks.
use sandpost_core::ImapMailboxSource;
use sandpost_storage::Storage;

use super::support::{
    create_imap_account_fixture, insert_message_for_imap_check, mailbox_member_uids,
    membership_definition, reconcile_imap_mailbox,
};
use crate::ConformanceFailure;
use crate::support::{failure, unique_token, unwrap_storage, verify, verify_equal};

/// Verify mailbox identities: stable on repeat, unique ascending UIDVALIDITY, removed with their
/// source, and never reusing UIDVALIDITY when recreated.
pub async fn imap_mailbox_identities(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "imap_mailbox_identities";
    let account = create_imap_account_fixture(storage, CHECK).await?;
    let sources = [ImapMailboxSource::Inbox, ImapMailboxSource::Trash];
    let first = unwrap_storage(
        CHECK,
        "reconcile_imap_mailboxes",
        storage
            .reconcile_imap_mailboxes(account.identifier, &sources, None)
            .await,
    )?;
    verify_equal(
        CHECK,
        "reconciled sources",
        first.iter().map(|state| state.source).collect::<Vec<_>>(),
        sources.to_vec(),
    )?;
    verify(
        CHECK,
        first[0].uid_validity != first[1].uid_validity
            && first.iter().all(|state| state.uid_next == 1),
        format!("identities need distinct UIDVALIDITY and start at UID 1: {first:?}"),
    )?;
    let again = unwrap_storage(
        CHECK,
        "reconcile_imap_mailboxes again",
        storage
            .reconcile_imap_mailboxes(account.identifier, &sources, None)
            .await,
    )?;
    verify_equal(CHECK, "repeated reconciliation", again, first.clone())?;
    let only_inbox = unwrap_storage(
        CHECK,
        "reconcile without trash",
        storage
            .reconcile_imap_mailboxes(account.identifier, &[ImapMailboxSource::Inbox], None)
            .await,
    )?;
    verify_equal(CHECK, "one reconciled mailbox", only_inbox.len(), 1)?;
    verify_equal(CHECK, "kept inbox", only_inbox[0], first[0])?;
    verify_equal(
        CHECK,
        "omitted valid source retains its identity",
        unwrap_storage(
            CHECK,
            "get omitted trash",
            storage.get_imap_mailbox(first[1].identifier).await,
        )?,
        Some(first[1]),
    )?;
    let restored = unwrap_storage(
        CHECK,
        "restore omitted catalog source",
        storage
            .reconcile_imap_mailboxes(account.identifier, &sources, None)
            .await,
    )?;
    verify_equal(
        CHECK,
        "restored source keeps UIDVALIDITY",
        restored[1],
        first[1],
    )?;

    // Genuine definition deletion, rather than a stale plan, must remove the mailbox identity.
    let folder_name = format!("Identity {}", unique_token());
    let folder = unwrap_storage(
        CHECK,
        "create identity folder",
        storage
            .create_imap_private_folder(account.identifier, &folder_name, 1_700_000_001, None)
            .await,
    )?;
    let folder_sources = [
        ImapMailboxSource::Inbox,
        ImapMailboxSource::Trash,
        ImapMailboxSource::LocalFolder(folder.identifier),
    ];
    let folder_states = unwrap_storage(
        CHECK,
        "reconcile identity folder",
        storage
            .reconcile_imap_mailboxes(account.identifier, &folder_sources, None)
            .await,
    )?;
    let original = folder_states[2];
    unwrap_storage(
        CHECK,
        "delete identity folder",
        storage
            .delete_imap_local_folder(folder.identifier, None)
            .await,
    )?;
    verify(
        CHECK,
        unwrap_storage(
            CHECK,
            "get deleted folder identity",
            storage.get_imap_mailbox(original.identifier).await,
        )?
        .is_none(),
        "actual source deletion removes its identity",
    )?;
    let replacement = unwrap_storage(
        CHECK,
        "recreate identity folder",
        storage
            .create_imap_private_folder(account.identifier, &folder_name, 1_700_000_002, None)
            .await,
    )?;
    let replacement_sources = [
        ImapMailboxSource::Inbox,
        ImapMailboxSource::Trash,
        ImapMailboxSource::LocalFolder(replacement.identifier),
    ];
    let recreated = unwrap_storage(
        CHECK,
        "reconcile recreated folder",
        storage
            .reconcile_imap_mailboxes(account.identifier, &replacement_sources, None)
            .await,
    )?;
    verify(
        CHECK,
        recreated[2].identifier != original.identifier
            && recreated[2].uid_validity > original.uid_validity.max(first[1].uid_validity),
        format!("a genuinely recreated mailbox needs a fresh UIDVALIDITY: {recreated:?}"),
    )?;
    unwrap_storage(
        CHECK,
        "delete_imap_account",
        storage.delete_imap_account(account.identifier).await,
    )?;
    Ok(())
}

/// Verify membership reconciliation: ascending UIDs in message order, incremental additions,
/// full reconciliation on a changed definition without renumbering, removal with deleted mail,
/// and that explicit mailboxes only lose members.
pub async fn imap_membership_synchronization(
    storage: &dyn Storage,
) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "imap_membership_synchronization";
    let token = format!("membership-{}-{}", std::process::id(), unique_token());
    let account = create_imap_account_fixture(storage, CHECK).await?;
    let sources = [ImapMailboxSource::Inbox, ImapMailboxSource::Trash];
    let inbox =
        reconcile_imap_mailbox(storage, CHECK, &account, &sources, ImapMailboxSource::Inbox)
            .await?;
    let trash =
        reconcile_imap_mailbox(storage, CHECK, &account, &sources, ImapMailboxSource::Trash)
            .await?;
    insert_message_for_imap_check(storage, CHECK, &format!("{token} alpha")).await?;
    insert_message_for_imap_check(storage, CHECK, &format!("{token} beta")).await?;
    insert_message_for_imap_check(storage, CHECK, "unrelated").await?;
    let alpha_or_beta = membership_definition(&format!("subject contains \"{token}\""));
    let state = unwrap_storage(
        CHECK,
        "synchronize_imap_mailbox",
        storage
            .synchronize_imap_mailbox(inbox, &alpha_or_beta)
            .await,
    )?
    .ok_or_else(|| failure(CHECK, "mailbox missing"))?;
    verify_equal(CHECK, "uid_next after first sync", state.uid_next, 3)?;
    let members = unwrap_storage(
        CHECK,
        "imap_mailbox_members",
        storage.imap_mailbox_members(inbox, 0).await,
    )?;
    verify_equal(
        CHECK,
        "member UIDs",
        members.iter().map(|member| member.uid).collect::<Vec<_>>(),
        vec![1, 2],
    )?;
    let gamma = insert_message_for_imap_check(storage, CHECK, &format!("{token} gamma")).await?;
    unwrap_storage(
        CHECK,
        "incremental synchronize",
        storage
            .synchronize_imap_mailbox(inbox, &alpha_or_beta)
            .await,
    )?;
    verify_equal(
        CHECK,
        "incremental UIDs",
        mailbox_member_uids(storage, CHECK, inbox).await?,
        vec![1, 2, 3],
    )?;
    let without_beta = membership_definition(&format!(
        "subject contains \"{token}\" and not (subject contains \"beta\")"
    ));
    unwrap_storage(
        CHECK,
        "full synchronize",
        storage.synchronize_imap_mailbox(inbox, &without_beta).await,
    )?;
    verify_equal(
        CHECK,
        "UIDs after narrowing",
        mailbox_member_uids(storage, CHECK, inbox).await?,
        vec![1, 3],
    )?;
    unwrap_storage(
        CHECK,
        "widen again",
        storage
            .synchronize_imap_mailbox(inbox, &alpha_or_beta)
            .await,
    )?;
    verify_equal(
        CHECK,
        "a message that re-enters receives a new UID; others keep theirs",
        mailbox_member_uids(storage, CHECK, inbox).await?,
        vec![1, 3, 4],
    )?;
    let deleted_identifier = unwrap_storage(
        CHECK,
        "imap_member_content",
        storage.imap_member_content(inbox, 3).await,
    )?
    .ok_or_else(|| failure(CHECK, "member content missing"))?
    .message_identifier;
    verify_equal(
        CHECK,
        "content sequence",
        unwrap_storage(
            CHECK,
            "indexed_messages",
            storage.indexed_messages(&[deleted_identifier]).await,
        )?
        .pop()
        .map(|message| message.sequence),
        Some(gamma),
    )?;
    unwrap_storage(
        CHECK,
        "delete_message",
        storage.delete_message(deleted_identifier).await,
    )?;
    verify_equal(
        CHECK,
        "deleted mail leaves the mailbox",
        mailbox_member_uids(storage, CHECK, inbox).await?,
        vec![1, 4],
    )?;
    unwrap_storage(
        CHECK,
        "synchronize trash",
        storage
            .synchronize_imap_mailbox(trash, &alpha_or_beta)
            .await,
    )?;
    verify(
        CHECK,
        mailbox_member_uids(storage, CHECK, trash).await?.is_empty(),
        "an explicit mailbox gains nothing from its definition",
    )?;
    let copied = unwrap_storage(
        CHECK,
        "copy_imap_members",
        storage
            .copy_imap_members(inbox, &[1, 4], trash, Some(&alpha_or_beta), false)
            .await,
    )?;
    verify(
        CHECK,
        copied.source_uids == [1, 4] && copied.destination_uids == [1, 2] && copied.all_new,
        format!("copy result: {copied:?}"),
    )?;
    unwrap_storage(
        CHECK,
        "synchronize trash with false",
        storage
            .synchronize_imap_mailbox(trash, &membership_definition("false"))
            .await,
    )?;
    verify(
        CHECK,
        mailbox_member_uids(storage, CHECK, trash).await?.is_empty(),
        "an explicit mailbox loses members its definition no longer allows",
    )?;
    unwrap_storage(
        CHECK,
        "delete_imap_account",
        storage.delete_imap_account(account.identifier).await,
    )?;
    Ok(())
}
