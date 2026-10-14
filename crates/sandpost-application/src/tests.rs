//! Application-level tests for authorization composition, administration, and SMTP access.
use crate::{Application, MessageEvent, SessionUser};
use sandpost_core::{
    GlobalRole, MailAccess, Mailbox, Message, MessageFacts, MessageIdentifier, Scope,
    ScopeIdentifier, SmtpAuthenticationMechanism, User, UserIdentifier, View, ViewIdentifier,
};
use sandpost_query::Expression;
use sandpost_storage::{NewUser, Storage, StorageError, UpdateUser};
use sandpost_storage_sqlite::SqliteStorage;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Create an empty in-memory storage.
async fn empty_storage() -> Arc<dyn Storage> {
    Arc::new(
        SqliteStorage::memory()
            .await
            .expect("in-memory storage should initialize"),
    )
}

/// Create a user with a unique email, role, mail access, and optional personal filter.
async fn create_user(
    storage: &Arc<dyn Storage>,
    name: &str,
    role: GlobalRole,
    mail_access: MailAccess,
    personal_filter: Option<&str>,
) -> User {
    let email = format!(
        "{}-{}@example.test",
        name.to_lowercase().replace(' ', "-"),
        UserIdentifier::new()
    );
    storage
        .create_user(&NewUser {
            name: name.into(),
            email,
            password_hash: "hash".into(),
            global_role: role,
            mail_access,
            personal_filter: personal_filter.map(str::to_owned),
        })
        .await
        .unwrap()
}

/// Create a scope with a filter below an optional parent.
async fn create_scope(
    storage: &Arc<dyn Storage>,
    name: &str,
    filter: &str,
    parent: Option<ScopeIdentifier>,
) -> ScopeIdentifier {
    let identifier = ScopeIdentifier::new();
    storage
        .save_scope(&Scope {
            identifier,
            parent,
            name: name.into(),
            description: None,
            filter: filter.into(),
            position: 0,
            policy_version: 1,
        })
        .await
        .unwrap();
    identifier
}

/// Capture one message with a subject and envelope recipient.
async fn insert_message(
    storage: &Arc<dyn Storage>,
    subject: &str,
    recipient: &str,
) -> MessageIdentifier {
    let identifier = MessageIdentifier::new();
    let domain = recipient
        .split_once('@')
        .map(|(_, domain)| domain)
        .unwrap_or("example.test")
        .to_owned();
    storage
        .insert_message(&Message {
            identifier,
            facts: MessageFacts {
                subject: subject.into(),
                envelope_to: vec![Mailbox {
                    address: recipient.into(),
                    domain,
                }],
                ..MessageFacts::default()
            },
            raw_message: Vec::new(),
            attachments: Vec::new(),
        })
        .await
        .unwrap();
    identifier
}

/// Build the session identity of a stored user.
fn session(user: &User) -> SessionUser {
    SessionUser {
        id: user.identifier,
        name: user.name.clone(),
        email: user.email.clone(),
        role: user.global_role,
    }
}

/// Build an application whose memory index already contains the stored mail.
async fn application(storage: Arc<dyn Storage>) -> Application {
    let (events, _) = broadcast::channel::<MessageEvent>(16);
    Application::new(storage, events).await.unwrap()
}

/// Return transfer-decoded ISO-8859-1 attachment octets and enforce message visibility.
#[tokio::test]
async fn get_attachment_preserves_transfer_decoded_bytes_and_authorization() {
    let storage = empty_storage().await;
    let authorized_user = create_user(
        &storage,
        "Authorized",
        GlobalRole::Member,
        MailAccess::All,
        None,
    )
    .await;
    let unauthorized_user = create_user(
        &storage,
        "Unauthorized",
        GlobalRole::Member,
        MailAccess::Scoped,
        None,
    )
    .await;
    let raw_message = concat!(
        "From: sender@example.test\r\n",
        "To: recipient@example.test\r\n",
        "Subject: ISO-8859-1 attachments\r\n",
        "MIME-Version: 1.0\r\n",
        "Content-Type: multipart/mixed; boundary=attachment-boundary\r\n",
        "\r\n",
        "--attachment-boundary\r\n",
        "Content-Type: text/plain; charset=ISO-8859-1\r\n",
        "Content-Transfer-Encoding: base64\r\n",
        "Content-Disposition: attachment; filename=base64.txt\r\n",
        "\r\n",
        "Y2Fm6Q==\r\n",
        "--attachment-boundary\r\n",
        "Content-Type: text/plain; charset=ISO-8859-1\r\n",
        "Content-Transfer-Encoding: quoted-printable\r\n",
        "Content-Disposition: attachment; filename=quoted-printable.txt\r\n",
        "\r\n",
        "caf=E9\r\n",
        "--attachment-boundary--\r\n",
    );
    let message =
        sandpost_mime::RawMessage::new(raw_message.as_bytes().to_vec())
            .envelope(sandpost_mime::Envelope::new().with_recipient(
                sandpost_mime::normalize_mailbox("recipient@example.test").unwrap(),
            ))
            .received_at(1_700_000_000)
            .parse()
            .unwrap();
    let message_identifier = message.identifier;
    storage.insert_message(&message).await.unwrap();

    let application = application(storage).await;
    let authorized_session = application.authorized(&session(&authorized_user));
    let expected_bytes = [b'c', b'a', b'f', 0xe9];
    let expected_hash = "dafd66c0b98965e688be1fc12942c09f0350e6be0685017c3f234e97d0adc92e";
    let metadata = authorized_session
        .get_message(message_identifier)
        .await
        .unwrap()
        .attachments;
    assert_eq!(metadata.len(), 2);

    for (attachment_index, attachment) in metadata.iter().enumerate() {
        let bytes = authorized_session
            .get_attachment(message_identifier, attachment_index)
            .await
            .unwrap();
        assert_eq!(bytes, expected_bytes);
        assert_eq!(attachment.size, expected_bytes.len() as u64);
        assert_eq!(attachment.content_hash, expected_hash);
    }
    assert!(matches!(
        authorized_session
            .get_attachment(message_identifier, 2)
            .await,
        Err(crate::ApplicationError::NotFound)
    ));
    assert!(matches!(
        application
            .authorized(&session(&unauthorized_user))
            .get_attachment(message_identifier, 0)
            .await,
        Err(crate::ApplicationError::NotFound)
    ));
}

/// Return the subjects of every message the user can list, sorted.
async fn visible_subjects(application: &Application, user: &User) -> Vec<String> {
    let mut subjects: Vec<String> = application
        .authorized(&session(user))
        .list_messages(None, 50)
        .await
        .unwrap()
        .into_iter()
        .map(|summary| summary.subject)
        .collect();
    subjects.sort();
    subjects
}

