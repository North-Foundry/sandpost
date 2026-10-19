//! Regression tests for native values, authentication, and authorized event delivery.

use crate::NativeRuntime;
use crate::boundary::{
    attachment_index_from_number, page_size_from_number, parse_imap_account_identifier,
    parse_imap_folder_identifier, parse_inbox_identifier, parse_message_identifier,
    parse_message_identifiers, sequence_from_number, serialize,
};
use napi::Result;
use sandpost_application::{Application, Instance};
use sandpost_application::{MessageEvent, NewAccount, SetupRequest};
use sandpost_core::{MessageIdentifier, MessageSequence};
use serde_json::{Value, json};
use std::{fs, path::PathBuf, sync::Arc, time::Duration};
use tokio::sync::{Mutex, broadcast, watch};
use tokio::{task::JoinHandle, time::timeout};

/// Remove the temporary installation database and metadata after the test.
struct TestDirectory(PathBuf);

impl Drop for TestDirectory {
    /// Delete temporary installation files when their storage handles are gone.
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Build an initialized in-memory native host with persistent test storage.
async fn authenticated_test_host() -> (
    Arc<NativeRuntime>,
    Application,
    Arc<dyn sandpost_storage::Storage>,
    watch::Sender<Option<std::result::Result<(), String>>>,
    TestDirectory,
) {
    let directory = TestDirectory(
        std::env::temp_dir().join(format!("sandpost-node-events-{}", uuid::Uuid::new_v4())),
    );
    fs::create_dir_all(&directory.0).unwrap();
    let storage: Arc<dyn sandpost_storage::Storage> = Arc::new(
        sandpost_storage_sqlite::SqliteStorage::open(directory.0.join("sandpost.sqlite3"))
            .await
            .unwrap(),
    );
    let instance = Instance::open(storage.clone(), directory.0.clone())
        .await
        .unwrap();
    let bootstrap_secret = instance.bootstrap_secret_for_startup().unwrap().unwrap();
    instance
        .setup(SetupRequest {
            bootstrap_secret,
            name: "Smoke Owner".into(),
            email: "smoke-owner@example.test".into(),
            password: "smoke-owner-password".into(),
        })
        .await
        .unwrap();
    let (event_sender, _) = broadcast::channel(16);
    let application = Application::with_instance(storage.clone(), event_sender, instance)
        .await
        .unwrap();
    let (session_token, _) = watch::channel(None);
    let (stop, _) = watch::channel(false);
    let (cancel_events, _) = watch::channel(false);
    let (completion_sender, completion) = watch::channel(None);
    let runtime = Arc::new(NativeRuntime {
        events: Mutex::new(application.subscribe()),
        changes: Mutex::new(application.subscribe_changes()),
        application: application.clone(),
        session_token,
        stop,
        cancel_events,
        completion,
        smtp_address: String::new(),
        http_address: String::new(),
    });
    (runtime, application, storage, completion_sender, directory)
}

/// Confirm a native event promise stays pending until session state changes.
async fn assert_event_pending(event: &mut JoinHandle<Result<Option<Value>>>) {
    let outcome = timeout(Duration::from_millis(30), event).await;
    assert!(
        outcome.is_err(),
        "event completed unexpectedly: {outcome:?}"
    );
}

/// Reject lossy JavaScript integers, invalid pagination, and invalid public identifiers.
#[test]
fn validates_native_boundary_arguments() {
    assert_eq!(sequence_from_number(42.0).unwrap(), MessageSequence(42));
    for value in [-1.0, 0.5, f64::NAN, f64::INFINITY, 9_007_199_254_740_992.0] {
        assert!(sequence_from_number(value).is_err());
    }
    for value in [0.0, 101.0, 1.5, f64::NAN] {
        assert!(page_size_from_number(value).is_err());
    }
    assert_eq!(page_size_from_number(100.0).unwrap(), 100);
    for value in [-1.0, 0.5, f64::NAN, f64::INFINITY, 9_007_199_254_740_992.0] {
        assert!(attachment_index_from_number(value).is_err());
    }
    assert_eq!(attachment_index_from_number(0.0).unwrap(), 0);
    assert!(parse_message_identifier("invalid").is_err());
    assert!(parse_message_identifier("00000000-0000-0000-0000-000000000001").is_ok());
    assert!(parse_message_identifiers(vec!["invalid".into()]).is_err());
    assert!(parse_message_identifiers(vec!["00000000-0000-0000-0000-000000000001".into()]).is_ok());
    assert!(
        parse_message_identifiers(vec!["00000000-0000-0000-0000-000000000001".into(); 257])
            .is_err()
    );
    assert!(parse_inbox_identifier("invalid").is_err());
    assert!(parse_inbox_identifier("00000000-0000-0000-0000-000000000001").is_ok());
    assert!(parse_imap_account_identifier("invalid").is_err());
    assert!(parse_imap_account_identifier("00000000-0000-0000-0000-000000000001").is_ok());
    assert!(parse_imap_folder_identifier("invalid").is_err());
    assert!(parse_imap_folder_identifier("00000000-0000-0000-0000-000000000001").is_ok());
}

/// Preserve the application's serde identifiers and sequences in event conversion.
#[test]
fn preserves_application_wire_event_fields() {
    let identifier = parse_message_identifier("00000000-0000-0000-0000-000000000001").unwrap();
    let value = serialize(MessageEvent {
        identifier,
        sequence: MessageSequence(7),
    })
    .unwrap();
    assert_eq!(
        value,
        json!({"id":"00000000-0000-0000-0000-000000000001","seq":7.0})
    );
}

/// Let the trusted native host supply setup credentials without returning them to JavaScript.
#[tokio::test]
async fn native_setup_uses_host_bootstrap_secret_and_reports_trusted_status() {
    let directory = TestDirectory(
        std::env::temp_dir().join(format!("sandpost-node-setup-{}", uuid::Uuid::new_v4())),
    );
    fs::create_dir_all(&directory.0).unwrap();
    let storage: Arc<dyn sandpost_storage::Storage> = Arc::new(
        sandpost_storage_sqlite::SqliteStorage::open(directory.0.join("sandpost.sqlite3"))
            .await
            .unwrap(),
    );
    let instance = Instance::open(storage.clone(), directory.0.clone())
        .await
        .unwrap();
    let (event_sender, _) = broadcast::channel(16);
    let application = Application::with_instance(storage, event_sender, instance)
        .await
        .unwrap();
    let (session_token, _) = watch::channel(None);
    let (stop, _) = watch::channel(false);
    let (cancel_events, _) = watch::channel(false);
    let (_completion_sender, completion) = watch::channel(None);
    let runtime = NativeRuntime {
        events: Mutex::new(application.subscribe()),
        changes: Mutex::new(application.subscribe_changes()),
        application,
        session_token,
        stop,
        cancel_events,
        completion,
        smtp_address: String::new(),
        http_address: String::new(),
    };

    assert_eq!(
        runtime.instance_status().unwrap(),
        json!({"initialized": false, "setupKeyRequired": false})
    );
    runtime
        .setup(json!({
            "bootstrapSecret": "renderer-value-is-ignored",
            "name": "Native Owner",
            "email": "native-owner@example.test",
            "password": "native owner password",
        }))
        .await
        .unwrap();
    assert_eq!(
        runtime.instance_status().unwrap(),
        json!({"initialized": true, "setupKeyRequired": false})
    );
    let error = runtime
        .setup(json!({
            "bootstrapSecret": "",
            "name": "Other Owner",
            "email": "other-owner@example.test",
            "password": "another owner password",
        }))
        .await
        .unwrap_err();
    assert_eq!(error.reason, "already initialized");
}

/// Preserve numeric pagination above napi-rs's integer-to-BigInt threshold.
#[test]
fn converts_large_wire_integers_to_safe_javascript_numbers() {
    let value =
        serialize(json!({"seq": 4_294_967_297_u64, "nested": [2_147_483_648_i64]})).unwrap();
    assert!(value["seq"].as_number().unwrap().is_f64());
    assert!(value["nested"][0].as_number().unwrap().is_f64());
    assert_eq!(value["seq"].as_f64(), Some(4_294_967_297.0));
    assert!(serialize(json!({"seq": 9_007_199_254_740_992_u64})).is_err());
}

/// Hold broadcast events while logged out and deliver them after a valid native login.
#[tokio::test]
async fn next_event_waits_for_login_and_logout_before_releasing_messages() {
    let (runtime, application, storage, _completion_sender, _directory) =
        authenticated_test_host().await;
    let first_identifier = MessageIdentifier(uuid::Uuid::new_v4());
    storage
        .insert_message(&sandpost_core::Message {
            identifier: first_identifier,
            facts: sandpost_core::MessageFacts::default(),
            raw_message: b"Subject: first\r\n\r\nfirst".to_vec(),
            attachments: vec![],
        })
        .await
        .unwrap();
    let mut first_event = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.next_event().await })
    };
    application
        .publish_message(MessageEvent {
            identifier: first_identifier,
            sequence: MessageSequence(1),
        })
        .unwrap();
    assert_event_pending(&mut first_event).await;
    application.search_coordinator().catch_up().await.unwrap();

    runtime
        .login(json!({
            "email": "smoke-owner@example.test",
            "password": "smoke-owner-password"
        }))
        .await
        .unwrap();
    let first_event = timeout(Duration::from_secs(1), first_event)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first_event["id"], first_identifier.to_string());

    runtime.logout().await.unwrap();
    let mut second_event = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.next_event().await })
    };
    let second_identifier = MessageIdentifier(uuid::Uuid::new_v4());
    storage
        .insert_message(&sandpost_core::Message {
            identifier: second_identifier,
            facts: sandpost_core::MessageFacts::default(),
            raw_message: b"Subject: second\r\n\r\nsecond".to_vec(),
            attachments: vec![],
        })
        .await
        .unwrap();
    application
        .publish_message(MessageEvent {
            identifier: second_identifier,
            sequence: MessageSequence(2),
        })
        .unwrap();
    assert_event_pending(&mut second_event).await;
    application.search_coordinator().catch_up().await.unwrap();

    runtime
        .login(json!({
            "email": "smoke-owner@example.test",
            "password": "smoke-owner-password"
        }))
        .await
        .unwrap();
    let second_event = timeout(Duration::from_secs(1), second_event)
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(second_event["id"], second_identifier.to_string());

    runtime.cancel_events();
    assert_eq!(runtime.next_event().await.unwrap(), None);
}

