//! Shared fixtures for IMAP storage contract checks.
use sandpost_core::{
    GlobalRole, ImapAccount, ImapMailboxIdentifier, ImapMailboxSource, MessageSequence,
};
use sandpost_storage::{ImapMembershipDefinition, NewImapAccount, Storage};

use crate::ConformanceFailure;
use crate::support::{
    create_user, failure, simple_message, unique_email, unique_token, unwrap_storage,
};

/// Build a membership definition from query source, fingerprinted by the source text itself.
pub(super) fn membership_definition(source: &str) -> ImapMembershipDefinition {
    ImapMembershipDefinition {
        filter: sandpost_query::compile(source)
            .expect("conformance definitions are valid")
            .into_expression(),
        fingerprint: format!("conformance:{source}"),
    }
}

/// Create a member user with one IMAP account and return the account.
pub(super) async fn create_imap_account_fixture(
    storage: &dyn Storage,
    check: &'static str,
) -> Result<ImapAccount, ConformanceFailure> {
    let owner = create_user(storage, check, GlobalRole::Member, &unique_email(check)).await?;
    unwrap_storage(
        check,
        "create_imap_account",
        storage
            .create_imap_account(&NewImapAccount {
                owner_identifier: owner.identifier,
                name: "Conformance IMAP".to_owned(),
                username: format!("imap-{}-{}", std::process::id(), unique_token()),
                password_hash: "hash-one".to_owned(),
                mirror_views: true,
                created_at: 1_700_000_000,
            })
            .await,
    )
}

/// Insert a message with a subject and raw bytes and return its sequence.
pub(super) async fn insert_message_for_imap_check(
    storage: &dyn Storage,
    check: &'static str,
    subject: &str,
) -> Result<MessageSequence, ConformanceFailure> {
    let mut message = simple_message(subject);
    message.raw_message = format!("Subject: {subject}\r\n\r\nbody\r\n").into_bytes();
    message.facts.size = message.raw_message.len() as u64;
    message.facts.received_at = 1_700_000_100;
    unwrap_storage(
        check,
        "insert_message",
        storage.insert_message(&message).await,
    )
}

/// Reconcile and return the identifier of one source's mailbox.
pub(super) async fn reconcile_imap_mailbox(
    storage: &dyn Storage,
    check: &'static str,
    account: &ImapAccount,
    sources: &[ImapMailboxSource],
    wanted: ImapMailboxSource,
) -> Result<ImapMailboxIdentifier, ConformanceFailure> {
    let states = unwrap_storage(
        check,
        "reconcile_imap_mailboxes",
        storage
            .reconcile_imap_mailboxes(account.identifier, sources)
            .await,
    )?;
    states
        .into_iter()
        .find(|state| state.source == wanted)
        .map(|state| state.identifier)
        .ok_or_else(|| failure(check, "reconciled mailbox is missing"))
}

/// Return the member UIDs of a mailbox.
pub(super) async fn mailbox_member_uids(
    storage: &dyn Storage,
    check: &'static str,
    mailbox: ImapMailboxIdentifier,
) -> Result<Vec<u32>, ConformanceFailure> {
    unwrap_storage(
        check,
        "imap_mailbox_uids",
        storage.imap_mailbox_uids(mailbox, u32::MAX).await,
    )
}