/// All-mail access reads the whole pool; scoped access reads only assigned scopes and fails
/// closed without any.
#[tokio::test]
async fn mail_access_selects_all_mail_or_assigned_scopes() {
    let storage = empty_storage().await;
    let everything = create_user(&storage, "All", GlobalRole::Member, MailAccess::All, None).await;
    let scoped = create_user(
        &storage,
        "Scoped",
        GlobalRole::Member,
        MailAccess::Scoped,
        None,
    )
    .await;
    let scope = create_scope(&storage, "EU", "envelope.to.domain == \"eu.test\"", None).await;
    let eu = insert_message(&storage, "eu invoice", "a@eu.test").await;
    insert_message(&storage, "us invoice", "b@us.test").await;
    let application = application(storage.clone()).await;
    assert_eq!(
        visible_subjects(&application, &everything).await,
        ["eu invoice", "us invoice"]
    );
    assert!(
        visible_subjects(&application, &scoped).await.is_empty(),
        "scoped access with no assigned scope must see nothing"
    );
    storage
        .assign_user_to_scope(scoped.identifier, scope)
        .await
        .unwrap();
    let authorized = application.authorized(&session(&scoped));
    assert!(authorized.is_message_visible(eu).await.unwrap());
    assert_eq!(
        visible_subjects(&application, &scoped).await,
        ["eu invoice"]
    );
}

/// A scoped account reads the union of its scopes, each restricted by its ancestors.
#[tokio::test]
async fn scoped_access_unions_scopes_with_inherited_filters() {
    let storage = empty_storage().await;
    let user = create_user(
        &storage,
        "Scoped",
        GlobalRole::Member,
        MailAccess::Scoped,
        None,
    )
    .await;
    let eu = create_scope(&storage, "EU", "envelope.to.domain == \"eu.test\"", None).await;
    let eu_invoices = create_scope(
        &storage,
        "Invoices",
        "subject contains \"invoice\"",
        Some(eu),
    )
    .await;
    let alerts = create_scope(&storage, "Alerts", "subject contains \"alert\"", None).await;
    insert_message(&storage, "eu invoice", "a@eu.test").await;
    insert_message(&storage, "us invoice", "b@us.test").await;
    insert_message(&storage, "eu receipt", "c@eu.test").await;
    insert_message(&storage, "us alert", "d@us.test").await;
    for scope in [eu_invoices, alerts] {
        storage
            .assign_user_to_scope(user.identifier, scope)
            .await
            .unwrap();
    }
    let application = application(storage.clone()).await;
    assert_eq!(
        visible_subjects(&application, &user).await,
        ["eu invoice", "us alert"]
    );
}

/// Unknown users cannot be assigned to scopes and deleting a user removes its assignments.
#[tokio::test]
async fn scope_assignments_follow_users() {
    let storage = empty_storage().await;
    let user = create_user(
        &storage,
        "Member",
        GlobalRole::Member,
        MailAccess::Scoped,
        None,
    )
    .await;
    let scope = create_scope(&storage, "EU", "", None).await;
    assert!(matches!(
        storage
            .assign_user_to_scope(UserIdentifier::new(), scope)
            .await
            .unwrap_err(),
        StorageError::ConstraintViolation(_)
    ));
    storage
        .assign_user_to_scope(user.identifier, scope)
        .await
        .unwrap();
    storage.delete_user(user.identifier).await.unwrap();
    assert!(storage.list_scope_members(scope).await.unwrap().is_empty());
}

/// An owner reads everything regardless of personal filters; an admin follows its mail access.
#[tokio::test]
async fn owner_authority_differs_from_admin_visibility() {
    let storage = empty_storage().await;
    let owner = create_user(
        &storage,
        "Owner",
        GlobalRole::Owner,
        MailAccess::Scoped,
        Some("subject == \"nothing\""),
    )
    .await;
    let admin = create_user(
        &storage,
        "Admin",
        GlobalRole::Admin,
        MailAccess::Scoped,
        None,
    )
    .await;
    insert_message(&storage, "secret", "a@example.test").await;
    let application = application(storage.clone()).await;
    assert_eq!(visible_subjects(&application, &owner).await, ["secret"]);
    assert!(
        visible_subjects(&application, &admin).await.is_empty(),
        "a scoped admin without scopes sees no mail"
    );
}

/// A personal filter restricts all-mail access.
#[tokio::test]
async fn personal_filter_restricts_visible_mail() {
    let storage = empty_storage().await;
    let user = create_user(
        &storage,
        "Filtered",
        GlobalRole::Member,
        MailAccess::All,
        Some("subject == \"keep\""),
    )
    .await;
    insert_message(&storage, "keep", "a@example.test").await;
    insert_message(&storage, "drop", "b@example.test").await;
    let application = application(storage.clone()).await;
    assert_eq!(visible_subjects(&application, &user).await, ["keep"]);
}

/// The last owner cannot be demoted or deleted; other owners remain removable.
#[tokio::test]
async fn last_owner_invariant_is_enforced() {
    let storage = empty_storage().await;
    let first = storage
        .create_first_owner(&NewUser {
            name: "First".into(),
            email: "first@example.test".into(),
            password_hash: "hash".into(),
            global_role: GlobalRole::Owner,
            mail_access: MailAccess::All,
            personal_filter: None,
        })
        .await
        .unwrap();
    let demote = storage
        .update_user(
            first.identifier,
            &UpdateUser {
                name: first.name.clone(),
                email: first.email.clone(),
                password_hash: first.password_hash.clone(),
                global_role: GlobalRole::Member,
                mail_access: MailAccess::All,
                personal_filter: None,
            },
        )
        .await
        .unwrap_err();
    assert!(matches!(demote, StorageError::LastOwner));
    assert!(matches!(
        storage.delete_user(first.identifier).await.unwrap_err(),
        StorageError::LastOwner
    ));
    let second = create_user(&storage, "Second", GlobalRole::Owner, MailAccess::All, None).await;
    storage.delete_user(first.identifier).await.unwrap();
    assert!(storage.get_user(second.identifier).await.unwrap().is_some());
}