/// Mirror Seen writes wake native subscribers and update summaries without another mail arrival.
#[tokio::test]
async fn native_events_resynchronize_after_mirror_seen_changes() {
    let (runtime, application, storage, _completion_sender, _directory) =
        authenticated_test_host().await;
    login_owner(&runtime).await;
    let identifier = MessageIdentifier::new();
    storage
        .insert_message(&sandpost_core::Message {
            identifier,
            facts: sandpost_core::MessageFacts::default(),
            raw_message: b"Subject: read elsewhere\r\n\r\nbody".to_vec(),
            attachments: vec![],
        })
        .await
        .unwrap();
    let issued = runtime
        .imap_account_create(json!({"name": "Mirror client", "mode": "mirror"}))
        .await
        .unwrap();
    let session = match application
        .imap_service()
        .authenticate(
            issued["account"]["username"].as_str().unwrap(),
            issued["password"].as_str().unwrap(),
            None,
        )
        .await
        .unwrap()
    {
        sandpost_application::ImapLogin::Authenticated(session) => session,
        _ => panic!("IMAP authentication failed"),
    };
    let inbox = session.open("INBOX", false).await.unwrap();
    for seen in [true, false] {
        let mut event = {
            let runtime = runtime.clone();
            tokio::spawn(async move { runtime.next_event().await })
        };
        assert_event_pending(&mut event).await;
        session
            .store(
                &inbox.handle,
                &[inbox.members[0].0],
                &sandpost_storage::ImapFlagChange {
                    operation: if seen {
                        sandpost_storage::ImapFlagOperation::Add
                    } else {
                        sandpost_storage::ImapFlagOperation::Remove
                    },
                    flags: sandpost_core::ImapMessageFlags {
                        seen: true,
                        ..Default::default()
                    },
                },
            )
            .await
            .unwrap();
        assert_eq!(
            timeout(Duration::from_secs(1), event)
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
            Some(json!({"type": "resync"}))
        );
        let rows = runtime
            .messages_list(None, None, None, None, None)
            .await
            .unwrap();
        assert_eq!(rows[0]["id"], identifier.to_string());
        assert_eq!(rows[0]["seen"], seen);
    }
}

