//! IMAP account management and session authorization tests over in-memory SQLite.
use super::*;
use crate::{Application, ApplicationError, MessageEvent, SessionUser};
use sandpost_core::{
    GlobalRole, ImapAccountMode, ImapLinkedView, ImapMessageFlags, MailAccess, Mailbox, Message,
    MessageFacts, MessageIdentifier, Scope, ScopeIdentifier, User, UserIdentifier, View,
    ViewIdentifier,
};
use sandpost_storage::{ImapFlagChange, ImapFlagOperation, NewUser, Storage, UpdateUser};
use sandpost_storage_sqlite::SqliteStorage;
use std::sync::Arc;
use tokio::sync::broadcast;
mod generations;
mod handle_access;
mod mailbox;
mod modes;
mod subscriptions;

/// A storage, application, and helpers shared by the tests.
struct Fixture {
    storage: Arc<dyn Storage>,
    application: Application,
}

impl Fixture {
    /// Create an empty installation.
    async fn new() -> Self {
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::memory().await.unwrap());
        let (events, _) = broadcast::channel::<MessageEvent>(64);
        let application = Application::new(storage.clone(), events).await.unwrap();
        Self {
            storage,
            application,
        }
    }

    /// Create a user with a role and mail access.
    async fn user(&self, name: &str, role: GlobalRole, mail_access: MailAccess) -> User {
        self.storage
            .create_user(&NewUser {
                name: name.into(),
                email: format!(
                    "{}-{}@example.test",
                    name.to_lowercase(),
                    UserIdentifier::new()
                ),
                password_hash: "hash".into(),
                global_role: role,
                mail_access,
                personal_filter: None,
            })
            .await
            .unwrap()
    }

    /// Capture a message with a subject and recipient domain.
    async fn mail(&self, subject: &str, domain: &str) -> MessageIdentifier {
        let identifier = MessageIdentifier::new();
        let raw = format!("Subject: {subject}\r\nTo: dev@{domain}\r\n\r\nbody\r\n");
        self.storage
            .insert_message(&Message {
                identifier,
                facts: MessageFacts {
                    subject: subject.into(),
                    envelope_to: vec![Mailbox {
                        address: format!("dev@{domain}"),
                        domain: domain.into(),
                    }],
                    size: raw.len() as u64,
                    ..MessageFacts::default()
                },
                raw_message: raw.into_bytes(),
                attachments: Vec::new(),
            })
            .await
            .unwrap();
        identifier
    }

    /// Save a scope and assign it to a user.
    async fn scope(&self, user: &User, filter: &str) -> ScopeIdentifier {
        let identifier = ScopeIdentifier::new();
        self.storage
            .save_scope(&Scope {
                identifier,
                parent: None,
                name: "Scope".into(),
                description: None,
                filter: filter.into(),
                position: 0,
                policy_version: 1,
            })
            .await
            .unwrap();
        self.storage
            .assign_user_to_scope(user.identifier, identifier)
            .await
            .unwrap();
        identifier
    }

    /// Save a personal View for a user.
    async fn view(&self, owner: &User, name: &str, filter: &str) -> View {
        let view = View {
            identifier: ViewIdentifier::new(),
            owner_identifier: Some(owner.identifier),
            name: name.into(),
            icon: None,
            filter: filter.into(),
        };
        self.storage.save_view(&view).await.unwrap();
        view
    }

    /// The authorized application of a user.
    fn as_user(&self, user: &User) -> crate::AuthorizedApplication {
        self.application.authorized(&SessionUser {
            id: user.identifier,
            name: user.name.clone(),
            email: user.email.clone(),
            role: user.global_role,
        })
    }

    /// Create an IMAP account for a user and return its credentials.
    async fn account(&self, user: &User, mode: ImapAccountMode) -> IssuedImapCredentials {
        self.as_user(user)
            .create_imap_account(NewImapAccountRequest {
                name: "Debug".into(),
                username: None,
                mode: Some(mode),
                mirror_views: true,
                linked_views: Vec::new(),
            })
            .await
            .unwrap()
    }

    /// Log in through the IMAP service, expecting success.
    async fn login(&self, issued: &IssuedImapCredentials) -> ImapAccountSession {
        match self
            .application
            .imap_service()
            .authenticate(&issued.account.account.username, &issued.password, None)
            .await
            .unwrap()
        {
            ImapLogin::Authenticated(session) => session,
            _ => panic!("login failed"),
        }
    }
}