/// Only owners read index status; only global administrators manage scopes and assignments.
#[tokio::test]
async fn scope_and_search_administration_require_global_roles() {
    let storage = empty_storage().await;
    let owner = create_user(&storage, "Owner", GlobalRole::Owner, MailAccess::All, None).await;
    let admin = create_user(&storage, "Admin", GlobalRole::Admin, MailAccess::All, None).await;
    let member = create_user(
        &storage,
        "Member",
        GlobalRole::Member,
        MailAccess::All,
        None,
    )
    .await;
    let viewer = create_user(
        &storage,
        "Viewer",
        GlobalRole::Viewer,
        MailAccess::All,
        None,
    )
    .await;
    let application = application(storage.clone()).await;
    assert!(
        application
            .authorized(&session(&admin))
            .search_status()
            .await
            .is_err()
    );
    assert!(
        application
            .authorized(&session(&owner))
            .search_status()
            .await
            .is_ok()
    );
    let scope = Scope {
        identifier: ScopeIdentifier::new(),
        parent: None,
        name: "EU".into(),
        description: None,
        filter: "envelope.to.domain == \"eu.test\"".into(),
        position: 0,
        policy_version: 0,
    };
    for user in [&member, &viewer] {
        let authorized = application.authorized(&session(user));
        assert!(matches!(
            authorized.save_scope(scope.clone()).await,
            Err(crate::ApplicationError::Forbidden)
        ));
        assert!(matches!(
            authorized.list_users().await,
            Err(crate::ApplicationError::Forbidden)
        ));
    }
    let admin_application = application.authorized(&session(&admin));
    let saved = admin_application.save_scope(scope.clone()).await.unwrap();
    assert_eq!(saved.policy_version, 1);
    admin_application
        .assign_scope(member.identifier, saved.identifier)
        .await
        .unwrap();
    assert_eq!(
        admin_application
            .list_scope_assignments(saved.identifier)
            .await
            .unwrap(),
        vec![member.identifier]
    );
    assert!(matches!(
        application
            .authorized(&session(&member))
            .assign_scope(viewer.identifier, saved.identifier)
            .await,
        Err(crate::ApplicationError::Forbidden)
    ));
    assert_eq!(
        application
            .authorized(&session(&member))
            .list_scopes()
            .await
            .unwrap()
            .len(),
        1,
        "non-administrators see only their own scopes"
    );
    assert!(
        application
            .authorized(&session(&viewer))
            .list_scopes()
            .await
            .unwrap()
            .is_empty()
    );
}

/// Members may delete mail they can read; viewers are read-only.
#[tokio::test]
async fn viewers_cannot_delete_mail() {
    let storage = empty_storage().await;
    let member = create_user(
        &storage,
        "Member",
        GlobalRole::Member,
        MailAccess::All,
        None,
    )
    .await;
    let viewer = create_user(
        &storage,
        "Viewer",
        GlobalRole::Viewer,
        MailAccess::All,
        None,
    )
    .await;
    let scoped = create_user(
        &storage,
        "Scoped",
        GlobalRole::Member,
        MailAccess::Scoped,
        None,
    )
    .await;
    let message = insert_message(&storage, "to delete", "a@example.test").await;
    let application = application(storage.clone()).await;
    assert!(matches!(
        application
            .authorized(&session(&viewer))
            .delete_message(message)
            .await,
        Err(crate::ApplicationError::Forbidden)
    ));
    assert!(matches!(
        application
            .authorized(&session(&scoped))
            .delete_message(message)
            .await,
        Err(crate::ApplicationError::NotFound)
    ));
    application
        .authorized(&session(&member))
        .delete_message(message)
        .await
        .unwrap();
    assert!(storage.get_message(message).await.unwrap().is_some());
    assert!(
        !application
            .authorized(&session(&member))
            .is_message_visible(message)
            .await
            .unwrap()
    );
    assert!(
        application
            .authorized(&session(&viewer))
            .is_message_visible(message)
            .await
            .unwrap()
    );
}

/// Trash is refused to Viewers and moves only mail the caller can read.
#[tokio::test]
async fn trash_requires_delete_permission_and_authorization() {
    let storage = empty_storage().await;
    let viewer = create_user(
        &storage,
        "Viewer",
        GlobalRole::Viewer,
        MailAccess::All,
        None,
    )
    .await;
    let scoped = create_user(
        &storage,
        "Scoped",
        GlobalRole::Member,
        MailAccess::Scoped,
        None,
    )
    .await;
    let scope = create_scope(&storage, "EU", "envelope.to.domain == \"eu.test\"", None).await;
    storage
        .assign_user_to_scope(scoped.identifier, scope)
        .await
        .unwrap();
    let readable = insert_message(&storage, "eu invoice", "a@eu.test").await;
    let hidden = insert_message(&storage, "us invoice", "b@us.test").await;
    let application = application(storage.clone()).await;
    let viewer_application = application.authorized(&session(&viewer));
    assert!(matches!(
        viewer_application.trash_web_messages(vec![readable]).await,
        Err(crate::ApplicationError::Forbidden)
    ));
    assert!(matches!(
        viewer_application.empty_web_trash().await,
        Err(crate::ApplicationError::Forbidden)
    ));
    let scoped_application = application.authorized(&session(&scoped));
    assert_eq!(
        scoped_application
            .trash_web_messages(vec![readable, hidden])
            .await
            .unwrap(),
        1
    );
    let trash = scoped_application
        .query_web_trash(crate::QueryOptions {
            limit: 50,
            ..crate::QueryOptions::default()
        })
        .await
        .unwrap();
    assert_eq!(
        trash
            .iter()
            .map(|message| message.identifier)
            .collect::<Vec<_>>(),
        [readable]
    );
    assert_eq!(scoped_application.empty_web_trash().await.unwrap(), 1);
    assert!(storage.get_message(readable).await.unwrap().is_some());
}

/// Atomic delete-all hides only authorized mail for the caller and preserves other users' mail.
#[tokio::test]
async fn delete_all_messages_removes_only_readable_mail() {
    let storage = empty_storage().await;
    let viewer = create_user(
        &storage,
        "Viewer",
        GlobalRole::Viewer,
        MailAccess::All,
        None,
    )
    .await;
    let scoped = create_user(
        &storage,
        "Scoped",
        GlobalRole::Member,
        MailAccess::Scoped,
        None,
    )
    .await;
    let owner = create_user(&storage, "Owner", GlobalRole::Owner, MailAccess::All, None).await;
    let scope = create_scope(&storage, "EU", "envelope.to.domain == \"eu.test\"", None).await;
    storage
        .assign_user_to_scope(scoped.identifier, scope)
        .await
        .unwrap();
    let readable_count = super::SEARCH_CANDIDATE_PAGE_SIZE + 6;
    for index in 0..readable_count {
        insert_message(&storage, &format!("eu {index}"), "a@eu.test").await;
    }
    let hidden = insert_message(&storage, "us invoice", "b@us.test").await;
    let application = application(storage.clone()).await;
    assert!(matches!(
        application
            .authorized(&session(&viewer))
            .delete_all_messages()
            .await,
        Err(crate::ApplicationError::Forbidden)
    ));
    assert_eq!(
        application
            .authorized(&session(&scoped))
            .delete_all_messages()
            .await
            .unwrap(),
        readable_count as u64
    );
    assert!(storage.get_message(hidden).await.unwrap().is_some());
    // User deletion does not enqueue global index mutations.
    application.search_coordinator().catch_up().await.unwrap();
    assert!(visible_subjects(&application, &scoped).await.is_empty());
    assert_eq!(
        application
            .authorized(&session(&owner))
            .delete_all_messages()
            .await
            .unwrap(),
        readable_count as u64 + 1
    );
    assert!(storage.get_message(hidden).await.unwrap().is_some());
    assert_eq!(
        application
            .authorized(&session(&owner))
            .delete_all_messages()
            .await
            .unwrap(),
        0
    );
}

