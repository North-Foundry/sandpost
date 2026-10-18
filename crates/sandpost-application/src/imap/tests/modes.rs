//! Mode history, isolation, durable retention, and final-reference collection through services.
use super::*;
use crate::{ApplicationChange, WebFolderRequest};

/// Check that a mutation publishes exactly one hint for the organization it changed.
fn assert_single_change(
    changes: &mut broadcast::Receiver<ApplicationChange>,
    expected: ApplicationChange,
) {
    assert_eq!(changes.try_recv().unwrap(), expected);
    assert_eq!(
        changes.try_recv(),
        Err(broadcast::error::TryRecvError::Empty)
    );
}

/// Switch organization, then authenticate a fresh session with the unchanged credentials.
async fn switch_mode(
    fixture: &Fixture,
    user: &User,
    issued: &IssuedImapCredentials,
    mode: ImapAccountMode,
) -> ImapAccountSession {
    fixture
        .as_user(user)
        .update_imap_account(
            issued.account.account.identifier,
            ImapAccountUpdate {
                mode: Some(mode),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    fixture.login(issued).await
}

/// Flag and expunge every current occurrence, returning the selected mailbox's original UIDs.
async fn expunge_all(session: &ImapAccountSession, name: &str) -> Vec<u32> {
    let snapshot = session.open(name, false).await.unwrap();
    let identifiers: Vec<_> = snapshot.members.iter().map(|member| member.0).collect();
    session
        .store(
            &snapshot.handle,
            &identifiers,
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
    assert_eq!(
        session.expunge(&snapshot.handle, None).await.unwrap(),
        identifiers
    );
    identifiers
}

/// Creation snapshots primary history, folder placement, Trash, copies, and flags within authorization.
#[tokio::test]
async fn independent_creation_snapshots_current_authorized_primary_history() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Scoped", GlobalRole::Member, MailAccess::Scoped)
        .await;
    fixture
        .scope(&user, "envelope.to.domain == \"allowed.test\"")
        .await;
    fixture.mail("inbox", "allowed.test").await;
    let filed = fixture.mail("filed", "allowed.test").await;
    let trashed = fixture.mail("trashed", "allowed.test").await;
    let released = fixture.mail("released", "allowed.test").await;
    fixture.mail("denied history", "denied.test").await;
    let caller = fixture.as_user(&user);
    let folder = caller
        .create_web_folder(WebFolderRequest {
            name: "History".into(),
            icon: None,
            exposed_accounts: Vec::new(),
        })
        .await
        .unwrap();
    caller
        .move_web_messages(vec![filed], Some(folder.identifier))
        .await
        .unwrap();
    caller.trash_web_messages(vec![trashed]).await.unwrap();
    let mirror = fixture.account(&user, ImapAccountMode::Mirror).await;
    let primary = fixture.login(&mirror).await;
    let opened = primary.open("History", false).await.unwrap();
    primary
        .store(
            &opened.handle,
            &[opened.members[0].0],
            &ImapFlagChange {
                operation: ImapFlagOperation::Add,
                flags: ImapMessageFlags {
                    seen: true,
                    flagged: true,
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();
    primary
        .copy(&opened.handle, &[opened.members[0].0], "INBOX", false)
        .await
        .unwrap();
    caller.delete_message(released).await.unwrap();
    assert!(
        fixture
            .storage
            .get_message(released)
            .await
            .unwrap()
            .is_some(),
        "dormant Mirror account state retains released primary history"
    );

    let issued = fixture.account(&user, ImapAccountMode::Independent).await;
    let independent = fixture.login(&issued).await;
    assert_eq!(
        subjects(&fixture, &independent, "INBOX").await,
        ["inbox", "filed"]
    );
    assert_eq!(subjects(&fixture, &independent, "History").await, ["filed"]);
    assert_eq!(subjects(&fixture, &independent, "Trash").await, ["trashed"]);
    let copied_folder = independent.open("History", true).await.unwrap();
    assert!(copied_folder.members[0].1.seen && copied_folder.members[0].1.flagged);
    let details = caller
        .get_imap_account(issued.account.account.identifier)
        .await
        .unwrap();
    assert_eq!(
        details.folders[0].kind,
        sandpost_core::ImapFolderKind::Private
    );
    assert_ne!(copied_folder.handle.source, opened.handle.source);

    // Expanding permission does not import historical mail absent from the creation snapshot.
    fixture
        .scope(&user, "envelope.to.domain == \"denied.test\"")
        .await;
    assert!(
        subjects(&fixture, &primary, "INBOX")
            .await
            .contains(&"denied history".into())
    );
    assert_eq!(
        subjects(&fixture, &independent, "INBOX").await,
        ["inbox", "filed"]
    );
    fixture
        .mail("new denied-domain arrival", "denied.test")
        .await;
    assert_eq!(
        subjects(&fixture, &independent, "INBOX").await,
        ["inbox", "filed", "new denied-domain arrival"]
    );
    assert!(
        !subjects(&fixture, &independent, "INBOX")
            .await
            .contains(&"released".into())
    );
}

/// Mirror shares primary flags; Independent restores its own UIDs, flags, placement, and dormant arrivals.
#[tokio::test]
async fn mode_round_trip_restores_independent_identity_and_receives_dormant_mail() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    fixture.mail("original", "app.test").await;
    let issued = fixture.account(&user, ImapAccountMode::Mirror).await;
    let sibling = fixture.account(&user, ImapAccountMode::Mirror).await;
    let separate = fixture.account(&user, ImapAccountMode::Independent).await;
    let primary = fixture.login(&issued).await;
    let sibling = fixture.login(&sibling).await;
    let separate = fixture.login(&separate).await;
    let primary_inbox = primary.open("INBOX", false).await.unwrap();
    primary
        .store(
            &primary_inbox.handle,
            &[primary_inbox.members[0].0],
            &ImapFlagChange {
                operation: ImapFlagOperation::Add,
                flags: ImapMessageFlags {
                    flagged: true,
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();
    assert!(
        sibling.open("INBOX", true).await.unwrap().members[0]
            .1
            .flagged
    );
    assert!(
        !separate.open("INBOX", true).await.unwrap().members[0]
            .1
            .flagged
    );
    primary
        .copy(
            &primary_inbox.handle,
            &[primary_inbox.members[0].0],
            "Trash",
            true,
        )
        .await
        .unwrap();
    assert_eq!(subjects(&fixture, &sibling, "Trash").await, ["original"]);
    assert_eq!(subjects(&fixture, &separate, "INBOX").await, ["original"]);

    let independent = switch_mode(&fixture, &user, &issued, ImapAccountMode::Independent).await;
    assert!(matches!(
        primary.validate().await,
        Err(ImapAccessError::Revoked)
    ));
    let independent_inbox = independent.open("INBOX", false).await.unwrap();
    assert_ne!(independent_inbox.uid_validity, primary_inbox.uid_validity);
    assert!(!independent_inbox.members[0].1.flagged);
    independent.create_mailbox("Private").await.unwrap();
    independent
        .copy(
            &independent_inbox.handle,
            &[independent_inbox.members[0].0],
            "Private",
            true,
        )
        .await
        .unwrap();
    let private = independent.open("Private", false).await.unwrap();
    independent
        .store(
            &private.handle,
            &[private.members[0].0],
            &ImapFlagChange {
                operation: ImapFlagOperation::Add,
                flags: ImapMessageFlags {
                    seen: true,
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();
    let mirror = switch_mode(&fixture, &user, &issued, ImapAccountMode::Mirror).await;
    assert_eq!(
        mirror.open("INBOX", true).await.unwrap().uid_validity,
        primary_inbox.uid_validity
    );
    assert_eq!(subjects(&fixture, &mirror, "Trash").await, ["original"]);
    assert!(matches!(
        mirror.open("Private", true).await,
        Err(ImapAccessError::NonExistent)
    ));
    let arrival = fixture.mail("dormant arrival", "app.test").await;
    fixture
        .as_user(&user)
        .delete_message(arrival)
        .await
        .unwrap();
    assert!(subjects(&fixture, &mirror, "INBOX").await.is_empty());

    let restored = switch_mode(&fixture, &user, &issued, ImapAccountMode::Independent).await;
    assert!(matches!(
        independent.validate().await,
        Err(ImapAccessError::Revoked)
    ));
    let private_again = restored.open("Private", true).await.unwrap();
    assert_eq!(private_again.uid_validity, private.uid_validity);
    assert_eq!(private_again.members[0].0, private.members[0].0);
    assert!(private_again.members[0].1.seen);
    assert_eq!(subjects(&fixture, &restored, "Private").await, ["original"]);
    let arrivals = restored.open("INBOX", false).await.unwrap();
    assert_eq!(arrivals.uid_validity, independent_inbox.uid_validity);
    assert!(arrivals.members[0].0 > independent_inbox.members[0].0);
    assert_eq!(
        subjects(&fixture, &restored, "INBOX").await,
        ["dormant arrival"]
    );
}

/// Released occurrences never reappear, and dormant references delay physical collection until account deletion.
#[tokio::test]
async fn final_release_collects_only_after_dormant_views_release_and_never_resurrects() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Member", GlobalRole::Member, MailAccess::Scoped)
        .await;
    let scope = fixture.scope(&user, "true").await;
    let message = fixture.mail("retained", "app.test").await;
    let dormant = fixture.account(&user, ImapAccountMode::Mirror).await;
    let active = fixture.account(&user, ImapAccountMode::Independent).await;
    let caller = fixture.as_user(&user);
    caller.delete_message(message).await.unwrap();
    let independent = fixture.login(&active).await;
    assert_eq!(
        subjects(&fixture, &independent, "INBOX").await,
        ["retained"]
    );
    expunge_all(&independent, "INBOX").await;
    assert!(subjects(&fixture, &independent, "INBOX").await.is_empty());
    assert_eq!(
        subjects(&fixture, &independent, "Trash").await,
        ["retained"]
    );
    expunge_all(&independent, "Trash").await;
    assert!(subjects(&fixture, &independent, "Trash").await.is_empty());
    assert!(
        fixture
            .storage
            .get_message(message)
            .await
            .unwrap()
            .is_some()
    );
    caller
        .restore_imap_hidden_messages(active.account.account.identifier)
        .await
        .unwrap();
    let reconnected = fixture.login(&active).await;
    assert!(subjects(&fixture, &reconnected, "INBOX").await.is_empty());
    assert!(subjects(&fixture, &reconnected, "Trash").await.is_empty());
    assert_eq!(
        caller.move_web_messages(vec![message], None).await.unwrap(),
        0
    );
    let released_mirror = switch_mode(&fixture, &user, &active, ImapAccountMode::Mirror).await;
    assert!(
        subjects(&fixture, &released_mirror, "INBOX")
            .await
            .is_empty()
    );
    let reconnected = switch_mode(&fixture, &user, &active, ImapAccountMode::Independent).await;
    assert!(subjects(&fixture, &reconnected, "INBOX").await.is_empty());
    assert!(subjects(&fixture, &reconnected, "Trash").await.is_empty());
    let fresh = fixture.account(&user, ImapAccountMode::Independent).await;
    assert!(
        subjects(&fixture, &fixture.login(&fresh).await, "INBOX")
            .await
            .is_empty()
    );
    fixture
        .storage
        .remove_user_from_scope(user.identifier, scope)
        .await
        .unwrap();
    caller
        .delete_imap_account(fresh.account.account.identifier)
        .await
        .unwrap();
    assert!(
        fixture
            .storage
            .get_message(message)
            .await
            .unwrap()
            .is_some(),
        "a denied dormant reference still prevents collection"
    );
    let retained = switch_mode(&fixture, &user, &dormant, ImapAccountMode::Independent).await;
    assert!(subjects(&fixture, &retained, "INBOX").await.is_empty());
    fixture
        .storage
        .assign_user_to_scope(user.identifier, scope)
        .await
        .unwrap();
    assert_eq!(subjects(&fixture, &retained, "INBOX").await, ["retained"]);
    caller
        .delete_imap_account(dormant.account.account.identifier)
        .await
        .unwrap();
    assert!(matches!(
        retained.validate().await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(
        fixture
            .storage
            .get_message(message)
            .await
            .unwrap()
            .is_none(),
        "no primary or independent reference remains"
    );
    assert!(subjects(&fixture, &reconnected, "INBOX").await.is_empty());
}

/// Both modes promote surviving normal copies, trash the last normal occurrence, and release final Trash.
#[tokio::test]
async fn expunge_and_folder_deletion_apply_the_occurrence_lifecycle_in_both_modes() {
    for mode in [ImapAccountMode::Mirror, ImapAccountMode::Independent] {
        let fixture = Fixture::new().await;
        let user = fixture
            .user("Member", GlobalRole::Member, MailAccess::All)
            .await;
        let message = fixture.mail("retained", "app.test").await;
        let issued = fixture.account(&user, mode).await;
        let session = fixture.login(&issued).await;
        session.create_mailbox("Copy").await.unwrap();
        session.create_mailbox("Archive").await.unwrap();
        let inbox = session.open("INBOX", false).await.unwrap();
        session
            .copy(&inbox.handle, &[inbox.members[0].0], "Copy", false)
            .await
            .unwrap();
        expunge_all(&session, "INBOX").await;
        assert!(subjects(&fixture, &session, "INBOX").await.is_empty());
        assert!(subjects(&fixture, &session, "Trash").await.is_empty());
        let copy = session.open("Copy", false).await.unwrap();
        session
            .copy(&copy.handle, &[copy.members[0].0], "Archive", false)
            .await
            .unwrap();
        expunge_all(&session, "Copy").await;
        assert_eq!(subjects(&fixture, &session, "Archive").await, ["retained"]);
        assert!(subjects(&fixture, &session, "Trash").await.is_empty());
        session.delete_mailbox("Archive").await.unwrap();
        assert_eq!(subjects(&fixture, &session, "Trash").await, ["retained"]);
        expunge_all(&session, "Trash").await;
        fixture
            .as_user(&user)
            .restore_imap_hidden_messages(issued.account.account.identifier)
            .await
            .unwrap();
        let reconnected = fixture.login(&issued).await;
        assert!(subjects(&fixture, &reconnected, "INBOX").await.is_empty());
        assert!(subjects(&fixture, &reconnected, "Copy").await.is_empty());
        assert!(subjects(&fixture, &reconnected, "Trash").await.is_empty());
        assert!(
            fixture
                .storage
                .get_message(message)
                .await
                .unwrap()
                .is_some(),
            "the other organization still retains content"
        );
    }
}

/// Dormant independent smart definitions cannot be mutated through Mirror management requests.
#[tokio::test]
async fn dormant_smart_definitions_are_protected_from_active_mode_management() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    fixture.mail("retained", "app.test").await;
    let issued = fixture.account(&user, ImapAccountMode::Independent).await;
    let identifier = issued.account.account.identifier;
    let caller = fixture.as_user(&user);
    let smart = caller
        .create_imap_folder(
            identifier,
            ImapFolderRequest {
                name: "Private smart".into(),
                filter: "true".into(),
                include_in_inbox: false,
            },
        )
        .await
        .unwrap();
    let mirror = switch_mode(&fixture, &user, &issued, ImapAccountMode::Mirror).await;
    assert!(matches!(
        mirror.open("Private smart", true).await,
        Err(ImapAccessError::NonExistent)
    ));
    assert!(matches!(
        caller
            .update_imap_folder(
                identifier,
                smart.identifier,
                ImapFolderRequest {
                    name: "Changed while dormant".into(),
                    filter: "false".into(),
                    include_in_inbox: false,
                }
            )
            .await,
        Err(ApplicationError::NotFound)
    ));
    assert!(matches!(
        caller
            .delete_imap_folder(identifier, smart.identifier)
            .await,
        Err(ApplicationError::NotFound)
    ));
    let independent = switch_mode(&fixture, &user, &issued, ImapAccountMode::Independent).await;
    assert_eq!(
        subjects(&fixture, &independent, "Private smart").await,
        ["retained"]
    );
}

/// Deleting a populated credential removes its catalogs and subscriptions without changing peers.
#[tokio::test]
async fn deleting_populated_account_preserves_primary_and_peer_organization() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    let message = fixture.mail("shared content", "app.test").await;
    let issued = fixture.account(&user, ImapAccountMode::Independent).await;
    let peer = fixture.account(&user, ImapAccountMode::Independent).await;
    let mirror = fixture.account(&user, ImapAccountMode::Mirror).await;
    let session = fixture.login(&issued).await;
    let peer_session = fixture.login(&peer).await;
    let mirror_session = fixture.login(&mirror).await;
    let identifier = issued.account.account.identifier;
    let caller = fixture.as_user(&user);
    let smart = caller
        .create_imap_folder(
            identifier,
            ImapFolderRequest {
                name: "Private matches".into(),
                filter: "true".into(),
                include_in_inbox: false,
            },
        )
        .await
        .unwrap();
    session.create_mailbox("Private copies").await.unwrap();
    let inbox = session.open("INBOX", false).await.unwrap();
    session
        .copy(
            &inbox.handle,
            &[inbox.members[0].0],
            "Private copies",
            false,
        )
        .await
        .unwrap();
    let copied = session.open("Private copies", false).await.unwrap();
    session
        .store(
            &copied.handle,
            &[copied.members[0].0],
            &ImapFlagChange {
                operation: ImapFlagOperation::Add,
                flags: ImapMessageFlags {
                    seen: true,
                    flagged: true,
                    keywords: vec!["Private".into()],
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();
    assert!(session.open("INBOX", true).await.unwrap().members[0].1.seen);
    fixture
        .storage
        .set_imap_subscription(identifier, "Private copies", true)
        .await
        .unwrap();
    fixture
        .storage
        .set_imap_subscription(identifier, "Private matches", false)
        .await
        .unwrap();
    let folders = fixture
        .storage
        .list_imap_local_folders(identifier)
        .await
        .unwrap();
    assert_eq!(folders.len(), 2);
    caller.delete_imap_account(identifier).await.unwrap();
    assert!(matches!(
        session.validate().await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(
        fixture
            .storage
            .get_imap_account(identifier)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .storage
            .get_imap_account_credential(identifier)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        fixture
            .storage
            .list_imap_local_folders(identifier)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .storage
            .get_imap_local_folder(smart.identifier)
            .await
            .unwrap()
            .is_none()
    );
    for folder in folders {
        assert!(
            fixture
                .storage
                .get_imap_local_folder(folder.identifier)
                .await
                .unwrap()
                .is_none()
        );
    }
    for handle in [inbox.handle, copied.handle] {
        assert!(
            fixture
                .storage
                .get_imap_mailbox(handle.identifier)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            fixture
                .storage
                .imap_mailbox_members(handle.identifier, 0)
                .await
                .unwrap()
                .is_empty()
        );
    }
    assert!(
        fixture
            .storage
            .list_imap_subscribed(identifier)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .storage
            .list_imap_unsubscribed(identifier)
            .await
            .unwrap()
            .is_empty()
    );
    for surviving in [&peer_session, &mirror_session] {
        assert_eq!(
            subjects(&fixture, surviving, "INBOX").await,
            ["shared content"]
        );
        let remaining = surviving.open("INBOX", true).await.unwrap();
        assert_eq!(remaining.members[0].1, ImapMessageFlags::default());
        assert!(matches!(
            surviving.open("Private copies", true).await,
            Err(ImapAccessError::NonExistent)
        ));
    }
    assert!(
        fixture
            .storage
            .get_message(message)
            .await
            .unwrap()
            .is_some()
    );
}

/// Shared mutations notify Mirror observers, whereas independent mutations notify only that account.
#[tokio::test]
async fn organization_mutations_publish_only_their_mode_notification() {
    for mode in [ImapAccountMode::Independent, ImapAccountMode::Mirror] {
        let fixture = Fixture::new().await;
        let user = fixture
            .user("Member", GlobalRole::Member, MailAccess::All)
            .await;
        fixture.mail("retained", "app.test").await;
        let issued = fixture.account(&user, mode).await;
        let session = fixture.login(&issued).await;
        let mut changes = fixture.application.subscribe_changes();
        let expected = if mode == ImapAccountMode::Mirror {
            ApplicationChange::Visibility
        } else {
            ApplicationChange::ImapAccount(issued.account.account.identifier)
        };
        session.create_mailbox("Folder").await.unwrap();
        assert_single_change(&mut changes, expected);
        session.rename_mailbox("Folder", "Renamed").await.unwrap();
        assert_single_change(&mut changes, expected);
        let inbox = session.open("INBOX", false).await.unwrap();
        session
            .store(
                &inbox.handle,
                &[inbox.members[0].0],
                &ImapFlagChange {
                    operation: ImapFlagOperation::Add,
                    flags: ImapMessageFlags {
                        seen: true,
                        ..Default::default()
                    },
                },
            )
            .await
            .unwrap();
        assert_single_change(&mut changes, expected);
        session
            .copy(&inbox.handle, &[inbox.members[0].0], "Renamed", true)
            .await
            .unwrap();
        assert_single_change(&mut changes, expected);
        session.delete_mailbox("Renamed").await.unwrap();
        assert_single_change(&mut changes, expected);
        let trash = session.open("Trash", false).await.unwrap();
        session
            .store(
                &trash.handle,
                &[trash.members[0].0],
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
        assert_single_change(&mut changes, expected);
        session.expunge(&trash.handle, None).await.unwrap();
        assert_single_change(&mut changes, expected);
    }
}

/// Final Trash EXPUNGE physically collects content when every other view has released it.
#[tokio::test]
async fn final_independent_trash_expunge_collects_the_last_reference() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    let message = fixture.mail("last reference", "app.test").await;
    let issued = fixture.account(&user, ImapAccountMode::Independent).await;
    let session = fixture.login(&issued).await;
    fixture
        .as_user(&user)
        .delete_message(message)
        .await
        .unwrap();
    assert!(
        fixture
            .storage
            .get_message(message)
            .await
            .unwrap()
            .is_some()
    );
    expunge_all(&session, "INBOX").await;
    let trash = session.open("Trash", true).await.unwrap();
    assert_eq!(trash.members.len(), 1);
    expunge_all(&session, "Trash").await;
    assert!(
        fixture
            .storage
            .get_message(message)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        session
            .content(&trash.handle, trash.members[0].0)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        session
            .metadata(&trash.handle, &[trash.members[0].0])
            .await
            .unwrap()
            .is_empty()
    );
    fixture
        .as_user(&user)
        .restore_imap_hidden_messages(issued.account.account.identifier)
        .await
        .unwrap();
    assert!(subjects(&fixture, &session, "INBOX").await.is_empty());
    assert!(subjects(&fixture, &session, "Trash").await.is_empty());
}

/// Derived search content survives retained Independent views and disappears after final-reference collection.
#[tokio::test]
async fn final_view_release_removes_the_physical_search_document() {
    let fixture = Fixture::new().await;
    let user = fixture
        .user("Retainer", GlobalRole::Member, MailAccess::All)
        .await;
    let identifier = fixture.mail("Indexed retained content", "app.test").await;
    let issued = fixture.account(&user, ImapAccountMode::Independent).await;
    let query = sandpost_search::MessageQuery {
        authorization: None,
        filter: sandpost_query::Expression::True,
        message_identifier: Some(identifier),
        before: None,
        limit: 1,
    };
    fixture.application.search.catch_up().await.unwrap();
    assert_eq!(
        fixture.application.search.search(&query).unwrap(),
        [identifier]
    );
    fixture
        .as_user(&user)
        .delete_message(identifier)
        .await
        .unwrap();
    fixture.application.search.catch_up().await.unwrap();
    assert_eq!(
        fixture.application.search.search(&query).unwrap(),
        [identifier],
        "primary removal must retain the shared document for Independent readers"
    );
    let session = fixture.login(&issued).await;
    expunge_all(&session, "INBOX").await;
    fixture.application.search.catch_up().await.unwrap();
    assert_eq!(
        fixture.application.search.search(&query).unwrap(),
        [identifier]
    );
    expunge_all(&session, "Trash").await;
    fixture.application.search.catch_up().await.unwrap();
    assert!(
        fixture
            .application
            .search
            .search(&query)
            .unwrap()
            .is_empty(),
        "collection deletes the actual Tantivy document, not only user-visible results"
    );
    assert_eq!(
        fixture
            .application
            .search
            .status()
            .await
            .unwrap()
            .pending_operations,
        0
    );
    assert!(
        fixture
            .storage
            .get_message(identifier)
            .await
            .unwrap()
            .is_none()
    );
}
