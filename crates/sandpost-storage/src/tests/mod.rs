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

/// Build a complete message fixture with overlapping ordered recipients and attachments.
fn normalized_message() -> Message {
    let mailbox = |address: &str, domain: &str| Mailbox {
        address: address.into(),
        domain: domain.into(),
    };
    Message {
        identifier: MessageIdentifier::new(),
        facts: MessageFacts {
            envelope_from: None,
            envelope_to: vec![
                mailbox("shared@example.test", "example.test"),
                mailbox("envelope@example.net", "example.net"),
            ],
            from: vec![
                mailbox("shared@example.test", "example.test"),
                mailbox("sender@example.org", "example.org"),
            ],
            to: vec![
                mailbox("shared@example.test", "example.test"),
                mailbox("reader@example.net", "example.net"),
            ],
            carbon_copy: vec![
                mailbox("reader@example.net", "example.net"),
                mailbox("reader@example.net", "example.net"),
            ],
            subject: "Normalized mail roundtrip".into(),
            text: "plain body".into(),
            markup_body: "<p>markup body</p>".into(),
            message_identifier: Some("normalized@example.test".into()),
            received_at: 1_791_072_000,
            size: 812,
            attachment_count: 73,
            headers: [
                ("x-repeat".into(), vec!["first".into(), "second".into()]),
                ("subject".into(), vec!["header subject".into()]),
            ]
            .into(),
        },
        raw_message: b"raw normalized message".to_vec(),
        attachments: vec![
            Attachment {
                filename: Some("first.bin".into()),
                content_type: "application/octet-stream".into(),
                size: 7,
                content_hash: "first-hash".into(),
            },
            Attachment {
                filename: None,
                content_type: "text/plain".into(),
                size: 11,
                content_hash: "second-hash".into(),
            },
        ],
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