/// Anyone keeps personal views; only global administrators manage shared views.
#[tokio::test]
async fn shared_views_require_global_administration() {
    let storage = empty_storage().await;
    let admin = create_user(&storage, "Admin", GlobalRole::Admin, MailAccess::All, None).await;
    let viewer = create_user(
        &storage,
        "Viewer",
        GlobalRole::Viewer,
        MailAccess::All,
        None,
    )
    .await;
    let application = application(storage.clone()).await;
    let view = |owner: Option<UserIdentifier>| View {
        identifier: ViewIdentifier::new(),
        owner_identifier: owner,
        name: "Receipts".into(),
        filter: "subject contains \"receipt\"".into(),
    };
    let viewer_application = application.authorized(&session(&viewer));
    viewer_application
        .save_view(view(Some(viewer.identifier)))
        .await
        .unwrap();
    assert!(matches!(
        viewer_application.save_view(view(None)).await,
        Err(crate::ApplicationError::Forbidden)
    ));
    let shared = application
        .authorized(&session(&admin))
        .save_view(view(None))
        .await
        .unwrap();
    assert_eq!(viewer_application.list_views().await.unwrap().len(), 2);
    assert!(matches!(
        viewer_application.delete_view(shared.identifier).await,
        Err(crate::ApplicationError::Forbidden)
    ));
    viewer_application
        .create_inbox("QA".into(), "qa@sandpost.local".into())
        .await
        .unwrap();
}

