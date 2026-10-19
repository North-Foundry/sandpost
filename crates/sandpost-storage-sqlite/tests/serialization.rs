//! Naming changes must preserve persisted JSON and public identifier representations.
use sandpost_core::{
    GlobalRole, MailAccess, Message, MessageFacts, MessageIdentifier, Scope, ScopeIdentifier, User,
    UserIdentifier, View, ViewIdentifier,
};
use serde_json::json;

/// Read and write the original message-facts representation without changing its keys.
#[test]
fn message_facts_preserve_the_original_serialized_fields() {
    let stored_facts = json!({
        "envelope_from": null,
        "envelope_to": [],
        "from": [],
        "to": [],
        "cc": [{ "address": "copy@example.test", "domain": "example.test" }],
        "subject": "Original representation",
        "text": "Plain body",
        "html": "<p>Markup body</p>",
        "message_id": "old-message@example.test",
        "received_at": 1791072000,
        "size": 42,
        "attachment_count": 0,
        "headers": { "x-test": ["first", "second"] }
    });
    let facts: MessageFacts = serde_json::from_value(stored_facts.clone()).unwrap();
    assert_eq!(facts.carbon_copy[0].address, "copy@example.test");
    assert_eq!(facts.markup_body, "<p>Markup body</p>");
    assert_eq!(
        facts.message_identifier.as_deref(),
        Some("old-message@example.test")
    );
    assert_eq!(serde_json::to_value(facts).unwrap(), stored_facts);
}

/// Preserve historical identifier, raw-message, role, membership, and view keys.
#[test]
fn domain_models_preserve_identifiers_and_role_representation() {
    let message_identifier = MessageIdentifier::new();
    let scope_identifier = ScopeIdentifier::new();
    let user_identifier = UserIdentifier::new();
    let view_identifier = ViewIdentifier::new();

    let message = Message {
        identifier: message_identifier,
        facts: MessageFacts::default(),
        raw_message: vec![65],
        attachments: vec![],
    };
    let serialized_message = serde_json::to_value(&message).unwrap();
    assert_eq!(serialized_message["id"], message_identifier.to_string());
    assert_eq!(serialized_message["raw_mime"], json!([65]));
    assert!(serialized_message.get("identifier").is_none());
    let restored_message: Message = serde_json::from_value(serialized_message).unwrap();
    assert_eq!(restored_message.identifier, message_identifier);
    assert_eq!(restored_message.raw_message, vec![65]);

    let scope = Scope {
        identifier: scope_identifier,
        parent: None,
        name: "Scope".into(),
        description: None,
        filter: String::new(),
        position: 0,
        policy_version: 1,
    };
    assert_eq!(
        serde_json::to_value(scope).unwrap()["id"],
        scope_identifier.to_string()
    );

    let user = User {
        identifier: user_identifier,
        name: "User".into(),
        email: "user@example.test".into(),
        password_hash: "hash".into(),
        global_role: GlobalRole::Owner,
        mail_access: MailAccess::Scoped,
        personal_filter: None,
        created_at: 1,
        updated_at: 2,
    };
    let serialized_user = serde_json::to_value(&user).unwrap();
    assert_eq!(serialized_user["identifier"], user_identifier.to_string());
    assert_eq!(serialized_user["global_role"], "owner");
    assert_eq!(serialized_user["mail_access"], "scoped");

    let view = View {
        identifier: view_identifier,
        owner_identifier: Some(user_identifier),
        name: "View".into(),
        icon: None,
        filter: String::new(),
    };
    let serialized_view = serde_json::to_value(&view).unwrap();
    assert_eq!(
        serialized_view,
        json!({
            "identifier": view_identifier.to_string(),
            "owner_identifier": user_identifier.to_string(),
            "name": "View",
            "icon": null,
            "filter": ""
        })
    );
    assert_eq!(
        serde_json::from_value::<View>(serialized_view).unwrap(),
        view
    );

    assert_eq!(
        serde_json::from_value::<GlobalRole>(json!("viewer")).unwrap(),
        GlobalRole::Viewer
    );
    assert_eq!(
        serde_json::from_value::<GlobalRole>(json!("owner")).unwrap(),
        GlobalRole::Owner
    );
    assert_eq!(
        serde_json::from_value::<MailAccess>(json!("all")).unwrap(),
        MailAccess::All
    );
}

/**
 * Older View and WebFolder payloads omit icons and continue to deserialize as `None`.
 *
 * These defaults keep existing API and native bridge payloads compatible.
 */
#[test]
fn icon_fields_default_when_omitted() {
    let view: View = serde_json::from_value(json!({
        "identifier": ViewIdentifier::new(),
        "owner_identifier": null,
        "name": "Legacy",
        "filter": "true"
    }))
    .unwrap();
    assert_eq!(view.icon, None);

    let folder: sandpost_core::WebFolder = serde_json::from_value(json!({
        "identifier": sandpost_core::ImapFolderIdentifier::new(),
        "owner_identifier": sandpost_core::UserIdentifier::new(),
        "name": "Legacy folder",
        "exposed_accounts": [],
        "created_at": 1,
        "updated_at": 2
    }))
    .unwrap();
    assert_eq!(folder.icon, None);
}
