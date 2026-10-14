//! Direct session operations enforce handle ownership, authorization, and current membership.

use super::*;
use crate::imap::ImapMailboxHandle;
use sandpost_core::ImapMailboxSource;

/// Request a flag whose absence can demonstrate that a refused write had no side effects.
fn flagged_change() -> ImapFlagChange {
    ImapFlagChange {
        operation: ImapFlagOperation::Add,
        flags: ImapMessageFlags {
            flagged: true,
            ..ImapMessageFlags::default()
        },
    }
}

/// Assert every member operation refuses a foreign or source-inconsistent handle.
async fn assert_handle_rejected(
    session: &ImapAccountSession,
    handle: &ImapMailboxHandle,
    uid: u32,
) {
    assert!(matches!(
        session.content(handle, uid).await,
        Err(ImapAccessError::NonExistent)
    ));
    assert!(matches!(
        session.metadata(handle, &[uid]).await,
        Err(ImapAccessError::NonExistent)
    ));
    assert!(matches!(
        session.store(handle, &[uid], &flagged_change()).await,
        Err(ImapAccessError::NonExistent)
    ));
    assert!(matches!(
        session.copy(handle, &[uid], "INBOX", false).await,
        Err(ImapAccessError::NonExistent)
    ));
    assert!(matches!(
        session.expunge(handle, None).await,
        Err(ImapAccessError::NonExistent)
    ));
}

