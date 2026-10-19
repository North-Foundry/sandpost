//! Shared Mirror organization, Independent isolation, and occurrence lifecycles through services.
use super::*;
use crate::{QueryOptions, WebFolderRequest};

/// Return sorted Web mailbox subjects for comparison with IMAP.
async fn web_subjects(
    caller: &crate::AuthorizedApplication,
    folder: Option<sandpost_core::ImapFolderIdentifier>,
) -> Vec<String> {
    let mut subjects: Vec<_> = caller
        .query_web_mailbox(QueryOptions::default(), folder)
        .await
        .unwrap()
        .into_iter()
        .map(|message| message.subject)
        .collect();
    subjects.sort();
    subjects
}

/// Independent filing keeps smart results and other views intact; deleting the last folder trashes mail.
#[tokio::test]
async fn private_moves_survive_permissions_and_folder_deletion_trashes_last_occurrence() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Member", GlobalRole::Member, MailAccess::Scoped)
        .await;
    let scope = fixture.scope(&user, "true").await;
    let message = fixture.mail("Production error", "app.test").await;
    fixture.view(&user, "Primary View", "true").await;
    let first = fixture.account(&user, ImapAccountMode::Independent).await;
    let second = fixture.account(&user, ImapAccountMode::Independent).await;
    let first_session = fixture.login(&first).await;
    let second_session = fixture.login(&second).await;
    let caller = fixture.as_user(&user);
    let smart = caller
        .create_imap_folder(
            first.account.account.identifier,
            ImapFolderRequest {
                name: "Laravel".into(),
                filter: "subject contains \"Production\"".into(),
                include_in_inbox: false,
            },
        )
        .await
        .unwrap();
    first_session.create_mailbox("To Review").await.unwrap();
    let source = first_session.open("INBOX", false).await.unwrap();
    let moved = first_session
        .copy(&source.handle, &[source.members[0].0], "To Review", true)
        .await
        .unwrap();
    assert_eq!(moved.source_uids, [source.members[0].0]);
    assert_eq!(
        subjects(&fixture, &first_session, "To Review").await,
        ["Production error"]
    );
    assert_eq!(
        subjects(&fixture, &first_session, "Laravel").await,
        ["Production error"]
    );
    assert_eq!(
        subjects(&fixture, &second_session, "INBOX").await,
        ["Production error"]
    );
    assert_eq!(web_subjects(&caller, None).await, ["Production error"]);
    assert!(caller.get_message(message).await.is_ok());
    caller
        .update_imap_folder(
            first.account.account.identifier,
            smart.identifier,
            ImapFolderRequest {
                name: "Laravel".into(),
                filter: "true".into(),
                include_in_inbox: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        subjects(&fixture, &first_session, "Laravel").await,
        ["Production error"]
    );
    fixture
        .storage
        .remove_user_from_scope(user.identifier, scope)
        .await
        .unwrap();
    assert!(
        subjects(&fixture, &first_session, "To Review")
            .await
            .is_empty()
    );
    assert!(
        subjects(&fixture, &first_session, "Laravel")
            .await
            .is_empty()
    );
    fixture
        .storage
        .assign_user_to_scope(user.identifier, scope)
        .await
        .unwrap();
    assert_eq!(
        subjects(&fixture, &first_session, "To Review").await,
        ["Production error"]
    );
    first_session.delete_mailbox("To Review").await.unwrap();
    assert!(subjects(&fixture, &first_session, "INBOX").await.is_empty());
    assert!(
        subjects(&fixture, &first_session, "Laravel")
            .await
            .is_empty()
    );
    assert_eq!(
        subjects(&fixture, &first_session, "Trash").await,
        ["Production error"]
    );
    assert_eq!(web_subjects(&caller, None).await, ["Production error"]);
}

