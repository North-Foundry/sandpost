//! Shared domain fixtures for public storage API tests.
use sandpost_core::{
    Message, MessageFacts, MessageIdentifier, MessageSequence, Scope, ScopeIdentifier,
};

/// Build a minimal message with a stable subject for summary assertions.
pub fn message(subject: impl Into<String>) -> Message {
    Message {
        identifier: MessageIdentifier::new(),
        facts: MessageFacts {
            subject: subject.into(),
            ..MessageFacts::default()
        },
        raw_message: Vec::new(),
        attachments: Vec::new(),
    }
}

/// Build a root scope with an empty filter and the requested policy version.
pub fn scope(policy_version: u64) -> Scope {
    Scope {
        identifier: ScopeIdentifier::new(),
        parent: None,
        name: "test scope".into(),
        description: None,
        filter: String::new(),
        position: 0,
        policy_version,
    }
}

/// Return the public sequence associated with a newly inserted test message.
pub fn insert_message(
    storage: &sandpost_storage::Storage,
    subject: impl Into<String>,
) -> MessageSequence {
    storage
        .insert_message(&message(subject), &[])
        .expect("insert test message")
}