/// Assert every member operation revalidates the credentials rather than trusting an old handle.
async fn assert_member_operations_revoked(
    session: &ImapAccountSession,
    handle: &ImapMailboxHandle,
    uid: u32,
) {
    assert!(matches!(
        session.content(handle, uid).await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(matches!(
        session.metadata(handle, &[uid]).await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(matches!(
        session.store(handle, &[uid], &flagged_change()).await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(matches!(
        session.copy(handle, &[uid], "INBOX", false).await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(matches!(
        session.expunge(handle, None).await,
        Err(ImapAccessError::Revoked)
    ));
}

/// Handles from another user or another account of the same user cannot read or change flags.
#[tokio::test]
async fn foreign_account_handles_reject_member_operations_without_changing_flags() {
    for mode in [ImapAccountMode::Mirror, ImapAccountMode::Independent] {
        let fixture = Fixture::new().await;
        let owner = fixture
            .user("Owner", GlobalRole::Member, MailAccess::All)
            .await;
        let other = fixture
            .user("Other", GlobalRole::Member, MailAccess::Scoped)
            .await;
        fixture.mail("Private message", "private.test").await;
        let issued = fixture.account(&owner, mode).await;
        let owner_session = fixture.login(&issued).await;
        let opened = owner_session.open("INBOX", false).await.unwrap();
        let uid = opened.members[0].0;
        for user in [&owner, &other] {
            let foreign_account = fixture.account(user, mode).await;
            let foreign_session = fixture.login(&foreign_account).await;
            assert_handle_rejected(&foreign_session, &opened.handle, uid).await;
        }
        let unchanged = owner_session.open("INBOX", false).await.unwrap();
        assert_eq!(unchanged.cursor, opened.cursor);
        assert!(!unchanged.members[0].1.flagged);
    }
}

/// A correct mailbox identifier with a different source is rejected rather than trusted.
#[tokio::test]
async fn mismatched_handle_sources_reject_member_operations() {
    let fixture = Fixture::new().await;
    let owner = fixture
        .user("Owner", GlobalRole::Member, MailAccess::All)
        .await;
    fixture.mail("Message", "example.test").await;
    let issued = fixture.account(&owner, ImapAccountMode::Mirror).await;
    let session = fixture.login(&issued).await;
    let opened = session.open("INBOX", false).await.unwrap();
    let inconsistent = ImapMailboxHandle {
        source: ImapMailboxSource::Trash,
        ..opened.handle
    };
    assert_handle_rejected(&session, &inconsistent, opened.members[0].0).await;
    let unchanged = session.open("INBOX", false).await.unwrap();
    assert_eq!(unchanged.cursor, opened.cursor);
    assert!(!unchanged.members[0].1.flagged);
}

/// Each member operation independently reconciles stale membership after scope revocation.
#[tokio::test]
async fn revoked_scope_blocks_direct_content_metadata_and_flag_writes() {
    for mode in [ImapAccountMode::Mirror, ImapAccountMode::Independent] {
        let fixture = Fixture::new().await;
        let owner = fixture
            .user("Owner", GlobalRole::Member, MailAccess::Scoped)
            .await;
        let scope = fixture.scope(&owner, "true").await;
        fixture.mail("Message", "example.test").await;
        let issued = fixture.account(&owner, mode).await;
        let session = fixture.login(&issued).await;

        let opened = session.open("INBOX", false).await.unwrap();
        assert!(
            session
                .content(&opened.handle, opened.members[0].0)
                .await
                .unwrap()
                .is_some()
        );
        fixture
            .storage
            .remove_user_from_scope(owner.identifier, scope)
            .await
            .unwrap();
        assert!(
            session
                .content(&opened.handle, opened.members[0].0)
                .await
                .unwrap()
                .is_none()
        );

        fixture
            .storage
            .assign_user_to_scope(owner.identifier, scope)
            .await
            .unwrap();
        let opened = session.open("INBOX", false).await.unwrap();
        assert_eq!(
            session
                .metadata(&opened.handle, &[opened.members[0].0])
                .await
                .unwrap()
                .len(),
            1
        );
        fixture
            .storage
            .remove_user_from_scope(owner.identifier, scope)
            .await
            .unwrap();
        assert!(
            session
                .metadata(&opened.handle, &[opened.members[0].0])
                .await
                .unwrap()
                .is_empty()
        );

        fixture
            .storage
            .assign_user_to_scope(owner.identifier, scope)
            .await
            .unwrap();
        let opened = session.open("INBOX", false).await.unwrap();
        session
            .store(
                &opened.handle,
                &[opened.members[0].0],
                &ImapFlagChange {
                    operation: ImapFlagOperation::Add,
                    flags: ImapMessageFlags {
                        deleted: true,
                        ..Default::default()
                    },
                },
            )
            .await
            .unwrap();
        fixture
            .storage
            .remove_user_from_scope(owner.identifier, scope)
            .await
            .unwrap();
        assert!(
            session
                .store(&opened.handle, &[opened.members[0].0], &flagged_change())
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            session
                .expunge(&opened.handle, None)
                .await
                .unwrap()
                .is_empty()
        );
        assert!(
            session
                .copy(&opened.handle, &[opened.members[0].0], "Trash", false)
                .await
                .unwrap()
                .source_uids
                .is_empty()
        );
        fixture
            .storage
            .assign_user_to_scope(owner.identifier, scope)
            .await
            .unwrap();
        let restored = session.open("INBOX", false).await.unwrap();
        assert!(!restored.members[0].1.flagged);
        let updated = session
            .store(
                &restored.handle,
                &[restored.members[0].0],
                &flagged_change(),
            )
            .await
            .unwrap();
        assert_eq!(updated.len(), 1);
        assert!(updated[0].1.flagged);
    }
}

/// Disabling an account and rotating its password revoke direct member operations on old handles.
#[tokio::test]
async fn disabled_accounts_and_rotated_credentials_revoke_member_operations() {
    let fixture = Fixture::new().await;
    let owner = fixture
        .user("Owner", GlobalRole::Member, MailAccess::All)
        .await;
    fixture.mail("Message", "example.test").await;
    let issued = fixture.account(&owner, ImapAccountMode::Mirror).await;
    let account_identifier = issued.account.account.identifier;
    let caller = fixture.as_user(&owner);
    let session = fixture.login(&issued).await;
    let opened = session.open("INBOX", false).await.unwrap();
    caller
        .update_imap_account(
            account_identifier,
            ImapAccountUpdate {
                enabled: Some(false),
                ..ImapAccountUpdate::default()
            },
        )
        .await
        .unwrap();
    assert_member_operations_revoked(&session, &opened.handle, opened.members[0].0).await;

    caller
        .update_imap_account(
            account_identifier,
            ImapAccountUpdate {
                enabled: Some(true),
                ..ImapAccountUpdate::default()
            },
        )
        .await
        .unwrap();
    let session = fixture.login(&issued).await;
    let opened = session.open("INBOX", false).await.unwrap();
    caller
        .regenerate_imap_account_password(account_identifier)
        .await
        .unwrap();
    assert_member_operations_revoked(&session, &opened.handle, opened.members[0].0).await;
}

/// COPY reconciles a stale, unopened destination and still refuses a same-mailbox MOVE.
#[tokio::test]
async fn copy_reconciles_unopened_destination_before_duplicate_check() {
    let fixture = Fixture::new().await;
    let owner = fixture
        .user("Owner", GlobalRole::Member, MailAccess::All)
        .await;
    fixture.mail("Message", "example.test").await;
    let issued = fixture.account(&owner, ImapAccountMode::Mirror).await;
    let session = fixture.login(&issued).await;
    let sibling = fixture
        .login(&fixture.account(&owner, ImapAccountMode::Mirror).await)
        .await;
    session.create_mailbox("Moved").await.unwrap();
    session.create_mailbox("Independent Copy").await.unwrap();
    let inbox = session.open("INBOX", false).await.unwrap();
    session
        .copy(
            &inbox.handle,
            &[inbox.members[0].0],
            "Independent Copy",
            false,
        )
        .await
        .unwrap();

    // A sibling Mirror credential moves the shared primary occurrence, leaving this session's
    // materialized INBOX stale until the COPY destination is reconciled.
    let baseline = sibling.open("INBOX", false).await.unwrap();
    sibling
        .copy(&baseline.handle, &[baseline.members[0].0], "Moved", true)
        .await
        .unwrap();
    let stale = fixture
        .storage
        .imap_mailbox_members(inbox.handle.identifier, 0)
        .await
        .unwrap();
    assert_eq!(stale.len(), 1);
    assert_eq!(stale[0].uid, inbox.members[0].0);

    let independent = session.open("Independent Copy", false).await.unwrap();
    let copied = session
        .copy(
            &independent.handle,
            &[independent.members[0].0],
            "INBOX",
            false,
        )
        .await
        .unwrap();
    assert_eq!(copied.uid_validity, inbox.uid_validity);
    assert_eq!(copied.source_uids, [independent.members[0].0]);
    assert_eq!(copied.destination_uids.len(), 1);
    assert!(copied.destination_uids[0] > inbox.members[0].0);
    let restored = session.open("INBOX", false).await.unwrap();
    assert_eq!(restored.members.len(), 1);
    assert_eq!(restored.members[0].0, copied.destination_uids[0]);
    assert_eq!(session.open("Moved", false).await.unwrap().members.len(), 1);
    assert_eq!(
        session
            .open("Independent Copy", false)
            .await
            .unwrap()
            .members
            .len(),
        1
    );
    assert!(matches!(
        session.copy(&restored.handle, &[restored.members[0].0], "INBOX", true).await,
        Err(ImapAccessError::NotPermitted(message)) if message == "destination already contains a message"
    ));
}