/// Mirror follows primary folders and placements; Independent keeps its creation snapshot.
#[tokio::test]
async fn mirror_folder_management_is_shared_without_live_independent_inheritance() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    let message = fixture.mail("Production error", "app.test").await;
    fixture.view(&user, "Laravel", "true").await;
    let first = fixture.account(&user, ImapAccountMode::Mirror).await;
    let second = fixture.account(&user, ImapAccountMode::Independent).await;
    let first_session = fixture.login(&first).await;
    let second_session = fixture.login(&second).await;
    let caller = fixture.as_user(&user);
    let folder = caller
        .create_web_folder(WebFolderRequest {
            name: "Web Review".into(),
            icon: None,
            exposed_accounts: vec![],
        })
        .await
        .unwrap();
    let before = first_session.open("Web Review", false).await.unwrap();
    assert!(matches!(
        second_session.open("Web Review", false).await,
        Err(ImapAccessError::NonExistent)
    ));
    caller
        .move_web_messages(vec![message], Some(folder.identifier))
        .await
        .unwrap();
    assert_eq!(
        subjects(&fixture, &first_session, "Web Review").await,
        ["Production error"]
    );
    assert_eq!(
        subjects(&fixture, &first_session, "Laravel").await,
        ["Production error"]
    );
    assert_eq!(
        subjects(&fixture, &second_session, "INBOX").await,
        ["Production error"]
    );
    first_session
        .rename_mailbox("Web Review", "Renamed")
        .await
        .unwrap();
    let renamed = first_session.open("Renamed", false).await.unwrap();
    assert_eq!(renamed.uid_validity, before.uid_validity);
    assert_eq!(caller.list_web_folders().await.unwrap()[0].name, "Renamed");
    first_session.create_mailbox("Shared").await.unwrap();
    let shared = caller
        .list_web_folders()
        .await
        .unwrap()
        .into_iter()
        .find(|folder| folder.name == "Shared")
        .unwrap();
    first_session
        .copy(&renamed.handle, &[renamed.members[0].0], "Shared", true)
        .await
        .unwrap();
    assert_eq!(
        web_subjects(&caller, Some(shared.identifier)).await,
        ["Production error"]
    );
    assert!(
        web_subjects(&caller, Some(folder.identifier))
            .await
            .is_empty()
    );
    caller.move_web_messages(vec![message], None).await.unwrap();
    assert!(
        subjects(&fixture, &first_session, "Shared")
            .await
            .is_empty()
    );
    assert_eq!(
        subjects(&fixture, &first_session, "INBOX").await,
        ["Production error"]
    );
    caller
        .move_web_messages(vec![message], Some(folder.identifier))
        .await
        .unwrap();
    first_session.delete_mailbox("Renamed").await.unwrap();
    assert!(
        fixture
            .storage
            .get_web_folder(folder.identifier)
            .await
            .unwrap()
            .is_none()
    );
    assert!(subjects(&fixture, &first_session, "INBOX").await.is_empty());
    assert_eq!(
        subjects(&fixture, &first_session, "Trash").await,
        ["Production error"]
    );
    assert_eq!(
        subjects(&fixture, &second_session, "INBOX").await,
        ["Production error"]
    );
}