/// Committed-change hints remain gated by login and are cancellable while idle.
#[tokio::test]
async fn native_change_events_wait_for_login_and_honor_cancellation() {
    let (runtime, application, _storage, _completion_sender, _directory) =
        authenticated_test_host().await;
    let mut event = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.next_event().await })
    };
    application.notify_change(sandpost_application::ApplicationChange::Visibility);
    assert_event_pending(&mut event).await;
    login_owner(&runtime).await;
    assert_eq!(
        timeout(Duration::from_secs(1), event)
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Some(json!({"type": "resync"}))
    );
    let mut waiting = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.next_event().await })
    };
    assert_event_pending(&mut waiting).await;
    runtime.cancel_events();
    assert_eq!(waiting.await.unwrap().unwrap(), None);
}

/// Deny native administrative reads and destructive mail actions without permissions.
#[tokio::test]
async fn non_admin_native_actions_are_rejected() {
    let (runtime, application, storage, _completion_sender, _directory) =
        authenticated_test_host().await;
    runtime
        .login(json!({
            "email": "smoke-owner@example.test",
            "password": "smoke-owner-password"
        }))
        .await
        .unwrap();
    let owner = runtime.ensure_authenticated().await.unwrap();
    application
        .authorized(&owner)
        .create_user(NewAccount {
            name: "Limited User".into(),
            email: "limited-native@example.test".into(),
            password: "limited-user-password".into(),
            global_role: sandpost_core::GlobalRole::Member,
            mail_access: sandpost_core::MailAccess::Scoped,
            personal_filter: None,
        })
        .await
        .unwrap();
    runtime
        .login(json!({
            "email": "limited-native@example.test",
            "password": "limited-user-password"
        }))
        .await
        .unwrap();

    assert!(runtime.users_list().await.is_err());
    assert!(
        runtime
            .message_delete(MessageIdentifier::new().to_string())
            .await
            .is_err()
    );
    let hidden_identifier = MessageIdentifier::new();
    storage
        .insert_message(&sandpost_core::Message {
            identifier: hidden_identifier,
            facts: sandpost_core::MessageFacts::default(),
            raw_message: b"Subject: hidden\r\n\r\nhidden".to_vec(),
            attachments: vec![],
        })
        .await
        .unwrap();
    assert_eq!(
        runtime.messages_delete_all().await.unwrap(),
        json!({ "deleted": 0.0 })
    );
    assert!(
        storage
            .get_message(hidden_identifier)
            .await
            .unwrap()
            .is_some()
    );
    let mut event = {
        let runtime = runtime.clone();
        tokio::spawn(async move { runtime.next_event().await })
    };
    application
        .publish_message(MessageEvent {
            identifier: hidden_identifier,
            sequence: MessageSequence(1),
        })
        .unwrap();
    assert_event_pending(&mut event).await;
    runtime.cancel_events();
    assert_eq!(event.await.unwrap().unwrap(), None);
}

