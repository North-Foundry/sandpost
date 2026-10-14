//! Session revocation remains durable across mode and enabled-state round trips.
use super::*;

/// Verify old sessions cannot return view data or apply message and folder mutations.
async fn assert_session_generation_revoked(
    session: &ImapAccountSession,
    snapshot: &ImapMailboxSnapshot,
) {
    let handle = &snapshot.handle;
    let uid = snapshot.members[0].0;
    let seen = ImapFlagChange {
        operation: ImapFlagOperation::Add,
        flags: ImapMessageFlags {
            seen: true,
            ..ImapMessageFlags::default()
        },
    };
    assert!(matches!(
        session.mailboxes().await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(matches!(
        session.open("INBOX", true).await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(matches!(
        session.status("INBOX").await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(matches!(
        session
            .synchronize(handle, uid, 1, snapshot.cursor, false)
            .await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(matches!(
        session.content(handle, uid).await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(matches!(
        session.metadata(handle, &[uid]).await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(matches!(
        session.store(handle, &[uid], &seen).await,
        Err(ImapAccessError::Revoked)
    ));
    for remove_source in [false, true] {
        assert!(matches!(
            session
                .copy(handle, &[uid], "Projects", remove_source)
                .await,
            Err(ImapAccessError::Revoked)
        ));
    }
    assert!(matches!(
        session.expunge(handle, Some(&[uid])).await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(matches!(
        session.create_mailbox("Stale CREATE").await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(matches!(
        session.rename_mailbox("Projects", "Stale RENAME").await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(matches!(
        session.delete_mailbox("Projects").await,
        Err(ImapAccessError::Revoked)
    ));
}

/// Restoring a mode or reenabling an account does not reactivate its earlier authenticated sessions.
#[tokio::test]
async fn mode_and_enabled_round_trips_revoke_every_session_operation() {
    for mode in [ImapAccountMode::Mirror, ImapAccountMode::Independent] {
        for change_mode in [false, true] {
            let fixture = Fixture::new().await;
            let owner = fixture
                .user("Member", GlobalRole::Member, MailAccess::All)
                .await;
            fixture.mail("Original message", "example.test").await;
            let issued = fixture.account(&owner, mode).await;
            let session = fixture.login(&issued).await;
            session.create_mailbox("Projects").await.unwrap();
            let original = session.open("INBOX", false).await.unwrap();
            let caller = fixture.as_user(&owner);
            let away = if mode == ImapAccountMode::Mirror {
                ImapAccountMode::Independent
            } else {
                ImapAccountMode::Mirror
            };
            for restore in [false, true] {
                caller
                    .update_imap_account(
                        issued.account.account.identifier,
                        ImapAccountUpdate {
                            mode: change_mode.then_some(if restore { mode } else { away }),
                            enabled: (!change_mode).then_some(restore),
                            ..ImapAccountUpdate::default()
                        },
                    )
                    .await
                    .unwrap();
            }
            assert_session_generation_revoked(&session, &original).await;
            let current = fixture.login(&issued).await;
            let remaining = current.open("INBOX", true).await.unwrap();
            assert_eq!(remaining.members.len(), original.members.len());
            for (current, original) in remaining.members.iter().zip(&original.members) {
                assert_eq!((&current.0, &current.1), (&original.0, &original.1));
            }
            assert_eq!(remaining.uid_validity, original.uid_validity);
            let folders = current.mailboxes().await.unwrap();
            assert!(folders.iter().any(|folder| folder.name == "Projects"));
            assert!(
                !folders
                    .iter()
                    .any(|folder| folder.name.starts_with("Stale"))
            );
            assert!(
                current
                    .open("Trash", true)
                    .await
                    .unwrap()
                    .members
                    .is_empty()
            );
        }
    }
}

/// Web owners can manage disabled credentials' smart definitions while their IMAP sessions stay revoked.
#[tokio::test]
async fn disabled_credentials_allow_owner_smart_configuration_but_revoke_sessions() {
    for mode in [ImapAccountMode::Mirror, ImapAccountMode::Independent] {
        let fixture = Fixture::new().await;
        let owner = fixture
            .user("Member", GlobalRole::Member, MailAccess::All)
            .await;
        fixture.mail("Retained message", "example.test").await;
        let issued = fixture.account(&owner, mode).await;
        let identifier = issued.account.account.identifier;
        let session = fixture.login(&issued).await;
        session.create_mailbox("Projects").await.unwrap();
        let original = session.open("INBOX", true).await.unwrap();
        let caller = fixture.as_user(&owner);
        let existing = caller
            .create_imap_folder(
                identifier,
                ImapFolderRequest {
                    name: "Existing smart".into(),
                    filter: "true".into(),
                    include_in_inbox: false,
                },
            )
            .await
            .unwrap();
        caller
            .update_imap_account(
                identifier,
                ImapAccountUpdate {
                    enabled: Some(false),
                    ..ImapAccountUpdate::default()
                },
            )
            .await
            .unwrap();

        let created = caller
            .create_imap_folder(
                identifier,
                ImapFolderRequest {
                    name: "Created while disabled".into(),
                    filter: "true".into(),
                    include_in_inbox: false,
                },
            )
            .await
            .unwrap();
        for (folder, name) in [
            (existing.identifier, "Edited existing"),
            (created.identifier, "Edited disabled"),
        ] {
            let updated = caller
                .update_imap_folder(
                    identifier,
                    folder,
                    ImapFolderRequest {
                        name: name.into(),
                        filter: "subject contains \"Retained\"".into(),
                        include_in_inbox: false,
                    },
                )
                .await
                .unwrap();
            assert_eq!(updated.name, name);
            assert_eq!(updated.filter, "subject contains \"Retained\"");
            caller.delete_imap_folder(identifier, folder).await.unwrap();
        }
        let details = caller.get_imap_account(identifier).await.unwrap();
        assert!(!details.account.enabled);
        assert!(
            details
                .folders
                .iter()
                .all(|folder| folder.kind != sandpost_core::ImapFolderKind::Smart)
        );
        assert_session_generation_revoked(&session, &original).await;
        assert!(matches!(
            fixture
                .application
                .imap_service()
                .authenticate(&issued.account.account.username, &issued.password, None)
                .await
                .unwrap(),
            ImapLogin::Invalid
        ));

        caller
            .update_imap_account(
                identifier,
                ImapAccountUpdate {
                    enabled: Some(true),
                    ..ImapAccountUpdate::default()
                },
            )
            .await
            .unwrap();
        assert_session_generation_revoked(&session, &original).await;
        let fresh = fixture.login(&issued).await;
        let remaining = fresh.open("INBOX", true).await.unwrap();
        assert_eq!(remaining.members, original.members);
        assert_eq!(remaining.uid_validity, original.uid_validity);
    }
}