/// Smart folders reject even matching insertions and manual folder ownership does not follow roles.
#[tokio::test]
async fn smart_insertion_and_cross_user_folder_access_are_refused() {
    for mode in [ImapAccountMode::Mirror, ImapAccountMode::Independent] {
        let fixture = Fixture::new().await;
        let user = fixture
            .user("First", GlobalRole::Member, MailAccess::All)
            .await;
        let other = fixture
            .user("Other", GlobalRole::Owner, MailAccess::All)
            .await;
        let message = fixture.mail("Production error", "app.test").await;
        fixture.view(&user, "Laravel", "true").await;
        let account = fixture.account(&user, mode).await;
        if mode == ImapAccountMode::Independent {
            fixture
                .as_user(&user)
                .create_imap_folder(
                    account.account.account.identifier,
                    ImapFolderRequest {
                        name: "Laravel".into(),
                        filter: "true".into(),
                        include_in_inbox: false,
                    },
                )
                .await
                .unwrap();
        }
        let session = fixture.login(&account).await;
        let source = session.open("INBOX", false).await.unwrap();
        for move_source in [false, true] {
            assert!(matches!(
                session
                    .copy(&source.handle, &[1], "Laravel", move_source)
                    .await,
                Err(ImapAccessError::NotPermitted(_))
            ));
        }
        let smart = session.open("Laravel", false).await.unwrap();
        assert!(smart.seen_only);
        assert!(!source.seen_only);
        session
            .store(
                &source.handle,
                &[1],
                &ImapFlagChange {
                    operation: ImapFlagOperation::Add,
                    flags: ImapMessageFlags {
                        flagged: true,
                        ..ImapMessageFlags::default()
                    },
                },
            )
            .await
            .unwrap();
        session
            .store(
                &smart.handle,
                &[1],
                &ImapFlagChange {
                    operation: ImapFlagOperation::Replace,
                    flags: ImapMessageFlags {
                        seen: true,
                        ..ImapMessageFlags::default()
                    },
                },
            )
            .await
            .unwrap();
        let updated = session.open("Laravel", true).await.unwrap();
        assert!(updated.members[0].1.seen);
        assert!(updated.members[0].1.flagged);
        for operation in [ImapFlagOperation::Add, ImapFlagOperation::Remove] {
            session
                .store(
                    &smart.handle,
                    &[1],
                    &ImapFlagChange {
                        operation,
                        flags: ImapMessageFlags::default(),
                    },
                )
                .await
                .unwrap();
            assert!(
                session.open("Laravel", true).await.unwrap().members[0]
                    .1
                    .seen
            );
        }
        session
            .store(
                &smart.handle,
                &[1],
                &ImapFlagChange {
                    operation: ImapFlagOperation::Replace,
                    flags: ImapMessageFlags::default(),
                },
            )
            .await
            .unwrap();
        let cleared = session.open("Laravel", true).await.unwrap();
        assert!(!cleared.members[0].1.seen);
        assert!(cleared.members[0].1.flagged);
        assert!(matches!(
            session.rename_mailbox("Laravel", "Changed").await,
            Err(ImapAccessError::NotPermitted(_))
        ));
        assert!(matches!(
            session.delete_mailbox("Laravel").await,
            Err(ImapAccessError::NotPermitted(_))
        ));
        assert!(matches!(
            session
                .store(
                    &smart.handle,
                    &[1],
                    &ImapFlagChange {
                        operation: ImapFlagOperation::Add,
                        flags: ImapMessageFlags {
                            flagged: true,
                            ..ImapMessageFlags::default()
                        },
                    }
                )
                .await,
            Err(ImapAccessError::NotPermitted(_))
        ));
        assert!(matches!(
            session.expunge(&smart.handle, None).await,
            Err(ImapAccessError::NotPermitted(_))
        ));
        assert!(matches!(
            session.copy(&smart.handle, &[1], "INBOX", true).await,
            Err(ImapAccessError::NotPermitted(_))
        ));
        let folder = fixture
            .as_user(&user)
            .create_web_folder(WebFolderRequest {
                name: "Private Web".into(),
                icon: None,
                exposed_accounts: vec![],
            })
            .await
            .unwrap();
        let outsider = fixture.as_user(&other);
        assert!(matches!(
            outsider
                .query_web_mailbox(QueryOptions::default(), Some(folder.identifier))
                .await,
            Err(ApplicationError::NotFound)
        ));
        assert!(matches!(
            outsider
                .move_web_messages(vec![message], Some(folder.identifier))
                .await,
            Err(ApplicationError::NotFound)
        ));
        assert!(matches!(
            outsider.delete_web_folder(folder.identifier).await,
            Err(ApplicationError::NotFound)
        ));
        let caller = fixture.as_user(&user);
        caller.delete_message(message).await.unwrap();
        assert_eq!(
            caller
                .move_web_messages(vec![message], Some(folder.identifier))
                .await
                .unwrap(),
            0
        );
        assert!(
            web_subjects(&caller, Some(folder.identifier))
                .await
                .is_empty()
        );
    }
}

/// General mailbox COPY and Trash overrides survive authorization loss without broadening access.
#[tokio::test]
async fn general_copies_and_trash_moves_survive_permission_recalculation() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Member", GlobalRole::Member, MailAccess::Scoped)
        .await;
    let scope = fixture.scope(&user, "true").await;
    fixture.mail("Production error", "app.test").await;
    let issued = fixture.account(&user, ImapAccountMode::Independent).await;
    let session = fixture.login(&issued).await;
    session.create_mailbox("Private").await.unwrap();
    let inbox = session.open("INBOX", false).await.unwrap();
    session
        .copy(&inbox.handle, &[1], "Private", true)
        .await
        .unwrap();
    let private = session.open("Private", false).await.unwrap();
    session
        .copy(&private.handle, &[1], "INBOX", false)
        .await
        .unwrap();
    fixture
        .storage
        .remove_user_from_scope(user.identifier, scope)
        .await
        .unwrap();
    assert!(subjects(&fixture, &session, "INBOX").await.is_empty());
    assert!(subjects(&fixture, &session, "Private").await.is_empty());
    fixture
        .storage
        .assign_user_to_scope(user.identifier, scope)
        .await
        .unwrap();
    assert_eq!(
        subjects(&fixture, &session, "INBOX").await,
        ["Production error"]
    );
    let private = session.open("Private", false).await.unwrap();
    session
        .copy(&private.handle, &[private.members[0].0], "Trash", true)
        .await
        .unwrap();
    fixture
        .storage
        .remove_user_from_scope(user.identifier, scope)
        .await
        .unwrap();
    assert!(subjects(&fixture, &session, "Trash").await.is_empty());
    fixture
        .storage
        .assign_user_to_scope(user.identifier, scope)
        .await
        .unwrap();
    assert_eq!(
        subjects(&fixture, &session, "Trash").await,
        ["Production error"]
    );
    assert_eq!(
        subjects(&fixture, &session, "INBOX").await,
        ["Production error"],
        "an independent COPY is preserved"
    );
    assert!(subjects(&fixture, &session, "Private").await.is_empty());
}

