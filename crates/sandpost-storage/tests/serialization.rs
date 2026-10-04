//! Naming changes must preserve persisted JSON and public identifier representations.
use sandpost_core::{
    Inbox, InboxIdentifier, Membership, Message, MessageFacts, MessageIdentifier, Role, Scope,
    ScopeIdentifier, User, UserIdentifier,
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

/// Preserve historical identifier, raw-message, membership, and administrator keys.
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
        personal_filter: None,
    };
    assert_eq!(
        serde_json::to_value(user).unwrap()["id"],
        user_identifier.to_string()
    );
    let inbox = Inbox {
        identifier: inbox_identifier,
        user_identifier,
        name: "Inbox".into(),
        filter: String::new(),
    };
    let serialized_inbox = serde_json::to_value(inbox).unwrap();
    assert_eq!(serialized_inbox["id"], inbox_identifier.to_string());
    assert_eq!(serialized_inbox["user_id"], user_identifier.to_string());
    let membership = Membership {
        user_identifier,
        scope_identifier,
        role: Role::Administrator,
    };
    assert_eq!(
        serde_json::to_value(membership).unwrap(),
        json!({
            "user_id": user_identifier.to_string(),
            "scope_id": scope_identifier.to_string(),
            "role": "admin"
        })
    );
    assert_eq!(
        serde_json::from_value::<Role>(json!("admin")).unwrap(),
        Role::Administrator
    );
}