/// Sign the native host in as the owner created by `authenticated_test_host`.
async fn login_owner(runtime: &NativeRuntime) {
    runtime
        .login(json!({
            "email": "smoke-owner@example.test",
            "password": "smoke-owner-password"
        }))
        .await
        .unwrap();
}

/**
 * Restore a durable token into a fresh native host and reject invalid or revoked tokens.
 */
#[tokio::test]
async fn remembered_session_token_restores_through_instance_validation() {
    let (runtime, _application, _storage, _completion_sender, _directory) =
        authenticated_test_host().await;
    let login = runtime
        .login(json!({
            "email": "smoke-owner@example.test",
            "password": "smoke-owner-password",
            "rememberMe": true
        }))
        .await
        .unwrap();
    let token = runtime.remembered_session_token().await.unwrap().unwrap();
    runtime.session_token.send_replace(None);
    assert_eq!(runtime.session().await.unwrap(), Value::Null);
    assert_eq!(
        runtime
            .restore_session("invalid-token".into())
            .await
            .unwrap(),
        Value::Null
    );
    assert_eq!(
        runtime.restore_session(token.clone()).await.unwrap()["email"],
        "smoke-owner@example.test"
    );
    runtime.logout().await.unwrap();
    assert_eq!(runtime.restore_session(token).await.unwrap(), Value::Null);
    assert_eq!(login["email"], "smoke-owner@example.test");
}

/// Report whether a serialized value contains a string anywhere in its tree.
fn contains_text(value: &Value, text: &str) -> bool {
    match value {
        Value::String(candidate) => candidate.contains(text),
        Value::Array(values) => values.iter().any(|value| contains_text(value, text)),
        Value::Object(values) => values.values().any(|value| contains_text(value, text)),
        _ => false,
    }
}