/// Moving one occurrence never removes a copy explicitly stored in an unrelated folder.
#[tokio::test]
async fn moves_preserve_unrelated_manual_copies() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    fixture.mail("Production error", "app.test").await;
    let issued = fixture.account(&user, ImapAccountMode::Independent).await;
    let session = fixture.login(&issued).await;
    for name in ["Review", "Archive"] {
        session.create_mailbox(name).await.unwrap();
    }
    let inbox = session.open("INBOX", false).await.unwrap();
    session
        .copy(&inbox.handle, &[1], "Review", false)
        .await
        .unwrap();
    session
        .copy(&inbox.handle, &[1], "Archive", true)
        .await
        .unwrap();
    assert_eq!(
        subjects(&fixture, &session, "Review").await,
        ["Production error"]
    );
    let archive = session.open("Archive", false).await.unwrap();
    session
        .copy(&archive.handle, &[1], "INBOX", true)
        .await
        .unwrap();
    assert_eq!(
        subjects(&fixture, &session, "Review").await,
        ["Production error"]
    );
    assert_eq!(
        subjects(&fixture, &session, "INBOX").await,
        ["Production error"]
    );
    assert!(subjects(&fixture, &session, "Archive").await.is_empty());
}

/// Moving an additional COPY occurrence retains the original placement after reconciliation.
#[tokio::test]
async fn moving_copied_occurrence_preserves_primary_placement() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Member", GlobalRole::Member, MailAccess::Scoped)
        .await;
    let scope = fixture.scope(&user, "true").await;
    fixture.mail("Production error", "app.test").await;
    let issued = fixture.account(&user, ImapAccountMode::Independent).await;
    let session = fixture.login(&issued).await;
    for name in ["Original", "Copy", "Moved Copy"] {
        session.create_mailbox(name).await.unwrap();
    }
    let inbox = session.open("INBOX", false).await.unwrap();
    session
        .copy(&inbox.handle, &[1], "Original", true)
        .await
        .unwrap();
    let original = session.open("Original", false).await.unwrap();
    session
        .copy(&original.handle, &[1], "Copy", false)
        .await
        .unwrap();
    let copied = session.open("Copy", false).await.unwrap();
    session
        .copy(&copied.handle, &[1], "Moved Copy", true)
        .await
        .unwrap();
    assert_eq!(
        subjects(&fixture, &session, "Original").await,
        ["Production error"]
    );
    assert_eq!(
        subjects(&fixture, &session, "Moved Copy").await,
        ["Production error"]
    );
    assert!(subjects(&fixture, &session, "Copy").await.is_empty());
    fixture
        .storage
        .remove_user_from_scope(user.identifier, scope)
        .await
        .unwrap();
    assert!(subjects(&fixture, &session, "Original").await.is_empty());
    fixture
        .storage
        .assign_user_to_scope(user.identifier, scope)
        .await
        .unwrap();
    assert_eq!(
        subjects(&fixture, &session, "Original").await,
        ["Production error"]
    );
    assert_eq!(
        subjects(&fixture, &session, "Moved Copy").await,
        ["Production error"]
    );
    assert!(subjects(&fixture, &session, "Copy").await.is_empty());
}

/// COPY preserves the source flags, including the removable mailbox-local Deleted flag.
#[tokio::test]
async fn copied_members_preserve_deleted_flags() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    fixture.mail("Production error", "app.test").await;
    let issued = fixture.account(&user, ImapAccountMode::Independent).await;
    let session = fixture.login(&issued).await;
    session.create_mailbox("Copy").await.unwrap();
    let inbox = session.open("INBOX", false).await.unwrap();
    session
        .store(
            &inbox.handle,
            &[1],
            &ImapFlagChange {
                operation: ImapFlagOperation::Add,
                flags: ImapMessageFlags {
                    deleted: true,
                    seen: true,
                    ..ImapMessageFlags::default()
                },
            },
        )
        .await
        .unwrap();
    session
        .copy(&inbox.handle, &[1], "Copy", false)
        .await
        .unwrap();
    let copied = session.open("Copy", false).await.unwrap();
    assert!(copied.members[0].1.deleted);
    assert!(copied.members[0].1.seen);
    assert_eq!(
        subjects(&fixture, &session, "INBOX").await,
        ["Production error"]
    );
}