/// Account administration stores the global role and mail access and keeps owners exclusive.
#[tokio::test]
async fn accounts_carry_role_and_mail_access() {
    let storage = empty_storage().await;
    let admin = create_user(&storage, "Admin", GlobalRole::Admin, MailAccess::All, None).await;
    let application = application(storage.clone()).await;
    let authorized = application.authorized(&session(&admin));
    let created = authorized
        .create_user(crate::NewAccount {
            name: "Reader".into(),
            email: "reader@example.test".into(),
            password: "reader password".into(),
            global_role: GlobalRole::Viewer,
            mail_access: MailAccess::Scoped,
            personal_filter: None,
        })
        .await
        .unwrap();
    assert_eq!(created.global_role, GlobalRole::Viewer);
    assert_eq!(created.mail_access, MailAccess::Scoped);
    let updated = authorized
        .update_user(
            created.identifier,
            crate::UpdateAccount {
                name: None,
                password: None,
                global_role: Some(GlobalRole::Member),
                mail_access: Some(MailAccess::All),
                personal_filter: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        (updated.global_role, updated.mail_access),
        (GlobalRole::Member, MailAccess::All)
    );
    assert!(matches!(
        authorized
            .update_user(
                created.identifier,
                crate::UpdateAccount {
                    name: None,
                    password: None,
                    global_role: Some(GlobalRole::Owner),
                    mail_access: None,
                    personal_filter: None,
                },
            )
            .await,
        Err(crate::ApplicationError::Forbidden)
    ));
    let defaulted: crate::NewAccount = serde_json::from_value(serde_json::json!({
        "name": "Defaulted",
        "email": "defaulted@example.test",
        "password": "defaulted password",
        "role": "member",
        "personal_filter": null
    }))
    .unwrap();
    assert_eq!(
        defaulted.mail_access,
        MailAccess::Scoped,
        "accounts created without a choice see no mail until scoped"
    );
}

/// The canonical filter AST is only used through the contract; evaluate it directly as a sanity check.
#[test]
fn filter_expression_is_shared_with_the_query_language() {
    let expression: Expression = sandpost_query::compile("subject == \"x\"")
        .unwrap()
        .into_expression();
    let facts = MessageFacts {
        subject: "x".into(),
        ..MessageFacts::default()
    };
    assert!(expression.evaluate(&facts));
}

/// Record listener reconciliation requests instead of binding a real socket.
#[derive(Default)]
struct RecordingListeners {
    applied: std::sync::Mutex<Vec<Option<std::net::SocketAddr>>>,
}

impl RecordingListeners {
    /// Report an address as listening, or the disabled state for none.
    fn status_for(address: Option<std::net::SocketAddr>) -> crate::MailListenerStatus {
        match address {
            Some(address) => crate::MailListenerStatus {
                configured_address: Some(address),
                bound_address: Some(address),
                source: crate::MailListenerSource::Database,
                state: crate::MailListenerState::Listening,
                error: None,
                authentication_required: !address.ip().is_loopback(),
                encryption: crate::MailListenerEncryption::None,
            },
            None => crate::MailListenerStatus::disabled(),
        }
    }
}

#[async_trait::async_trait]
impl crate::MailListenerControl for RecordingListeners {
    /// Record the request and report the address as listening.
    async fn apply(&self, address: Option<std::net::SocketAddr>) -> crate::MailListenerStatus {
        self.applied.lock().unwrap().push(address);
        Self::status_for(address)
    }

    /// Report the latest recorded address.
    async fn status(&self) -> crate::MailListenerStatus {
        Self::status_for(self.applied.lock().unwrap().last().copied().flatten())
    }
}

/// Parse a socket address literal used by listener tests.
fn socket(address: &str) -> std::net::SocketAddr {
    address.parse().unwrap()
}

/// Deleting a scope with children reports InUse; an unknown scope is NotFound.
#[tokio::test]
async fn in_use_scopes_are_not_deleted() {
    let storage = empty_storage().await;
    let owner = create_user(&storage, "Owner", GlobalRole::Owner, MailAccess::All, None).await;
    let parent = create_scope(&storage, "Parent", "", None).await;
    let child = create_scope(&storage, "Child", "", Some(parent)).await;
    let application = application(storage.clone()).await;
    let authorized = application.authorized(&session(&owner));
    assert!(matches!(
        authorized.delete_scope(parent).await,
        Err(crate::ApplicationError::InUse(_))
    ));
    authorized.delete_scope(child).await.unwrap();
    authorized.delete_scope(parent).await.unwrap();
    assert!(matches!(
        authorized.delete_scope(parent).await,
        Err(crate::ApplicationError::NotFound)
    ));
}

/// Preview the invoice filter below a parent scope and return the matching subjects.
async fn preview_subjects(
    application: &Application,
    user: &User,
    parent: ScopeIdentifier,
) -> Result<Vec<String>, crate::ApplicationError> {
    let mut subjects: Vec<String> = application
        .authorized(&session(user))
        .preview_scope(crate::ScopePreview {
            parent: Some(parent),
            filter: "subject contains \"invoice\"".into(),
            limit: 20,
        })
        .await?
        .into_iter()
        .map(|summary| summary.subject)
        .collect();
    subjects.sort();
    Ok(subjects)
}

/// Scope previews inherit ancestor filters and never exceed the caller's own visibility.
#[tokio::test]
async fn scope_preview_inherits_ancestors_and_caller_visibility() {
    let storage = empty_storage().await;
    let owner = create_user(&storage, "Owner", GlobalRole::Owner, MailAccess::All, None).await;
    let restricted = create_user(
        &storage,
        "Restricted",
        GlobalRole::Admin,
        MailAccess::All,
        Some("subject contains \"visible\""),
    )
    .await;
    let parent = create_scope(&storage, "EU", "envelope.to.domain == \"eu.test\"", None).await;
    insert_message(&storage, "visible eu invoice", "a@eu.test").await;
    insert_message(&storage, "hidden eu invoice", "b@eu.test").await;
    insert_message(&storage, "visible eu receipt", "c@eu.test").await;
    insert_message(&storage, "visible us invoice", "d@us.test").await;
    let application = application(storage.clone()).await;
    assert_eq!(
        preview_subjects(&application, &owner, parent)
            .await
            .unwrap(),
        ["hidden eu invoice", "visible eu invoice"]
    );
    assert_eq!(
        preview_subjects(&application, &restricted, parent)
            .await
            .unwrap(),
        ["visible eu invoice"]
    );
    let member = create_user(
        &storage,
        "Member",
        GlobalRole::Member,
        MailAccess::All,
        None,
    )
    .await;
    assert!(matches!(
        preview_subjects(&application, &member, parent).await,
        Err(crate::ApplicationError::Forbidden)
    ));
    assert!(matches!(
        preview_subjects(&application, &owner, ScopeIdentifier::new()).await,
        Err(crate::ApplicationError::NotFound)
    ));
}

/// Query validation reports the parser's message and byte position without running the query.
#[tokio::test]
async fn query_validation_reports_positions() {
    let storage = empty_storage().await;
    let user = create_user(
        &storage,
        "Member",
        GlobalRole::Member,
        MailAccess::All,
        None,
    )
    .await;
    let application = application(storage).await;
    let authorized = application.authorized(&session(&user));
    assert!(authorized.validate_query("").valid);
    assert!(authorized.validate_query("to.domain == \"recipient.test\"").valid);
    let invalid = authorized.validate_query("subject == ");
    assert!(!invalid.valid);
    let diagnostic = invalid.error.unwrap();
    assert!(!diagnostic.message.is_empty());
    assert!(diagnostic.position <= "subject == ".len());
    assert!(!authorized.validate_query("unknown_field == 1").valid);
}

/// Create an SMTP access as the given administrator.
async fn issue_access(
    application: &Application,
    administrator: &User,
    name: &str,
) -> crate::IssuedSmtpCredentials {
    application
        .authorized(&session(administrator))
        .create_smtp_access(crate::NewSmtpAccessRequest {
            name: name.into(),
            username: None,
            requires_encryption: false,
            allowed_mechanisms: SmtpAuthenticationMechanism::ALL.to_vec(),
        })
        .await
        .unwrap()
}

/// Creation returns the generated password once, storage keeps only an Argon2id hash, and no
/// read operation exposes either.
#[tokio::test]
async fn smtp_access_creation_returns_the_password_once_and_stores_only_a_hash() {
    let storage = empty_storage().await;
    let owner = create_user(&storage, "Owner", GlobalRole::Owner, MailAccess::All, None).await;
    let listeners = Arc::new(RecordingListeners::default());
    let application = application(storage.clone())
        .await
        .with_mail_listeners(listeners.clone());
    let authorized = application.authorized(&session(&owner));
    let issued = issue_access(&application, &owner, "Laravel Development").await;
    let access = &issued.access;
    assert_eq!(issued.password.len(), 32);
    assert!(access.username.starts_with("laravel-development-"));
    assert!(access.enabled);
    assert_eq!(access.last_used_at, None);
    assert_eq!(
        issued.server.listen_address,
        Some(socket("127.0.0.1:1025")),
        "the response carries the global server's connection details"
    );
    assert!(issued.server.listener.is_some());

    let credential = storage
        .get_smtp_access_credential(access.identifier)
        .await
        .unwrap()
        .unwrap();
    assert_ne!(credential.password_hash, issued.password);
    assert!(credential.password_hash.starts_with("$argon2id$"));
    assert!(!credential.password_hash.contains(&issued.password));
    assert!(crate::instance::verify_password(
        &issued.password,
        &credential.password_hash
    ));

    let listed = authorized.list_smtp_accesses().await.unwrap();
    assert_eq!(listed.len(), 1);
    let serialized = serde_json::to_string(&listed).unwrap();
    assert!(!serialized.contains(&issued.password));
    assert!(!serialized.contains(&credential.password_hash));
    assert!(!serialized.contains("password"));
    let changed = serde_json::to_string(
        &authorized
            .update_smtp_access(
                access.identifier,
                crate::SmtpAccessUpdate {
                    enabled: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap(),
    )
    .unwrap();
    assert!(!changed.contains("password"));
    let server = serde_json::to_string(&authorized.smtp_server().await.unwrap()).unwrap();
    assert!(!server.contains(&issued.password));
}

/// Valid credentials authenticate to their own access only.
#[tokio::test]
async fn smtp_credentials_are_verified_and_isolated_between_accesses() {
    let storage = empty_storage().await;
    let owner = create_user(&storage, "Owner", GlobalRole::Owner, MailAccess::All, None).await;
    let application = application(storage.clone()).await;
    let first = issue_access(&application, &owner, "Staging").await;
    let second = issue_access(&application, &owner, "CI Pipeline").await;
    let authenticator = crate::SmtpAuthenticator::new(storage.clone());

    let principal = authenticator
        .authenticate(
            &first.access.username,
            &first.password,
            SmtpAuthenticationMechanism::Plain,
            false,
        )
        .await
        .unwrap()
        .principal()
        .expect("valid credentials authenticate");
    assert_eq!(principal.access_identifier(), first.access.identifier);
    assert!(authenticator.may_deliver(&principal).await.unwrap());
    let other = authenticator
        .authenticate(
            &second.access.username,
            &second.password,
            SmtpAuthenticationMechanism::Plain,
            false,
        )
        .await
        .unwrap()
        .principal()
        .unwrap();
    assert_eq!(other.access_identifier(), second.access.identifier);
    for (username, password) in [
        (first.access.username.as_str(), second.password.as_str()),
        (second.access.username.as_str(), first.password.as_str()),
        (first.access.username.as_str(), ""),
        ("unknown-user", first.password.as_str()),
        (owner.email.as_str(), first.password.as_str()),
    ] {
        assert!(
            authenticator
                .authenticate(
                    username,
                    password,
                    SmtpAuthenticationMechanism::Plain,
                    false
                )
                .await
                .unwrap()
                .principal()
                .is_none(),
            "{username} must not authenticate with another credential"
        );
    }
    let listed = application
        .authorized(&session(&owner))
        .list_smtp_accesses()
        .await
        .unwrap();
    assert!(listed.iter().all(|access| access.last_used_at.is_some()));
}

/// Disabling and deleting an access stop authentication and refuse already-authenticated
/// sessions; re-enabling restores the same credentials.
#[tokio::test]
async fn disabled_and_deleted_smtp_accesses_are_refused() {
    let storage = empty_storage().await;
    let owner = create_user(&storage, "Owner", GlobalRole::Owner, MailAccess::All, None).await;
    let application = application(storage.clone()).await;
    let authorized = application.authorized(&session(&owner));
    let issued = issue_access(&application, &owner, "Staging").await;
    let identifier = issued.access.identifier;
    let username = issued.access.username.clone();
    let authenticator = crate::SmtpAuthenticator::new(storage.clone());
    let principal = authenticator
        .authenticate(
            &username,
            &issued.password,
            SmtpAuthenticationMechanism::Plain,
            false,
        )
        .await
        .unwrap()
        .principal()
        .unwrap();

    let disabled = authorized
        .update_smtp_access(
            identifier,
            crate::SmtpAccessUpdate {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(!disabled.enabled);
    assert!(
        authenticator
            .authenticate(
                &username,
                &issued.password,
                SmtpAuthenticationMechanism::Plain,
                false
            )
            .await
            .unwrap()
            .principal()
            .is_none()
    );
    assert!(!authenticator.may_deliver(&principal).await.unwrap());

    authorized
        .update_smtp_access(
            identifier,
            crate::SmtpAccessUpdate {
                enabled: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(authenticator.may_deliver(&principal).await.unwrap());

    authorized.delete_smtp_access(identifier).await.unwrap();
    assert!(
        authenticator
            .authenticate(
                &username,
                &issued.password,
                SmtpAuthenticationMechanism::Plain,
                false
            )
            .await
            .unwrap()
            .principal()
            .is_none()
    );
    assert!(!authenticator.may_deliver(&principal).await.unwrap());
    assert!(matches!(
        authorized.delete_smtp_access(identifier).await,
        Err(crate::ApplicationError::NotFound)
    ));
    assert!(authorized.list_smtp_accesses().await.unwrap().is_empty());
}

/// Regeneration issues a new password, invalidates the previous one immediately, and refuses
/// sessions that authenticated with it.
#[tokio::test]
async fn regenerating_a_password_invalidates_the_previous_one() {
    let storage = empty_storage().await;
    let admin = create_user(&storage, "Admin", GlobalRole::Admin, MailAccess::All, None).await;
    let application = application(storage.clone()).await;
    let issued = issue_access(&application, &admin, "Staging").await;
    let username = issued.access.username.clone();
    let authenticator = crate::SmtpAuthenticator::new(storage.clone());
    let previous = authenticator
        .authenticate(
            &username,
            &issued.password,
            SmtpAuthenticationMechanism::Plain,
            false,
        )
        .await
        .unwrap()
        .principal()
        .unwrap();

    let regenerated = application
        .authorized(&session(&admin))
        .regenerate_smtp_access_password(issued.access.identifier)
        .await
        .unwrap();
    assert_ne!(regenerated.password, issued.password);
    assert_eq!(regenerated.access.username, username);
    assert!(
        authenticator
            .authenticate(
                &username,
                &issued.password,
                SmtpAuthenticationMechanism::Plain,
                false
            )
            .await
            .unwrap()
            .principal()
            .is_none()
    );
    assert!(!authenticator.may_deliver(&previous).await.unwrap());
    let current = authenticator
        .authenticate(
            &username,
            &regenerated.password,
            SmtpAuthenticationMechanism::Plain,
            false,
        )
        .await
        .unwrap()
        .principal()
        .unwrap();
    assert!(authenticator.may_deliver(&current).await.unwrap());
    assert!(matches!(
        application
            .authorized(&session(&admin))
            .regenerate_smtp_access_password(sandpost_core::SmtpAccessIdentifier::new())
            .await,
        Err(crate::ApplicationError::NotFound)
    ));
}

/// Members and viewers cannot use any SMTP administration.
#[tokio::test]
async fn members_and_viewers_cannot_administer_smtp_accesses() {
    let storage = empty_storage().await;
    let owner = create_user(&storage, "Owner", GlobalRole::Owner, MailAccess::All, None).await;
    let application = application(storage.clone()).await;
    let issued = issue_access(&application, &owner, "Staging").await;
    let identifier = issued.access.identifier;
    for role in [GlobalRole::Member, GlobalRole::Viewer] {
        let user = create_user(&storage, "Restricted", role, MailAccess::All, None).await;
        let authorized = application.authorized(&session(&user));
        let forbidden = |result: Result<(), crate::ApplicationError>| {
            assert!(
                matches!(result, Err(crate::ApplicationError::Forbidden)),
                "{role:?} must be refused"
            )
        };
        forbidden(authorized.list_smtp_accesses().await.map(drop));
        forbidden(
            authorized
                .create_smtp_access(crate::NewSmtpAccessRequest {
                    name: "Sneaky".into(),
                    username: None,
                    requires_encryption: false,
                    allowed_mechanisms: SmtpAuthenticationMechanism::ALL.to_vec(),
                })
                .await
                .map(drop),
        );
        forbidden(
            authorized
                .regenerate_smtp_access_password(identifier)
                .await
                .map(drop),
        );
        forbidden(
            authorized
                .update_smtp_access(
                    identifier,
                    crate::SmtpAccessUpdate {
                        enabled: Some(false),
                        ..Default::default()
                    },
                )
                .await
                .map(drop),
        );
        forbidden(authorized.delete_smtp_access(identifier).await);
        forbidden(authorized.smtp_server().await.map(drop));
        forbidden(
            authorized
                .save_smtp_server(crate::SmtpServerUpdate {
                    listen_address: None,
                })
                .await
                .map(drop),
        );
    }
    let still_enabled = storage.get_smtp_access(identifier).await.unwrap().unwrap();
    assert!(still_enabled.enabled);
    assert_eq!(storage.list_smtp_accesses().await.unwrap().len(), 1);
}

/// Per-access rules are checked only after the password verifies: a TLS-only access refuses
/// clear text, and a LOGIN-only access refuses PLAIN.
#[tokio::test]
async fn smtp_access_rules_apply_after_the_password_verifies() {
    let storage = empty_storage().await;
    let owner = create_user(&storage, "Owner", GlobalRole::Owner, MailAccess::All, None).await;
    let application = application(storage.clone()).await;
    let issued = application
        .authorized(&session(&owner))
        .create_smtp_access(crate::NewSmtpAccessRequest {
            name: "Strict".into(),
            username: None,
            requires_encryption: true,
            allowed_mechanisms: vec![SmtpAuthenticationMechanism::Login],
        })
        .await
        .unwrap();
    assert!(issued.access.requires_encryption);
    assert_eq!(
        issued.access.allowed_mechanisms,
        [SmtpAuthenticationMechanism::Login]
    );
    /// Authenticate the strict access's username with one mechanism and TLS state.
    async fn attempt(
        authenticator: &crate::SmtpAuthenticator,
        username: &str,
        password: &str,
        mechanism: SmtpAuthenticationMechanism,
        encrypted: bool,
    ) -> crate::SmtpAuthentication {
        authenticator
            .authenticate(username, password, mechanism, encrypted)
            .await
            .unwrap()
    }
    let authenticator = crate::SmtpAuthenticator::new(storage.clone());
    let (username, password) = (issued.access.username.as_str(), issued.password.as_str());
    assert!(matches!(
        attempt(
            &authenticator,
            username,
            password,
            SmtpAuthenticationMechanism::Login,
            false
        )
        .await,
        crate::SmtpAuthentication::EncryptionRequired
    ));
    assert!(matches!(
        attempt(
            &authenticator,
            username,
            password,
            SmtpAuthenticationMechanism::Plain,
            true
        )
        .await,
        crate::SmtpAuthentication::MechanismNotAllowed
    ));
    assert!(
        matches!(
            attempt(
                &authenticator,
                username,
                "wrong password",
                SmtpAuthenticationMechanism::Plain,
                false
            )
            .await,
            crate::SmtpAuthentication::Invalid
        ),
        "a wrong password never reveals the rules"
    );
    assert!(
        attempt(
            &authenticator,
            username,
            password,
            SmtpAuthenticationMechanism::Login,
            true
        )
        .await
        .principal()
        .is_some()
    );
}

/// Updating rules is partial, rejects an empty mechanism set, and stops deliveries of sessions
/// that no longer satisfy the tightened rules.
#[tokio::test]
async fn tightened_smtp_access_rules_stop_existing_sessions() {
    let storage = empty_storage().await;
    let owner = create_user(&storage, "Owner", GlobalRole::Owner, MailAccess::All, None).await;
    let application = application(storage.clone()).await;
    let authorized = application.authorized(&session(&owner));
    let issued = issue_access(&application, &owner, "Staging").await;
    assert!(!issued.access.requires_encryption);
    assert_eq!(
        issued.access.allowed_mechanisms,
        SmtpAuthenticationMechanism::ALL
    );
    let identifier = issued.access.identifier;
    let authenticator = crate::SmtpAuthenticator::new(storage.clone());
    let plaintext = authenticator
        .authenticate(
            &issued.access.username,
            &issued.password,
            SmtpAuthenticationMechanism::Plain,
            false,
        )
        .await
        .unwrap()
        .principal()
        .unwrap();
    let encrypted = authenticator
        .authenticate(
            &issued.access.username,
            &issued.password,
            SmtpAuthenticationMechanism::Plain,
            true,
        )
        .await
        .unwrap()
        .principal()
        .unwrap();

    let updated = authorized
        .update_smtp_access(
            identifier,
            crate::SmtpAccessUpdate {
                requires_encryption: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(updated.requires_encryption && updated.enabled);
    assert_eq!(updated.allowed_mechanisms, SmtpAuthenticationMechanism::ALL);
    assert!(!authenticator.may_deliver(&plaintext).await.unwrap());
    assert!(authenticator.may_deliver(&encrypted).await.unwrap());

    let updated = authorized
        .update_smtp_access(
            identifier,
            crate::SmtpAccessUpdate {
                allowed_mechanisms: Some(vec![
                    SmtpAuthenticationMechanism::Login,
                    SmtpAuthenticationMechanism::Login,
                ]),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert!(updated.requires_encryption, "omitted fields are kept");
    assert_eq!(
        updated.allowed_mechanisms,
        [SmtpAuthenticationMechanism::Login]
    );
    assert!(!authenticator.may_deliver(&encrypted).await.unwrap());
    assert!(matches!(
        authorized
            .update_smtp_access(
                identifier,
                crate::SmtpAccessUpdate {
                    allowed_mechanisms: Some(Vec::new()),
                    ..Default::default()
                },
            )
            .await,
        Err(crate::ApplicationError::InvalidInput(_))
    ));
    assert!(matches!(
        authorized
            .update_smtp_access(
                sandpost_core::SmtpAccessIdentifier::new(),
                crate::SmtpAccessUpdate::default(),
            )
            .await,
        Err(crate::ApplicationError::NotFound)
    ));
}

/// Explicit usernames are validated and unique; names are required.
#[tokio::test]
async fn smtp_access_input_is_validated() {
    let storage = empty_storage().await;
    let owner = create_user(&storage, "Owner", GlobalRole::Owner, MailAccess::All, None).await;
    let application = application(storage.clone()).await;
    let authorized = application.authorized(&session(&owner));
    let request = |username: &str| crate::NewSmtpAccessRequest {
        name: "Explicit".into(),
        username: Some(username.into()),
        requires_encryption: false,
        allowed_mechanisms: SmtpAuthenticationMechanism::ALL.to_vec(),
    };
    let created = authorized
        .create_smtp_access(request("ci.pipeline"))
        .await
        .unwrap();
    assert_eq!(created.access.username, "ci.pipeline");
    assert!(matches!(
        authorized.create_smtp_access(request("ci.pipeline")).await,
        Err(crate::ApplicationError::Conflict)
    ));
    assert!(matches!(
        authorized.create_smtp_access(request("bad user")).await,
        Err(crate::ApplicationError::InvalidInput(_))
    ));
    assert!(matches!(
        authorized
            .create_smtp_access(crate::NewSmtpAccessRequest {
                name: " ".into(),
                username: None,
                requires_encryption: false,
                allowed_mechanisms: SmtpAuthenticationMechanism::ALL.to_vec(),
            })
            .await,
        Err(crate::ApplicationError::InvalidInput(_))
    ));
}

/// Saving the global server persists its address and applies it to the runtime listener.
#[tokio::test]
async fn saving_the_smtp_server_persists_and_applies_the_listener() {
    let storage = empty_storage().await;
    let admin = create_user(&storage, "Admin", GlobalRole::Admin, MailAccess::All, None).await;
    let listeners = Arc::new(RecordingListeners::default());
    let application = application(storage.clone())
        .await
        .with_mail_listeners(listeners.clone());
    let authorized = application.authorized(&session(&admin));
    let saved = authorized
        .save_smtp_server(crate::SmtpServerUpdate {
            listen_address: Some(socket("0.0.0.0:2525")),
        })
        .await
        .unwrap();
    let listener = saved.listener.unwrap();
    assert_eq!(listener.state, crate::MailListenerState::Listening);
    assert!(listener.authentication_required);
    assert_eq!(
        storage.smtp_listen_address().await.unwrap(),
        Some(socket("0.0.0.0:2525"))
    );
    let disabled = authorized
        .save_smtp_server(crate::SmtpServerUpdate {
            listen_address: None,
        })
        .await
        .unwrap();
    assert_eq!(
        disabled.listener.unwrap().state,
        crate::MailListenerState::Disabled
    );
    assert_eq!(
        listeners.applied.lock().unwrap().as_slice(),
        &[Some(socket("0.0.0.0:2525")), None]
    );
    assert_eq!(storage.smtp_listen_address().await.unwrap(), None);
}

/// Credentials survive a restart, and the plaintext password never reaches the database files.
#[tokio::test]
async fn smtp_credentials_persist_across_restarts_without_plaintext() {
    let directory =
        std::env::temp_dir().join(format!("sandpost-smtp-access-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&directory).unwrap();
    let path = directory.join("sandpost.sqlite3");
    let (username, password) = {
        let storage: Arc<dyn Storage> = Arc::new(SqliteStorage::open(&path).await.unwrap());
        let owner = create_user(&storage, "Owner", GlobalRole::Owner, MailAccess::All, None).await;
        let application = application(storage.clone()).await;
        let issued = issue_access(&application, &owner, "Persistent").await;
        (issued.access.username, issued.password)
    };
    for entry in std::fs::read_dir(&directory).unwrap() {
        let bytes = std::fs::read(entry.unwrap().path()).unwrap();
        assert!(
            !bytes
                .windows(password.len())
                .any(|window| window == password.as_bytes()),
            "the plaintext password must never be persisted"
        );
    }
    let reopened: Arc<dyn Storage> = Arc::new(SqliteStorage::open(&path).await.unwrap());
    let authenticator = crate::SmtpAuthenticator::new(reopened.clone());
    let principal = authenticator
        .authenticate(
            &username,
            &password,
            SmtpAuthenticationMechanism::Plain,
            false,
        )
        .await
        .unwrap()
        .principal()
        .expect("credentials survive a restart");
    assert!(authenticator.may_deliver(&principal).await.unwrap());
    drop(reopened);
    let _ = std::fs::remove_dir_all(&directory);
}

/// Tombstones refill pages, isolate users, and survive index rebuild and scope reassignment.
#[tokio::test]
async fn user_deletion_is_shared_across_reads_and_preserves_global_search() {
    let storage = empty_storage().await;
    let first = create_user(
        &storage,
        "First",
        GlobalRole::Member,
        MailAccess::Scoped,
        None,
    )
    .await;
    let second = create_user(
        &storage,
        "Second",
        GlobalRole::Member,
        MailAccess::All,
        None,
    )
    .await;
    let scope = create_scope(&storage, "Shared", "true", None).await;
    storage
        .assign_user_to_scope(first.identifier, scope)
        .await
        .unwrap();
    let mut identifiers = Vec::new();
    for index in 0..140 {
        identifiers
            .push(insert_message(&storage, &format!("shared {index}"), "a@example.test").await);
    }
    let application = application(storage.clone()).await;
    let first_application = application.authorized(&session(&first));
    let second_application = application.authorized(&session(&second));
    let before_status = storage.search_status().await.unwrap();
    for identifier in &identifiers[10..] {
        first_application.delete_message(*identifier).await.unwrap();
    }
    let results = first_application.list_messages(None, 5).await.unwrap();
    assert_eq!(results.len(), 5);
    assert_eq!(results[0].identifier, identifiers[9]);
    let next = first_application
        .list_messages(Some(results.last().unwrap().sequence), 5)
        .await
        .unwrap();
    assert_eq!(next.len(), 5);
    assert_eq!(next[0].identifier, identifiers[4]);
    assert_eq!(
        first_application
            .search_messages("shared".into(), None, 100)
            .await
            .unwrap()
            .len(),
        10
    );
    assert_eq!(
        second_application
            .list_messages(None, 100)
            .await
            .unwrap()
            .len(),
        100
    );
    let deleted = identifiers[139];
    assert!(matches!(
        first_application.get_message(deleted).await,
        Err(crate::ApplicationError::NotFound)
    ));
    assert!(matches!(
        first_application.get_raw_message(deleted).await,
        Err(crate::ApplicationError::NotFound)
    ));
    assert!(matches!(
        first_application.get_attachment(deleted, 0).await,
        Err(crate::ApplicationError::NotFound)
    ));
    assert!(second_application.get_message(deleted).await.is_ok());
    assert!(storage.get_message(deleted).await.unwrap().is_some());
    let after_status = storage.search_status().await.unwrap();
    assert_eq!(before_status.latest_sequence, after_status.latest_sequence);
    storage
        .remove_user_from_scope(first.identifier, scope)
        .await
        .unwrap();
    storage
        .assign_user_to_scope(first.identifier, scope)
        .await
        .unwrap();
    application.search_coordinator().rebuild().await.unwrap();
    assert!(!first_application.is_message_visible(deleted).await.unwrap());
    let view = View {
        identifier: ViewIdentifier::new(),
        owner_identifier: Some(first.identifier),
        name: "Shared".into(),
        filter: "true".into(),
    };
    first_application.save_view(view.clone()).await.unwrap();
    assert_eq!(
        first_application
            .query_messages(crate::QueryOptions {
                view_identifier: Some(view.identifier),
                limit: 100,
                ..Default::default()
            })
            .await
            .unwrap()
            .len(),
        10
    );
    assert_eq!(first_application.delete_all_messages().await.unwrap(), 10);
    assert_eq!(first_application.delete_all_messages().await.unwrap(), 0);
    assert_eq!(
        second_application
            .search_messages("shared".into(), None, 100)
            .await
            .unwrap()
            .len(),
        100
    );
}

/// Search cursors advance through an entirely stale page of physically removed index hits.
#[tokio::test]
async fn search_refills_past_physically_removed_candidates() {
    let storage = empty_storage().await;
    let user = create_user(
        &storage,
        "Reader",
        GlobalRole::Member,
        MailAccess::All,
        None,
    )
    .await;
    let survivor = insert_message(&storage, "survivor", "a@example.test").await;
    let mut stale = Vec::new();
    for _ in 0..70 {
        stale.push(insert_message(&storage, "stale", "a@example.test").await);
    }
    let application = application(storage.clone()).await;
    for identifier in stale {
        storage.delete_message(identifier).await.unwrap();
    }
    let results = application
        .authorized(&session(&user))
        .list_messages(None, 5)
        .await
        .unwrap();
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].identifier, survivor);
}