/// Manage an Independent IMAP account through the native API and preserve its wire fields.
#[tokio::test]
async fn native_imap_account_lifecycle_preserves_wire_fields_and_one_time_passwords() {
    let (runtime, _application, _storage, _completion_sender, _directory) =
        authenticated_test_host().await;
    assert_eq!(
        runtime.imap_accounts_list().await.unwrap_err().reason,
        "authentication required"
    );
    login_owner(&runtime).await;
    let owner = runtime.ensure_authenticated().await.unwrap();

    let server = runtime.imap_server_get().await.unwrap();
    assert_eq!(server["enabled"], json!(false));
    assert_eq!(server["security"], json!("none"));

    let issued = runtime
        .imap_account_create(json!({
            "name": "Apple Mail",
            "username": "apple-mail",
            "mode": "independent",
            "password": "renderer-chosen-password",
        }))
        .await
        .unwrap();
    let password = issued["password"].as_str().unwrap().to_owned();
    assert_eq!(password.len(), 32);
    assert_ne!(password, "renderer-chosen-password");
    assert_eq!(issued["server"]["enabled"], json!(false));
    let account = &issued["account"];
    let account_identifier = account["identifier"].as_str().unwrap().to_owned();
    assert_eq!(account["owner_identifier"], json!(owner.id.to_string()));
    assert_eq!(account["username"], json!("apple-mail"));
    assert_eq!(account["enabled"], json!(true));
    assert_eq!(account["mode"], json!("independent"));
    assert_eq!(account["mirror_views"], json!(false));
    assert_eq!(account["last_used_at"], Value::Null);
    assert!(account["created_at"].as_f64().unwrap() > 0.0);
    assert_eq!(account["linked_views"], json!([]));
    let mailbox_names: Vec<&str> = account["mailboxes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|mailbox| mailbox["name"].as_str().unwrap())
        .collect();
    assert_eq!(mailbox_names, ["INBOX", "Trash"]);

    let listed = runtime.imap_accounts_list().await.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert!(!contains_text(&listed, &password));
    assert!(!contains_text(&listed, "$argon2"));
    let loaded = runtime
        .imap_account_get(account_identifier.clone())
        .await
        .unwrap();
    assert!(loaded.get("password").is_none());
    assert_eq!(loaded["identifier"], json!(account_identifier));

    let updated = runtime
        .imap_account_update(
            account_identifier.clone(),
            json!({ "enabled": false, "mode": "independent" }),
        )
        .await
        .unwrap();
    assert_eq!(updated["enabled"], json!(false));
    assert_eq!(updated["mode"], json!("independent"));
    assert_eq!(updated["mirror_views"], json!(false));
    assert_eq!(updated["name"], json!("Apple Mail"));

    let invalid_folder = runtime
        .imap_folder_create(
            account_identifier.clone(),
            json!({ "name": "Broken", "filter": "subject ==" }),
        )
        .await
        .unwrap_err();
    assert_eq!(invalid_folder.status, napi::Status::InvalidArg);
    let folder = runtime
        .imap_folder_create(
            account_identifier.clone(),
            json!({ "name": "Signups", "filter": "subject contains \"welcome\"" }),
        )
        .await
        .unwrap();
    assert_eq!(folder["include_in_inbox"], json!(true));
    assert_eq!(folder["account_identifier"], json!(account_identifier));
    let folder_identifier = folder["identifier"].as_str().unwrap().to_owned();
    let account = runtime
        .imap_account_get(account_identifier.clone())
        .await
        .unwrap();
    let mailbox_names: Vec<&str> = account["mailboxes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|mailbox| mailbox["name"].as_str().unwrap())
        .collect();
    assert_eq!(mailbox_names, ["INBOX", "Trash", "Signups"]);
    assert_eq!(account["mailboxes"][2]["source"], json!("folder"));
    let duplicate = runtime
        .imap_folder_create(
            account_identifier.clone(),
            json!({ "name": "Signups", "filter": "" }),
        )
        .await
        .unwrap_err();
    assert_eq!(duplicate.reason, "conflict");
    let renamed = runtime
        .imap_folder_update(
            account_identifier.clone(),
            folder_identifier.clone(),
            json!({ "name": "Welcome", "filter": "subject contains \"hello\"", "include_in_inbox": false }),
        )
        .await
        .unwrap();
    assert_eq!(renamed["name"], json!("Welcome"));
    assert_eq!(renamed["include_in_inbox"], json!(false));
    runtime
        .imap_folder_delete(account_identifier.clone(), folder_identifier.clone())
        .await
        .unwrap();
    assert_eq!(
        runtime
            .imap_folder_delete(account_identifier.clone(), folder_identifier)
            .await
            .unwrap_err()
            .reason,
        "IMAP account or folder not found"
    );

    let regenerated = runtime
        .imap_account_regenerate(account_identifier.clone())
        .await
        .unwrap();
    let new_password = regenerated["password"].as_str().unwrap();
    assert_eq!(new_password.len(), 32);
    assert_ne!(new_password, password);
    assert_eq!(
        runtime
            .imap_account_restore(account_identifier.clone())
            .await
            .unwrap(),
        json!({ "restored": 0.0 })
    );

    runtime
        .imap_account_delete(account_identifier.clone())
        .await
        .unwrap();
    assert_eq!(
        runtime
            .imap_account_get(account_identifier)
            .await
            .unwrap_err()
            .reason,
        "IMAP account not found"
    );
    assert_eq!(runtime.imap_accounts_list().await.unwrap(), json!([]));
}

/// Validate IMAP identifiers and inputs at the native boundary before the application runs.
#[tokio::test]
async fn native_imap_operations_reject_invalid_identifiers_and_inputs() {
    let (runtime, _application, _storage, _completion_sender, _directory) =
        authenticated_test_host().await;
    login_owner(&runtime).await;
    let missing = "00000000-0000-4000-8000-000000000001".to_owned();

    let invalid_identifier = runtime
        .imap_account_get("not-an-identifier".into())
        .await
        .unwrap_err();
    assert_eq!(invalid_identifier.status, napi::Status::InvalidArg);
    assert_eq!(invalid_identifier.reason, "invalid IMAP account identifier");
    assert_eq!(
        runtime
            .imap_folder_delete(missing.clone(), "bad".into())
            .await
            .unwrap_err()
            .reason,
        "invalid IMAP folder identifier"
    );
    assert_eq!(
        runtime
            .imap_account_create(json!({ "username": "missing-name" }))
            .await
            .unwrap_err()
            .reason,
        "invalid IMAP account"
    );
    assert_eq!(
        runtime
            .imap_account_update(missing.clone(), json!({ "enabled": "no" }))
            .await
            .unwrap_err()
            .reason,
        "invalid IMAP account update"
    );
    assert_eq!(
        runtime
            .imap_folder_create(missing.clone(), json!({ "name": "No filter" }))
            .await
            .unwrap_err()
            .reason,
        "invalid IMAP folder"
    );
    let invalid_mode = runtime
        .imap_account_create(json!({
            "name": "Thunderbird",
            "mode": "shared",
        }))
        .await
        .unwrap_err();
    assert_eq!(invalid_mode.status, napi::Status::InvalidArg);
    let invalid_username = runtime
        .imap_account_create(json!({ "name": "Thunderbird", "username": "has space" }))
        .await
        .unwrap_err();
    assert_eq!(invalid_username.status, napi::Status::InvalidArg);
    assert_eq!(
        runtime
            .imap_account_regenerate(missing)
            .await
            .unwrap_err()
            .reason,
        "IMAP account not found"
    );
}

/// Keep IMAP accounts private to their owner and reject Viewer configuration mutations.
#[tokio::test]
async fn native_imap_accounts_are_private_to_their_owner() {
    let (runtime, application, _storage, _completion_sender, _directory) =
        authenticated_test_host().await;
    login_owner(&runtime).await;
    let owner = runtime.ensure_authenticated().await.unwrap();
    application
        .authorized(&owner)
        .create_user(NewAccount {
            name: "Read Only".into(),
            email: "viewer-native@example.test".into(),
            password: "viewer-user-password".into(),
            global_role: sandpost_core::GlobalRole::Viewer,
            mail_access: sandpost_core::MailAccess::Scoped,
            personal_filter: None,
        })
        .await
        .unwrap();
    application
        .authorized(&owner)
        .create_user(NewAccount {
            name: "Writable Member".into(),
            email: "member-native@example.test".into(),
            password: "member-user-password".into(),
            global_role: sandpost_core::GlobalRole::Member,
            mail_access: sandpost_core::MailAccess::Scoped,
            personal_filter: None,
        })
        .await
        .unwrap();
    let owner_account = runtime
        .imap_account_create(json!({ "name": "Owner Mail" }))
        .await
        .unwrap();
    let owner_identifier = owner_account["account"]["identifier"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(owner_account["account"]["mirror_views"], json!(true));

    runtime
        .login(json!({
            "email": "viewer-native@example.test",
            "password": "viewer-user-password"
        }))
        .await
        .unwrap();
    assert_eq!(runtime.imap_accounts_list().await.unwrap(), json!([]));
    assert_eq!(
        runtime
            .imap_account_get(owner_identifier.clone())
            .await
            .unwrap_err()
            .reason,
        "IMAP account not found"
    );
    for error in [
        runtime
            .imap_account_regenerate(owner_identifier.clone())
            .await
            .unwrap_err(),
        runtime
            .imap_account_update(owner_identifier.clone(), json!({ "enabled": false }))
            .await
            .unwrap_err(),
        runtime
            .imap_account_delete(owner_identifier.clone())
            .await
            .unwrap_err(),
    ] {
        assert_eq!(error.reason, "forbidden");
    }
    for error in [
        runtime
            .imap_account_create(json!({ "name": "Viewer Mail" }))
            .await
            .unwrap_err(),
        runtime
            .folder_create(json!({ "name": "Viewer Folder" }))
            .await
            .unwrap_err(),
        runtime
            .view_save(json!({
                "identifier": uuid::Uuid::new_v4().to_string(),
                "owner_identifier": null,
                "name": "Viewer View",
                "filter": "true"
            }))
            .await
            .unwrap_err(),
        runtime
            .messages_set_seen(json!({ "identifiers": [], "seen": true }))
            .await
            .unwrap_err(),
        runtime
            .messages_set_flags(json!({
                "identifiers": [], "operation": "add", "flags": { "flagged": true }
            }))
            .await
            .unwrap_err(),
    ] {
        assert_eq!(error.reason, "forbidden");
    }

    runtime
        .login(json!({
            "email": "member-native@example.test",
            "password": "member-user-password"
        }))
        .await
        .unwrap();
    let member_account = runtime
        .imap_account_create(json!({ "name": "Member Mail" }))
        .await
        .unwrap();
    assert_eq!(member_account["account"]["name"], json!("Member Mail"));

    login_owner(&runtime).await;
    let listed = runtime.imap_accounts_list().await.unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert_eq!(listed[0]["identifier"], json!(owner_identifier));
}

/// Manage Web folders and move visible mail while preserving the HTTP DTO field names.
#[tokio::test]
async fn native_web_folders_list_move_and_query_mail() {
    let (runtime, _application, storage, _completion_sender, _directory) =
        authenticated_test_host().await;
    assert_eq!(
        runtime.folders_list().await.unwrap_err().reason,
        "authentication required"
    );
    login_owner(&runtime).await;

    let folder = runtime
        .folder_create(json!({ "name": "Receipts", "exposedAccounts": [] }))
        .await
        .unwrap();
    assert!(folder["identifier"].is_string());
    assert!(folder["owner_identifier"].is_string());
    assert_eq!(folder["exposed_accounts"], json!([]));
    assert_eq!(
        runtime.folders_list().await.unwrap(),
        json!([folder.clone()])
    );
    assert_eq!(
        runtime
            .folder_update(
                folder["identifier"].as_str().unwrap().into(),
                json!({ "name": "Invoices", "exposedAccounts": [] }),
            )
            .await
            .unwrap()["name"],
        json!("Invoices")
    );

    let identifier = MessageIdentifier::new();
    storage
        .insert_message(&sandpost_core::Message {
            identifier,
            facts: sandpost_core::MessageFacts::default(),
            raw_message: b"Subject: invoice\r\n\r\ninvoice".to_vec(),
            attachments: vec![],
        })
        .await
        .unwrap();
    let moved = runtime
        .messages_move(json!({
            "identifiers": [identifier.to_string()],
            "folderIdentifier": folder["identifier"],
        }))
        .await
        .unwrap();
    assert_eq!(moved, json!({ "moved": 1.0 }));
    let messages = runtime
        .folder_messages(
            folder["identifier"].as_str().unwrap().into(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(messages.as_array().unwrap().len(), 1);
    assert_eq!(messages[0]["id"], json!(identifier.to_string()));
    assert_eq!(
        runtime
            .folder_messages("invalid".into(), None, None, None)
            .await
            .unwrap_err()
            .reason,
        "invalid IMAP folder identifier"
    );
    let trashed = runtime
        .messages_trash(json!({ "identifiers": [identifier.to_string()] }))
        .await
        .unwrap();
    assert_eq!(trashed, json!({ "trashed": 1.0 }));
    let trash = runtime.trash_messages(None, None, None).await.unwrap();
    assert_eq!(trash[0]["id"], json!(identifier.to_string()));
    assert_eq!(
        runtime
            .folder_messages(
                folder["identifier"].as_str().unwrap().into(),
                None,
                None,
                None,
            )
            .await
            .unwrap(),
        json!([]),
        "Trash replaces the folder placement"
    );
    assert_eq!(
        runtime.trash_empty().await.unwrap(),
        json!({ "deleted": 1.0 })
    );
    assert_eq!(
        runtime.trash_messages(None, None, None).await.unwrap(),
        json!([])
    );
    assert_eq!(
        runtime
            .messages_move(json!({ "identifiers": vec![identifier.to_string(); 257] }))
            .await
            .unwrap_err()
            .reason,
        "at most 256 message identifiers may be moved at once"
    );
    runtime
        .folder_delete(folder["identifier"].as_str().unwrap().into())
        .await
        .unwrap();
    assert!(
        runtime
            .folders_list()
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty()
    );
}

/// Native COPY and flag requests match HTTP defaults, authentication, conflicts and summary values.
#[tokio::test]
async fn native_primary_copy_and_flags_preserve_shared_wire_contracts() {
    let (runtime, _application, storage, _completion_sender, _directory) =
        authenticated_test_host().await;
    assert_eq!(
        runtime.messages_copy(json!({})).await.unwrap_err().reason,
        "authentication required"
    );
    assert_eq!(
        runtime
            .messages_set_flags(json!({}))
            .await
            .unwrap_err()
            .reason,
        "authentication required"
    );
    login_owner(&runtime).await;
    let identifier = MessageIdentifier::new();
    storage
        .insert_message(&sandpost_core::Message {
            identifier,
            facts: sandpost_core::MessageFacts::default(),
            raw_message: b"Subject: native flags\r\n\r\nbody".to_vec(),
            attachments: vec![],
        })
        .await
        .unwrap();
    let folder = runtime
        .folder_create(json!({"name":"Copies"}))
        .await
        .unwrap();
    let request = json!({"identifiers":[identifier],"folderIdentifier":folder["identifier"]});
    assert_eq!(
        runtime.messages_copy(request.clone()).await.unwrap(),
        json!({"copied":1.0})
    );
    assert!(runtime.messages_copy(request).await.is_err());
    assert_eq!(
        runtime
            .messages_list(None, None, None, None, None)
            .await
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let request = json!({"identifiers":[identifier],"operation":"add","flags":{"seen":true,"answered":true,"flagged":true,"keywords":["custom"]}});
    assert_eq!(
        runtime.messages_set_flags(request.clone()).await.unwrap(),
        json!({"changed":1.0})
    );
    assert_eq!(
        runtime.messages_set_flags(request).await.unwrap(),
        json!({"changed":0.0})
    );
    let rows = runtime
        .folder_messages(
            folder["identifier"].as_str().unwrap().into(),
            None,
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(rows[0]["seen"], true);
    assert_eq!(
        rows[0]["flags"],
        json!({"seen":true,"answered":true,"flagged":true,"draft":false,"deleted":false,"keywords":["custom"]})
    );
    assert!(
        runtime
            .messages_set_flags(
                json!({"identifiers":[identifier],"operation":"add","flags":{"deleted":true}})
            )
            .await
            .is_err()
    );
    assert_eq!(
        runtime
            .messages_set_flags(
                json!({"identifiers":[identifier],"operation":"replace","flags":{}})
            )
            .await
            .unwrap(),
        json!({"changed":1.0})
    );
    let rows = runtime
        .messages_list(None, None, None, None, None)
        .await
        .unwrap();
    assert_eq!(rows[0]["seen"], false);
    assert_eq!(rows[0]["flags"]["keywords"], json!([]));
    assert!(
        runtime
            .messages_set_flags(
                json!({"identifiers":[identifier],"operation":"invalid","flags":{}})
            )
            .await
            .is_err()
    );
    assert!(
        runtime
            .messages_set_flags(
                json!({"identifiers":[identifier],"operation":"add","flags":{"unknown":true}})
            )
            .await
            .is_err()
    );
}