/// Deleting one folder preserves surviving copies; deleting the last normal folder trashes mail.
#[tokio::test]
async fn deleting_last_normal_copy_moves_message_to_trash() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    fixture.mail("Production error", "app.test").await;
    let issued = fixture.account(&user, ImapAccountMode::Independent).await;
    let session = fixture.login(&issued).await;
    for name in ["Primary", "Moved Copy"] {
        session.create_mailbox(name).await.unwrap();
    }
    let inbox = session.open("INBOX", false).await.unwrap();
    session
        .copy(&inbox.handle, &[1], "Primary", true)
        .await
        .unwrap();
    let primary = session.open("Primary", false).await.unwrap();
    session
        .copy(&primary.handle, &[1], "INBOX", false)
        .await
        .unwrap();
    let inbox_copy = session.open("INBOX", false).await.unwrap();
    session
        .copy(
            &inbox_copy.handle,
            &[inbox_copy.members[0].0],
            "Moved Copy",
            true,
        )
        .await
        .unwrap();
    assert_eq!(
        subjects(&fixture, &session, "Primary").await,
        ["Production error"]
    );
    assert!(subjects(&fixture, &session, "INBOX").await.is_empty());
    session.delete_mailbox("Primary").await.unwrap();
    assert_eq!(
        subjects(&fixture, &session, "Moved Copy").await,
        ["Production error"]
    );
    assert!(subjects(&fixture, &session, "Trash").await.is_empty());
    session.delete_mailbox("Moved Copy").await.unwrap();
    assert!(subjects(&fixture, &session, "INBOX").await.is_empty());
    assert_eq!(
        subjects(&fixture, &session, "Trash").await,
        ["Production error"]
    );
}

/// A copied Mirror occurrence that becomes primary subsequently moves the shared Web placement.
#[tokio::test]
async fn moving_copy_that_became_primary_updates_web_organization() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    let message = fixture.mail("Production error", "app.test").await;
    let issued = fixture.account(&user, ImapAccountMode::Mirror).await;
    let session = fixture.login(&issued).await;
    let caller = fixture.as_user(&user);
    let folder = caller
        .create_web_folder(WebFolderRequest {
            name: "Web Baseline".into(),
            icon: None,
            exposed_accounts: vec![issued.account.account.identifier],
        })
        .await
        .unwrap();
    let inbox = session.open("INBOX", false).await.unwrap();
    session
        .copy(&inbox.handle, &[1], "Web Baseline", false)
        .await
        .unwrap();
    caller
        .move_web_messages(vec![message], Some(folder.identifier))
        .await
        .unwrap();
    session.create_mailbox("Private").await.unwrap();
    let inherited = session.open("Web Baseline", false).await.unwrap();
    session
        .copy(
            &inherited.handle,
            &[inherited.members[0].0],
            "Private",
            true,
        )
        .await
        .unwrap();
    assert!(
        subjects(&fixture, &session, "Web Baseline")
            .await
            .is_empty()
    );
    assert!(subjects(&fixture, &session, "INBOX").await.is_empty());
    assert_eq!(
        subjects(&fixture, &session, "Private").await,
        ["Production error"]
    );
    assert!(
        web_subjects(&caller, Some(folder.identifier))
            .await
            .is_empty()
    );
    let moved_folder = caller
        .list_web_folders()
        .await
        .unwrap()
        .into_iter()
        .find(|folder| folder.name == "Private")
        .unwrap();
    assert_eq!(
        web_subjects(&caller, Some(moved_folder.identifier)).await,
        ["Production error"]
    );
    session.delete_mailbox("Private").await.unwrap();
    assert!(
        subjects(&fixture, &session, "Web Baseline")
            .await
            .is_empty()
    );
    assert_eq!(
        subjects(&fixture, &session, "Trash").await,
        ["Production error"]
    );
}