/// Subjects of a mailbox, in UID order, after opening it.
async fn subjects(fixture: &Fixture, session: &ImapAccountSession, name: &str) -> Vec<String> {
    let snapshot = session.open(name, true).await.unwrap();
    let mut subjects = Vec::new();
    for (uid, _, _) in snapshot.members {
        let content = session
            .content(&snapshot.handle, uid)
            .await
            .unwrap()
            .unwrap();
        let text = String::from_utf8(content.raw_message).unwrap();
        subjects.push(
            text.lines()
                .next()
                .unwrap()
                .trim_start_matches("Subject: ")
                .to_owned(),
        );
    }
    let _ = fixture;
    subjects
}

/// An account never exposes mail its owner cannot read, and revoking a scope removes messages
/// at the next synchronization without renumbering the rest.
#[tokio::test]
async fn mailboxes_never_exceed_owner_permissions() {
    let fixture = Fixture::new().await;
    let member = fixture
        .user("Member", GlobalRole::Member, MailAccess::Scoped)
        .await;
    let alpha = fixture
        .scope(&member, "envelope.to.domain == \"alpha.test\"")
        .await;
    let beta = fixture
        .scope(&member, "envelope.to.domain == \"beta.test\"")
        .await;
    fixture.mail("alpha one", "alpha.test").await;
    fixture.mail("beta one", "beta.test").await;
    fixture.mail("gamma secret", "gamma.test").await;
    let issued = fixture.account(&member, ImapAccountMode::Mirror).await;
    let session = fixture.login(&issued).await;
    assert_eq!(
        subjects(&fixture, &session, "INBOX").await,
        ["alpha one", "beta one"]
    );
    let snapshot = session.open("INBOX", false).await.unwrap();
    assert_eq!(
        snapshot
            .members
            .iter()
            .map(|member| member.0)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    fixture
        .storage
        .remove_user_from_scope(member.identifier, alpha)
        .await
        .unwrap();
    let changes = session
        .synchronize(&snapshot.handle, 2, 2, snapshot.cursor, true)
        .await
        .unwrap();
    assert_eq!(
        changes.present_uids,
        Some(vec![2]),
        "alpha mail is no longer visible"
    );
    assert!(changes.added.is_empty());
    assert!(
        session
            .content(&snapshot.handle, 1)
            .await
            .unwrap()
            .is_none(),
        "a direct UID read cannot bypass authorization"
    );
    fixture
        .storage
        .remove_user_from_scope(member.identifier, beta)
        .await
        .unwrap();
    assert!(
        subjects(&fixture, &session, "INBOX").await.is_empty(),
        "no scope, no mail"
    );
    assert!(subjects(&fixture, &session, "Trash").await.is_empty());
}

/// Views are mirrored dynamically: new, renamed, and deleted Views change the mailbox list, and
/// a deleted View's selected mailbox disappears.
#[tokio::test]
async fn mirror_mode_follows_views() {
    let fixture = Fixture::new().await;
    let owner = fixture
        .user("Owner", GlobalRole::Owner, MailAccess::All)
        .await;
    fixture.mail("Laravel welcome", "app.test").await;
    fixture.mail("Billing invoice", "app.test").await;
    let laravel = fixture
        .view(&owner, "Laravel", "subject contains \"Laravel\"")
        .await;
    let issued = fixture.account(&owner, ImapAccountMode::Mirror).await;
    let session = fixture.login(&issued).await;
    let names: Vec<String> = session
        .mailboxes()
        .await
        .unwrap()
        .into_iter()
        .map(|listing| listing.name)
        .collect();
    assert_eq!(names, ["INBOX", "Trash", "Laravel"]);
    assert_eq!(
        subjects(&fixture, &session, "Laravel").await,
        ["Laravel welcome"]
    );
    assert_eq!(subjects(&fixture, &session, "INBOX").await.len(), 2);
    let opened = session.open("Laravel", false).await.unwrap();
    fixture
        .storage
        .save_view(&View {
            name: "Framework".into(),
            icon: None,
            filter: "subject contains \"Billing\"".into(),
            ..laravel.clone()
        })
        .await
        .unwrap();
    let renamed: Vec<String> = session
        .mailboxes()
        .await
        .unwrap()
        .into_iter()
        .filter(|listing| listing.selectable)
        .map(|listing| listing.name)
        .collect();
    assert_eq!(renamed, ["INBOX", "Trash", "Framework"]);
    let reopened = session.open("Framework", true).await.unwrap();
    assert_eq!(
        reopened.uid_validity, opened.uid_validity,
        "a rename keeps UIDVALIDITY"
    );
    assert_eq!(
        subjects(&fixture, &session, "Framework").await,
        ["Billing invoice"]
    );
    assert_eq!(
        reopened
            .members
            .iter()
            .map(|member| member.0)
            .collect::<Vec<_>>(),
        [2],
        "a filter change assigns new UIDs instead of renumbering"
    );
    fixture
        .storage
        .delete_view(laravel.identifier)
        .await
        .unwrap();
    assert!(matches!(
        session.synchronize(&opened.handle, 2, 1, 0, false).await,
        Err(ImapAccessError::NonExistent)
    ));
    assert!(matches!(
        session.open("Framework", true).await,
        Err(ImapAccessError::NonExistent)
    ));
}

/// Independent ignores legacy linked Views and keeps smart definitions private to each account.
#[tokio::test]
async fn independent_mode_ignores_legacy_views_and_keeps_smart_definitions_account_local() {
    let fixture = Fixture::new().await;
    let owner = fixture
        .user("Owner", GlobalRole::Owner, MailAccess::All)
        .await;
    fixture.mail("Laravel welcome", "app.test").await;
    fixture.mail("Password reset", "app.test").await;
    fixture.mail("Unrelated", "app.test").await;
    let laravel = fixture
        .view(&owner, "Laravel", "subject contains \"Laravel\"")
        .await;
    fixture.view(&owner, "Hidden", "true").await;
    let caller = fixture.as_user(&owner);
    let issued = caller
        .create_imap_account(NewImapAccountRequest {
            name: "Project Testing".into(),
            username: Some("project.testing".into()),
            mode: Some(ImapAccountMode::Independent),
            mirror_views: false,
            linked_views: vec![
                ImapLinkedView {
                    view_identifier: laravel.identifier,
                    alias: Some("Framework".into()),
                },
                ImapLinkedView {
                    view_identifier: ViewIdentifier::new(),
                    alias: Some("Unavailable legacy link".into()),
                },
            ],
        })
        .await
        .unwrap();
    assert_eq!(issued.account.account.username, "project.testing");
    assert!(issued.account.linked_views.is_empty());
    let folder = caller
        .create_imap_folder(
            issued.account.account.identifier,
            ImapFolderRequest {
                name: "Password Reset Emails".into(),
                filter: "subject contains \"reset\"".into(),
                include_in_inbox: true,
            },
        )
        .await
        .unwrap();
    let session = fixture.login(&issued).await;
    let names: Vec<String> = session
        .mailboxes()
        .await
        .unwrap()
        .into_iter()
        .map(|listing| listing.name)
        .collect();
    assert_eq!(names, ["INBOX", "Trash", "Password Reset Emails"]);
    assert_eq!(folder.kind, sandpost_core::ImapFolderKind::Smart);
    let sibling = fixture.account(&owner, ImapAccountMode::Independent).await;
    let sibling_session = fixture.login(&sibling).await;
    assert!(matches!(
        sibling_session.open("Password Reset Emails", true).await,
        Err(ImapAccessError::NonExistent)
    ));
    assert_eq!(
        subjects(&fixture, &session, "Password Reset Emails").await,
        ["Password reset"]
    );
    assert_eq!(
        subjects(&fixture, &session, "INBOX").await,
        ["Laravel welcome", "Password reset", "Unrelated"]
    );
    caller
        .update_imap_folder(
            issued.account.account.identifier,
            folder.identifier,
            ImapFolderRequest {
                name: "Password Reset Emails".into(),
                filter: "subject contains \"reset\"".into(),
                include_in_inbox: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        subjects(&fixture, &session, "INBOX").await,
        ["Laravel welcome", "Password reset", "Unrelated"]
    );
    assert_eq!(
        caller.list_views().await.unwrap().len(),
        2,
        "local folders are not Sandpost Views"
    );
    let invalid = caller
        .create_imap_folder(
            issued.account.account.identifier,
            ImapFolderRequest {
                name: "Broken".into(),
                filter: "subject ===".into(),
                include_in_inbox: true,
            },
        )
        .await;
    assert!(matches!(invalid, Err(ApplicationError::InvalidInput(_))));
}

/// Switching modes revokes old sessions and keeps the independent folder catalog dormant.
#[tokio::test]
async fn mode_changes_revoke_sessions_and_preserve_independent_folders() {
    let fixture = Fixture::new().await;
    let owner = fixture
        .user("Owner", GlobalRole::Owner, MailAccess::All)
        .await;
    let caller = fixture.as_user(&owner);
    let issued = caller
        .create_imap_account(NewImapAccountRequest {
            name: "Independent".into(),
            username: Some("mode.test".into()),
            mode: Some(ImapAccountMode::Independent),
            mirror_views: true,
            linked_views: Vec::new(),
        })
        .await
        .unwrap();
    let identifier = issued.account.account.identifier;
    let old_session = fixture.login(&issued).await;
    old_session.create_mailbox("Private").await.unwrap();

    caller
        .update_imap_account(
            identifier,
            ImapAccountUpdate {
                mode: Some(ImapAccountMode::Mirror),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        old_session.validate().await,
        Err(ImapAccessError::Revoked)
    ));
    caller
        .update_imap_account(
            identifier,
            ImapAccountUpdate {
                mode: Some(ImapAccountMode::Independent),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        old_session.validate().await,
        Err(ImapAccessError::Revoked)
    ));
    let current = fixture.login(&issued).await;
    let names: Vec<_> = current
        .mailboxes()
        .await
        .unwrap()
        .into_iter()
        .filter(|mailbox| mailbox.selectable)
        .map(|mailbox| mailbox.name)
        .collect();
    assert_eq!(names, ["INBOX", "Trash", "Private"]);
}

/// Owner folders are shared across credentials and can be managed through any owner account.
#[tokio::test]
async fn local_folders_are_shared_across_owner_accounts() {
    let fixture = Fixture::new().await;
    let owner = fixture
        .user("Owner", GlobalRole::Owner, MailAccess::All)
        .await;
    let caller = fixture.as_user(&owner);
    let first = caller
        .create_imap_account(NewImapAccountRequest {
            name: "First".into(),
            username: Some("owner.first".into()),
            mode: None,
            mirror_views: true,
            linked_views: Vec::new(),
        })
        .await
        .unwrap();
    let second = caller
        .create_imap_account(NewImapAccountRequest {
            name: "Second".into(),
            username: Some("owner.second".into()),
            mode: None,
            mirror_views: true,
            linked_views: Vec::new(),
        })
        .await
        .unwrap();
    let first_id = first.account.account.identifier;
    let second_id = second.account.account.identifier;
    let created = caller
        .create_imap_folder(
            first_id,
            ImapFolderRequest {
                name: "Shared".into(),
                filter: "true".into(),
                include_in_inbox: true,
            },
        )
        .await
        .unwrap();
    let second_folder = caller
        .get_imap_account(second_id)
        .await
        .unwrap()
        .folders
        .into_iter()
        .find(|folder| folder.name == "Shared")
        .expect("second credential discovers owner folder");
    assert_ne!(created.identifier, second_folder.identifier);

    caller
        .update_imap_folder(
            second_id,
            second_folder.identifier,
            ImapFolderRequest {
                name: "Renamed Shared".into(),
                filter: "subject contains \"shared\"".into(),
                include_in_inbox: false,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        caller.get_imap_account(first_id).await.unwrap().folders[0].name,
        "Renamed Shared"
    );

    caller
        .delete_imap_folder(second_id, second_folder.identifier)
        .await
        .unwrap();
    assert!(
        caller
            .get_imap_account(first_id)
            .await
            .unwrap()
            .folders
            .is_empty()
    );
}

/// Accounts belong to their owner: other users, including admins, cannot read or change them.
#[tokio::test]
async fn accounts_are_owned() {
    let fixture = Fixture::new().await;
    let owner = fixture
        .user("Owner", GlobalRole::Owner, MailAccess::All)
        .await;
    let member = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    let another_member = fixture
        .user("Another member", GlobalRole::Member, MailAccess::All)
        .await;
    let issued = fixture.account(&member, ImapAccountMode::Mirror).await;
    let identifier = issued.account.account.identifier;
    for intruder in [&owner, &another_member] {
        let caller = fixture.as_user(intruder);
        assert!(matches!(
            caller.get_imap_account(identifier).await,
            Err(ApplicationError::NotFound)
        ));
        assert!(matches!(
            caller
                .update_imap_account(
                    identifier,
                    ImapAccountUpdate {
                        enabled: Some(false),
                        ..ImapAccountUpdate::default()
                    }
                )
                .await,
            Err(ApplicationError::NotFound)
        ));
        assert!(matches!(
            caller.delete_imap_account(identifier).await,
            Err(ApplicationError::NotFound)
        ));
        assert!(caller.list_imap_accounts().await.unwrap().is_empty());
    }
    let other_view = fixture.view(&owner, "Owner only", "true").await;
    let linked = fixture
        .as_user(&member)
        .update_imap_account(
            identifier,
            ImapAccountUpdate {
                linked_views: Some(vec![ImapLinkedView {
                    view_identifier: other_view.identifier,
                    alias: None,
                }]),
                ..ImapAccountUpdate::default()
            },
        )
        .await;
    assert!(
        matches!(linked, Err(ApplicationError::InvalidInput(_))),
        "another user's personal View cannot be linked"
    );
}

/// Disabled accounts and wrong passwords fail; disabling, rotating, and deleting end running
/// sessions; mail survives account deletion.
#[tokio::test]
async fn credentials_and_revocation() {
    let fixture = Fixture::new().await;
    let member = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    let message = fixture.mail("kept", "app.test").await;
    let issued = fixture.account(&member, ImapAccountMode::Mirror).await;
    let identifier = issued.account.account.identifier;
    let service = fixture.application.imap_service();
    assert!(matches!(
        service
            .authenticate(&issued.account.account.username, "wrong password", None)
            .await
            .unwrap(),
        ImapLogin::Invalid
    ));
    let session = fixture.login(&issued).await;
    let caller = fixture.as_user(&member);
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
    assert!(matches!(
        session.validate().await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(matches!(
        service
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
    let session = fixture.login(&issued).await;
    let rotated = caller
        .regenerate_imap_account_password(identifier)
        .await
        .unwrap();
    assert_ne!(rotated.password, issued.password);
    assert!(matches!(
        session.validate().await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(matches!(
        service
            .authenticate(&issued.account.account.username, &issued.password, None)
            .await
            .unwrap(),
        ImapLogin::Invalid
    ));
    let session = fixture.login(&rotated).await;
    session.validate().await.unwrap();
    caller.delete_imap_account(identifier).await.unwrap();
    assert!(matches!(
        session.validate().await,
        Err(ImapAccessError::Revoked)
    ));
    assert!(
        fixture
            .storage
            .get_message(message)
            .await
            .unwrap()
            .is_some(),
        "deleting an IMAP account keeps mail"
    );
}

/// Deleting the owning user ends the account's sessions.
#[tokio::test]
async fn deleting_the_owner_revokes_sessions() {
    let fixture = Fixture::new().await;
    let member = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    let issued = fixture.account(&member, ImapAccountMode::Mirror).await;
    let session = fixture.login(&issued).await;
    fixture
        .storage
        .delete_user(member.identifier)
        .await
        .unwrap();
    assert!(matches!(
        session.validate().await,
        Err(ImapAccessError::Revoked)
    ));
}

/// Mirror shares primary placement and Trash release; expunging a copy preserves normal occurrences.
#[tokio::test]
async fn mirror_move_and_trash_expunge_preserve_other_views_and_normal_copies() {
    let fixture = Fixture::new().await;
    let member = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    let first = fixture.mail("first", "app.test").await;
    fixture.mail("second", "app.test").await;
    fixture
        .view(&member, "Firsts", "subject contains \"first\"")
        .await;
    let issued = fixture.account(&member, ImapAccountMode::Mirror).await;
    let second_account = fixture.account(&member, ImapAccountMode::Mirror).await;
    let other_user = fixture
        .user("Other", GlobalRole::Member, MailAccess::All)
        .await;
    let other_account = fixture.account(&other_user, ImapAccountMode::Mirror).await;
    let session = fixture.login(&issued).await;
    let second_session = fixture.login(&second_account).await;
    let other_session = fixture.login(&other_account).await;
    let _second_initial = second_session.open("INBOX", false).await.unwrap();
    let mut second_watch = second_session.watch();
    let inbox = session.open("INBOX", false).await.unwrap();
    let moved = session
        .copy(&inbox.handle, &[1], "Trash", true)
        .await
        .unwrap();
    assert_eq!(moved.removed_uids, [1]);
    assert_eq!(
        subjects(&fixture, &second_session, "INBOX").await,
        ["second"]
    );
    assert_eq!(subjects(&fixture, &session, "Trash").await, ["first"]);
    assert_eq!(subjects(&fixture, &session, "INBOX").await, ["second"]);
    assert!(
        subjects(&fixture, &session, "Firsts").await.is_empty(),
        "Trash suppresses smart results for every Mirror credential"
    );
    let refused = session.copy(&inbox.handle, &[2], "Firsts", false).await;
    assert!(matches!(refused, Err(ImapAccessError::NotPermitted(_))));
    let trash = session.open("Trash", false).await.unwrap();
    session
        .store(
            &trash.handle,
            &[1],
            &ImapFlagChange {
                operation: ImapFlagOperation::Add,
                flags: ImapMessageFlags {
                    deleted: true,
                    ..ImapMessageFlags::default()
                },
            },
        )
        .await
        .unwrap();
    session
        .store(
            &trash.handle,
            &[1],
            &ImapFlagChange {
                operation: ImapFlagOperation::Remove,
                flags: ImapMessageFlags {
                    deleted: true,
                    ..ImapMessageFlags::default()
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(subjects(&fixture, &session, "Trash").await, ["first"]);
    session
        .store(
            &trash.handle,
            &[1],
            &ImapFlagChange {
                operation: ImapFlagOperation::Add,
                flags: ImapMessageFlags {
                    deleted: true,
                    ..ImapMessageFlags::default()
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(session.expunge(&trash.handle, None).await.unwrap(), [1]);
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(1), second_watch.changed())
            .await
            .unwrap()
    );
    assert!(subjects(&fixture, &session, "Trash").await.is_empty());
    assert!(
        fixture
            .storage
            .get_user_message(member.identifier, first)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        subjects(&fixture, &second_session, "INBOX").await,
        ["second"]
    );
    assert_eq!(
        subjects(&fixture, &other_session, "INBOX").await,
        ["first", "second"]
    );

    // Expunging a Trash COPY removes that occurrence while the normal primary survives.
    let inbox = session.open("INBOX", false).await.unwrap();
    let copied = session
        .copy(&inbox.handle, &[2], "Trash", false)
        .await
        .unwrap();
    let copied_uid = copied.destination_uids[0];
    let trash = session.open("Trash", false).await.unwrap();
    session
        .store(
            &trash.handle,
            &[copied_uid],
            &ImapFlagChange {
                operation: ImapFlagOperation::Add,
                flags: ImapMessageFlags {
                    deleted: true,
                    ..ImapMessageFlags::default()
                },
            },
        )
        .await
        .unwrap();
    assert_eq!(
        session.expunge(&trash.handle, None).await.unwrap(),
        [copied_uid]
    );
    assert_eq!(
        subjects(&fixture, &second_session, "INBOX").await,
        ["second"]
    );
    assert_eq!(
        subjects(&fixture, &other_session, "INBOX").await,
        ["first", "second"]
    );
    assert!(
        fixture.storage.get_message(first).await.unwrap().is_some(),
        "other views and dormant independent state retain physical mail"
    );
    assert_eq!(subjects(&fixture, &session, "INBOX").await, ["second"]);
}

/**
 * A Viewer can read without changing flags, subscriptions, mail, or folder configuration.
 */
#[tokio::test]
async fn viewer_is_read_only_for_imap_mail_and_configuration() {
    for mode in [ImapAccountMode::Mirror, ImapAccountMode::Independent] {
        let fixture = Fixture::new().await;
        let viewer = fixture
            .user("Viewer", GlobalRole::Member, MailAccess::All)
            .await;
        fixture.mail("kept", "app.test").await;
        let account = fixture.account(&viewer, mode).await;
        fixture
            .storage
            .update_user(
                viewer.identifier,
                &UpdateUser {
                    name: viewer.name.clone(),
                    email: viewer.email.clone(),
                    password_hash: viewer.password_hash.clone(),
                    global_role: GlobalRole::Viewer,
                    mail_access: viewer.mail_access,
                    personal_filter: viewer.personal_filter.clone(),
                },
            )
            .await
            .unwrap()
            .unwrap();
        let session = fixture.login(&account).await;
        let inbox = session.open("INBOX", false).await.unwrap();
        assert!(inbox.read_only);
        assert!(matches!(
            session
                .store(
                    &inbox.handle,
                    &[1],
                    &ImapFlagChange {
                        operation: ImapFlagOperation::Add,
                        flags: ImapMessageFlags {
                            seen: true,
                            ..ImapMessageFlags::default()
                        },
                    }
                )
                .await,
            Err(ImapAccessError::NotPermitted(_))
        ));
        assert!(matches!(
            session.expunge(&inbox.handle, None).await,
            Err(ImapAccessError::NotPermitted(_))
        ));
        assert_eq!(subjects(&fixture, &session, "INBOX").await, ["kept"]);
        assert!(!session.open("INBOX", true).await.unwrap().members[0].1.seen);
        assert!(matches!(
            session.copy(&inbox.handle, &[1], "Trash", true).await,
            Err(ImapAccessError::NotPermitted(_))
        ));
        assert!(matches!(
            session.set_subscription("INBOX", true).await,
            Err(ImapAccessError::NotPermitted(_))
        ));
        assert!(matches!(
            session.create_mailbox("Review").await,
            Err(ImapAccessError::NotPermitted(_))
        ));
        assert!(matches!(
            session.rename_mailbox("INBOX", "Review").await,
            Err(ImapAccessError::NotPermitted(_))
        ));
        assert!(subjects(&fixture, &session, "Trash").await.is_empty());
        assert!(subjects(&fixture, &session, "INBOX").await == ["kept"]);
    }
}

/// Seen and other shared flags follow a message across the account's mailboxes.
#[tokio::test]
async fn flags_are_shared_across_mailboxes() {
    let fixture = Fixture::new().await;
    let member = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    fixture.mail("first", "app.test").await;
    fixture.view(&member, "All", "true").await;
    let issued = fixture.account(&member, ImapAccountMode::Mirror).await;
    let session = fixture.login(&issued).await;
    let inbox = session.open("INBOX", false).await.unwrap();
    let all = session.open("All", false).await.unwrap();
    session
        .store(
            &inbox.handle,
            &[1],
            &ImapFlagChange {
                operation: ImapFlagOperation::Add,
                flags: ImapMessageFlags {
                    seen: true,
                    ..ImapMessageFlags::default()
                },
            },
        )
        .await
        .unwrap();
    let changes = session
        .synchronize(&all.handle, 1, 1, all.cursor, false)
        .await
        .unwrap();
    assert_eq!(changes.flag_updates.len(), 1);
    assert!(changes.flag_updates[0].1.seen);
    let status = session.status("All").await.unwrap();
    assert_eq!((status.messages, status.unseen), (1, 0));
}

/// A second user's account never sees the first user's scoped mail, and new mail arrives with
/// ascending UIDs and as recent for the claiming session.
#[tokio::test]
async fn accounts_are_isolated_and_new_mail_is_recent() {
    let fixture = Fixture::new().await;
    let alice = fixture
        .user("Alice", GlobalRole::Member, MailAccess::Scoped)
        .await;
    let bob = fixture
        .user("Bob", GlobalRole::Member, MailAccess::Scoped)
        .await;
    fixture
        .scope(&alice, "envelope.to.domain == \"alice.test\"")
        .await;
    fixture
        .scope(&bob, "envelope.to.domain == \"bob.test\"")
        .await;
    fixture.mail("for alice", "alice.test").await;
    let alice_session = fixture
        .login(&fixture.account(&alice, ImapAccountMode::Mirror).await)
        .await;
    let bob_session = fixture
        .login(&fixture.account(&bob, ImapAccountMode::Mirror).await)
        .await;
    assert_eq!(
        subjects(&fixture, &alice_session, "INBOX").await,
        ["for alice"]
    );
    assert!(subjects(&fixture, &bob_session, "INBOX").await.is_empty());
    let opened = alice_session.open("INBOX", false).await.unwrap();
    fixture.mail("second for alice", "alice.test").await;
    fixture.mail("for bob", "bob.test").await;
    let changes = alice_session
        .synchronize(&opened.handle, 1, 1, opened.cursor, true)
        .await
        .unwrap();
    assert_eq!(changes.added.len(), 1);
    assert_eq!(changes.added[0].0, 2);
    assert!(
        changes.added[0].2,
        "new mail is recent for the read-write session"
    );
    assert_eq!(changes.uid_next, 3);
}

/// Repeated failures are throttled before verification.
#[tokio::test]
async fn failed_logins_are_throttled() {
    let fixture = Fixture::new().await;
    let member = fixture
        .user("Member", GlobalRole::Member, MailAccess::All)
        .await;
    let issued = fixture.account(&member, ImapAccountMode::Mirror).await;
    let service = fixture.application.imap_service();
    let address = Some("203.0.113.9".parse().unwrap());
    for _ in 0..20 {
        assert!(matches!(
            service
                .authenticate("nobody", "wrong", address)
                .await
                .unwrap(),
            ImapLogin::Invalid
        ));
    }
    assert!(matches!(
        service
            .authenticate(&issued.account.account.username, &issued.password, address)
            .await
            .unwrap(),
        ImapLogin::Throttled
    ));
    assert!(matches!(
        service
            .authenticate(&issued.account.account.username, &issued.password, None)
            .await
            .unwrap(),
        ImapLogin::Authenticated(_)
    ));
}

/**
 * Promotion to Owner preserves current scoped visibility in Mirror and Independent IMAP sessions.
 */
#[tokio::test]
async fn owner_promotion_does_not_expand_imap_mail_access() {
    for mode in [ImapAccountMode::Mirror, ImapAccountMode::Independent] {
        let fixture = Fixture::new().await;
        let user = fixture
            .user("Scoped", GlobalRole::Member, MailAccess::Scoped)
            .await;
        let scope = fixture
            .scope(&user, "envelope.to.domain == \"team.test\"")
            .await;
        fixture.mail("Allowed", "team.test").await;
        fixture.mail("Hidden", "other.test").await;
        let issued = fixture.account(&user, mode).await;
        let session = fixture.login(&issued).await;
        assert_eq!(subjects(&fixture, &session, "INBOX").await, ["Allowed"]);
        fixture
            .storage
            .update_user(
                user.identifier,
                &UpdateUser {
                    name: user.name.clone(),
                    email: user.email.clone(),
                    password_hash: user.password_hash.clone(),
                    global_role: GlobalRole::Owner,
                    mail_access: user.mail_access,
                    personal_filter: user.personal_filter.clone(),
                },
            )
            .await
            .unwrap();
        assert_eq!(subjects(&fixture, &session, "INBOX").await, ["Allowed"]);
        fixture
            .storage
            .remove_user_from_scope(user.identifier, scope)
            .await
            .unwrap();
        assert!(subjects(&fixture, &session, "INBOX").await.is_empty());
    }
}
