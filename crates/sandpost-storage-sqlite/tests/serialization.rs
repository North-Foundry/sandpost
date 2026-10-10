//! Naming changes must preserve persisted JSON and public identifier representations.
use sandpost_core::{
    GlobalRole, Inbox, InboxIdentifier, MailAccess, Message, MessageFacts, MessageIdentifier,
    Scope, ScopeIdentifier, User, UserIdentifier,
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
    let inbox_identifier = InboxIdentifier::new();

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
        global_role: GlobalRole::Admin,
        mail_access: MailAccess::Scoped,
        personal_filter: None,
        created_at: 1,
        updated_at: 2,
    };
    let serialized_user = serde_json::to_value(&user).unwrap();
    assert_eq!(serialized_user["identifier"], user_identifier.to_string());
    assert_eq!(serialized_user["global_role"], "admin");
    assert_eq!(serialized_user["mail_access"], "scoped");

    let inbox = Inbox {
        identifier: inbox_identifier,
        user_identifier,
        name: "Inbox".into(),
        filter: String::new(),
    };
    let serialized_inbox = serde_json::to_value(inbox).unwrap();
    assert_eq!(serialized_inbox["id"], inbox_identifier.to_string());
    assert_eq!(serialized_inbox["user_id"], user_identifier.to_string());

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