/// Web COPY reaches every Mirror, preserves Inbox/Trash, and accepts IMAP copies back into Web folders.
#[tokio::test]
async fn web_copy_round_trips_multiple_mirrors_and_normal_folders() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Copy owner", GlobalRole::Member, MailAccess::All)
        .await;
    let message = fixture.mail("Shared copies", "app.test").await;
    let caller = fixture.as_user(&user);
    let first_folder = caller
        .create_web_folder(WebFolderRequest {
            name: "First".into(),
            icon: None,
            exposed_accounts: Vec::new(),
        })
        .await
        .unwrap();
    let second_folder = caller
        .create_web_folder(WebFolderRequest {
            name: "Second".into(),
            icon: None,
            exposed_accounts: Vec::new(),
        })
        .await
        .unwrap();
    let mirror_credentials = fixture.account(&user, ImapAccountMode::Mirror).await;
    let peer_credentials = fixture.account(&user, ImapAccountMode::Mirror).await;
    let independent_credentials = fixture.account(&user, ImapAccountMode::Independent).await;
    let mirror = fixture.login(&mirror_credentials).await;
    let peer = fixture.login(&peer_credentials).await;
    let independent = fixture.login(&independent_credentials).await;
    // Opening before COPY proves stale materialized memberships cannot authorize a duplicate.
    assert!(mirror.open("First", true).await.unwrap().members.is_empty());
    assert!(matches!(
        caller.copy_web_messages(vec![message], None).await,
        Err(ApplicationError::Conflict)
    ));
    assert_eq!(
        caller
            .copy_web_messages(vec![message], Some(first_folder.identifier))
            .await
            .unwrap(),
        1
    );
    assert!(matches!(
        caller
            .copy_web_messages(vec![message], Some(first_folder.identifier))
            .await,
        Err(ApplicationError::Conflict)
    ));
    for session in [&mirror, &peer] {
        assert_eq!(
            subjects(&fixture, session, "INBOX").await,
            ["Shared copies"]
        );
        assert_eq!(
            subjects(&fixture, session, "First").await,
            ["Shared copies"]
        );
    }
    assert!(
        independent
            .open("First", true)
            .await
            .unwrap()
            .members
            .is_empty()
    );
    let source = mirror.open("First", false).await.unwrap();
    mirror
        .copy(&source.handle, &[source.members[0].0], "Second", false)
        .await
        .unwrap();
    assert_eq!(
        web_subjects(&caller, Some(second_folder.identifier)).await,
        ["Shared copies"]
    );
    assert_eq!(subjects(&fixture, &peer, "Second").await, ["Shared copies"]);
    caller.trash_web_messages(vec![message]).await.unwrap();
    assert_eq!(
        caller
            .copy_web_messages(vec![message], Some(first_folder.identifier))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        caller.copy_web_messages(vec![message], None).await.unwrap(),
        1
    );
    for session in [&mirror, &peer] {
        assert_eq!(
            subjects(&fixture, session, "Trash").await,
            ["Shared copies"]
        );
        assert_eq!(
            subjects(&fixture, session, "First").await,
            ["Shared copies"]
        );
        assert_eq!(
            subjects(&fixture, session, "INBOX").await,
            ["Shared copies"]
        );
    }
    assert_eq!(
        caller
            .query_web_trash(QueryOptions::default())
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        web_subjects(&caller, Some(first_folder.identifier)).await,
        ["Shared copies"]
    );
    assert_eq!(web_subjects(&caller, None).await, ["Shared copies"]);
    assert_eq!(
        fixture
            .storage
            .list_messages(sandpost_storage::MessageListQuery {
                filter: None,
                before: None,
                limit: 100
            })
            .await
            .unwrap()
            .len(),
        1
    );
}

/// COPY validates canonical occurrences, ownership and current access, rolling back whole batches.
#[tokio::test]
async fn web_copy_duplicate_batches_and_concurrent_retries_are_atomic() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Copy owner", GlobalRole::Member, MailAccess::Scoped)
        .await;
    let scope = fixture.scope(&user, "true").await;
    let first = fixture.mail("First", "app.test").await;
    let second = fixture.mail("Second", "app.test").await;
    let caller = fixture.as_user(&user);
    let folder = caller
        .create_web_folder(WebFolderRequest {
            name: "Copies".into(),
            icon: None,
            exposed_accounts: Vec::new(),
        })
        .await
        .unwrap();
    caller
        .copy_web_messages(vec![second], Some(folder.identifier))
        .await
        .unwrap();
    assert!(matches!(
        caller
            .copy_web_messages(vec![first, second], Some(folder.identifier))
            .await,
        Err(ApplicationError::Conflict)
    ));
    assert_eq!(
        web_subjects(&caller, Some(folder.identifier)).await,
        ["Second"]
    );
    let (left, right) = tokio::join!(
        caller.copy_web_messages(vec![first], Some(folder.identifier)),
        caller.copy_web_messages(vec![first], Some(folder.identifier))
    );
    assert!(
        matches!(
            (&left, &right),
            (Ok(1), Err(ApplicationError::Conflict)) | (Err(ApplicationError::Conflict), Ok(1))
        ),
        "{left:?} {right:?}"
    );
    let other_user = fixture
        .user("Other owner", GlobalRole::Member, MailAccess::All)
        .await;
    assert!(matches!(
        fixture
            .as_user(&other_user)
            .copy_web_messages(vec![first], Some(folder.identifier))
            .await,
        Err(ApplicationError::NotFound)
    ));
    assert!(matches!(
        caller.copy_web_messages(vec![first; 257], None).await,
        Err(ApplicationError::InvalidInput(_))
    ));
    let destination = caller
        .create_web_folder(WebFolderRequest {
            name: "Empty".into(),
            icon: None,
            exposed_accounts: Vec::new(),
        })
        .await
        .unwrap();
    let mut definition = fixture.storage.get_scope(scope).await.unwrap().unwrap();
    definition.filter = "false".into();
    definition.policy_version += 1;
    fixture.storage.save_scope(&definition).await.unwrap();
    assert!(matches!(
        caller
            .copy_web_messages(vec![first], Some(destination.identifier))
            .await,
        Err(ApplicationError::NotFound)
    ));
    definition.filter = "true".into();
    definition.policy_version += 1;
    fixture.storage.save_scope(&definition).await.unwrap();
    caller.delete_message(second).await.unwrap();
    assert!(matches!(
        caller
            .copy_web_messages(vec![first, second], Some(destination.identifier))
            .await,
        Err(ApplicationError::NotFound)
    ));
    assert!(
        web_subjects(&caller, Some(destination.identifier))
            .await
            .is_empty()
    );
    assert!(matches!(
        fixture
            .storage
            .copy_user_messages(
                UserIdentifier::new(),
                &[],
                None,
                &sandpost_query::Expression::True
            )
            .await,
        Err(sandpost_storage::StorageError::NotFound)
    ));
}

