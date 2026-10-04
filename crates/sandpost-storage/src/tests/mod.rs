//! Persistence regression tests grouped by responsibility.
use crate::{Storage, StorageError};
use sandpost_core::{
    Attachment, Mailbox, Message, MessageFacts, MessageIdentifier, Scope, ScopeIdentifier,
};

/// Create a representative message with sender, recipient, and attachment data.
fn message() -> Message {
    Message {
        identifier: MessageIdentifier::new(),
        facts: MessageFacts {
            envelope_from: Some(Mailbox {
                address: "sender@example.org".into(),
                domain: "example.org".into(),
            }),
            from: vec![Mailbox {
                address: "sender@example.org".into(),
                domain: "example.org".into(),
            }],
            to: vec![Mailbox {
                address: "reader@example.net".into(),
                domain: "example.net".into(),
            }],
            subject: "hello".into(),
            text: "body".into(),
            received_at: 123,
            size: 99,
            attachment_count: 1,
            headers: [("x-tag".into(), vec!["one".into(), "two".into()])].into(),
            ..Default::default()
        },
        raw_message: b"raw".to_vec(),
        attachments: vec![Attachment {
            filename: Some("a.bin".into()),
            content_type: "application/octet-stream".into(),
            size: 3,
            content_hash: "hash".into(),
        }],
    }
}
/// Create a root scope with the requested policy version.
fn scope(policy_version: u64) -> Scope {
    Scope {
        identifier: ScopeIdentifier::new(),
        parent: None,
        name: "root".into(),
        description: None,
        filter: String::new(),
        position: 0,
        policy_version,
    }
}

mod messages;
mod migrations;
mod visibility;
