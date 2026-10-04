//! Original behavior checks, grouped by responsibility.
use crate::{Expression, Field, Value, compile};
use sandpost_core::Mailbox;
use sandpost_core::MessageFacts;

/// Build message facts shared by the crate's unit tests.
fn facts() -> MessageFacts {
    MessageFacts {
        from: vec![Mailbox {
            address: "a@example.com".into(),
            domain: "example.com".into(),
        }],
        to: vec![
            Mailbox {
                address: "b@test.org".into(),
                domain: "test.org".into(),
            },
            Mailbox {
                address: "c@other.org".into(),
                domain: "other.org".into(),
            },
        ],
        carbon_copy: vec![],
        subject: "Hello, world".into(),
        text: "A quoted \"word\"".into(),
        received_at: 123,
        size: 42,
        attachment_count: 1,
        headers: [("x-tag".into(), vec!["blue".into(), "green".into()])].into(),
        ..Default::default()
    }
}

mod canonical;
mod evaluation;
mod syntax;