/// Primary supported flags round-trip Web/IMAP, isolate Independent, validate keywords and retry atomically.
#[tokio::test]
async fn web_supported_flags_share_mirrors_and_validate_atomic_results() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Flag owner", GlobalRole::Member, MailAccess::Scoped)
        .await;
    let scope = fixture.scope(&user, "true").await;
    let first = fixture.mail("First", "app.test").await;
    let second = fixture.mail("Second", "app.test").await;
    let caller = fixture.as_user(&user);
    let mirror_credentials = fixture.account(&user, ImapAccountMode::Mirror).await;
    let peer_credentials = fixture.account(&user, ImapAccountMode::Mirror).await;
    let independent_credentials = fixture.account(&user, ImapAccountMode::Independent).await;
    let mirror = fixture.login(&mirror_credentials).await;
    let peer = fixture.login(&peer_credentials).await;
    let independent = fixture.login(&independent_credentials).await;
    let watched = peer.open("INBOX", true).await.unwrap();
    let mut changes = fixture.application.subscribe_changes();
    let flags = ImapMessageFlags {
        seen: true,
        answered: true,
        flagged: true,
        draft: true,
        keywords: vec!["$Forwarded".into(), "custom".into(), "CUSTOM".into()],
        ..Default::default()
    };
    let change = ImapFlagChange {
        operation: ImapFlagOperation::Add,
        flags,
    };
    assert_eq!(
        caller
            .set_web_messages_flags(vec![first, first], change.clone())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        caller
            .set_web_messages_flags(vec![first], change.clone())
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        changes.try_recv().unwrap(),
        crate::ApplicationChange::Visibility
    );
    assert_eq!(
        changes.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    );
    let updates = peer
        .synchronize(&watched.handle, 2, 2, watched.cursor, false)
        .await
        .unwrap();
    assert_eq!(updates.flag_updates.len(), 1);
    assert!(updates.flag_updates[0].1.answered && updates.flag_updates[0].1.draft);
    let expected = ImapMessageFlags {
        keywords: vec!["$Forwarded".into(), "custom".into()],
        ..change.flags.clone()
    };
    for session in [&mirror, &peer] {
        let inbox = session.open("INBOX", true).await.unwrap();
        assert_eq!(inbox.members[0].1, expected);
    }
    assert_eq!(
        independent.open("INBOX", true).await.unwrap().members[0].1,
        ImapMessageFlags::default()
    );
    let summaries = caller
        .query_web_mailbox(QueryOptions::default(), None)
        .await
        .unwrap();
    let summary = summaries
        .iter()
        .find(|summary| summary.identifier == first)
        .unwrap();
    assert!(summary.seen);
    assert_eq!(summary.flags, expected);
    let indexed = fixture
        .storage
        .indexed_messages(&[first, second])
        .await
        .unwrap();
    let revisions: Vec<_> = indexed
        .iter()
        .map(|message| (message.identifier, message.revision))
        .collect();
    let hydrated = fixture
        .storage
        .hydrate_current_user_messages(
            user.identifier,
            &revisions,
            &sandpost_query::Expression::True,
        )
        .await
        .unwrap();
    assert_eq!(
        hydrated
            .iter()
            .find(|summary| summary.identifier == first)
            .unwrap()
            .flags,
        expected
    );
    assert!(
        fixture
            .storage
            .list_messages(sandpost_storage::MessageListQuery {
                filter: None,
                before: None,
                limit: 100
            })
            .await
            .unwrap()
            .iter()
            .all(|summary| summary.flags == ImapMessageFlags::default())
    );
    caller
        .set_web_messages_seen(vec![first], false)
        .await
        .unwrap();
    let inbox = mirror.open("INBOX", false).await.unwrap();
    assert!(!inbox.members[0].1.seen);
    assert!(inbox.members[0].1.answered && inbox.members[0].1.draft);
    mirror
        .store(
            &inbox.handle,
            &[inbox.members[0].0],
            &ImapFlagChange {
                operation: ImapFlagOperation::Remove,
                flags: ImapMessageFlags {
                    answered: true,
                    keywords: vec!["CUSTOM".into()],
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();
    let summary = caller
        .query_web_mailbox(QueryOptions::default(), None)
        .await
        .unwrap()
        .into_iter()
        .find(|summary| summary.identifier == first)
        .unwrap();
    assert!(!summary.flags.answered);
    assert_eq!(summary.flags.keywords, ["$Forwarded"]);
    assert!(summary.flags.flagged && summary.flags.draft);
    // A later member exceeding the resulting limit must roll back every earlier update.
    let full = ImapFlagChange {
        operation: ImapFlagOperation::Replace,
        flags: ImapMessageFlags {
            keywords: (0..32).map(|index| format!("keyword{index}")).collect(),
            ..Default::default()
        },
    };
    caller
        .set_web_messages_flags(vec![second], full)
        .await
        .unwrap();
    let overflow = ImapFlagChange {
        operation: ImapFlagOperation::Add,
        flags: ImapMessageFlags {
            keywords: vec!["extra".into()],
            ..Default::default()
        },
    };
    let before = caller
        .query_web_mailbox(QueryOptions::default(), None)
        .await
        .unwrap();
    for _ in 0..2 {
        assert!(matches!(
            caller
                .set_web_messages_flags(vec![first, second], overflow.clone())
                .await,
            Err(ApplicationError::InvalidInput(_))
        ));
        let after = caller
            .query_web_mailbox(QueryOptions::default(), None)
            .await
            .unwrap();
        assert_eq!(
            after
                .iter()
                .map(|summary| &summary.flags)
                .collect::<Vec<_>>(),
            before
                .iter()
                .map(|summary| &summary.flags)
                .collect::<Vec<_>>()
        );
    }
    for keyword in [
        "".into(),
        "two words".into(),
        "\\Seen".into(),
        "bad]".into(),
        "bad(".into(),
        "bad%".into(),
        "bad*".into(),
        "bad{\"".into(),
        "bad\\".into(),
        "bad\n".into(),
        "é".into(),
        "x".repeat(65),
    ] {
        assert!(matches!(
            caller
                .set_web_messages_flags(
                    vec![first],
                    ImapFlagChange {
                        operation: ImapFlagOperation::Add,
                        flags: ImapMessageFlags {
                            keywords: vec![keyword],
                            ..Default::default()
                        }
                    }
                )
                .await,
            Err(ApplicationError::InvalidInput(_))
        ));
    }
    assert!(matches!(
        caller
            .set_web_messages_flags(
                vec![first],
                ImapFlagChange {
                    operation: ImapFlagOperation::Add,
                    flags: ImapMessageFlags {
                        deleted: true,
                        ..Default::default()
                    }
                }
            )
            .await,
        Err(ApplicationError::InvalidInput(_))
    ));
    caller
        .set_web_messages_flags(
            vec![first],
            ImapFlagChange {
                operation: ImapFlagOperation::Replace,
                flags: ImapMessageFlags {
                    keywords: vec!["x".repeat(64)],
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();
    let replaced = caller
        .query_web_mailbox(QueryOptions::default(), None)
        .await
        .unwrap();
    let replaced = replaced
        .iter()
        .find(|summary| summary.identifier == first)
        .unwrap();
    assert!(!replaced.seen && !replaced.flags.flagged && !replaced.flags.draft);
    assert_eq!(replaced.flags.keywords, ["x".repeat(64)]);
    let mut definition = fixture.storage.get_scope(scope).await.unwrap().unwrap();
    definition.filter = "false".into();
    definition.policy_version += 1;
    fixture.storage.save_scope(&definition).await.unwrap();
    assert_eq!(
        caller
            .set_web_messages_flags(vec![first], change)
            .await
            .unwrap(),
        0
    );
    assert!(matches!(
        fixture
            .storage
            .set_user_message_flags(
                UserIdentifier::new(),
                &[],
                &overflow,
                &sandpost_query::Expression::True
            )
            .await,
        Err(sandpost_storage::StorageError::NotFound)
    ));
}
