use sandpost_core::{Mailbox, MessageFacts};
use std::collections::BTreeMap;

/// Build one mailbox fixture with the supplied address and normalized domain.
pub fn mailbox(address: &str, domain: &str) -> Mailbox {
    Mailbox {
        address: address.into(),
        domain: domain.into(),
    }
}

/// Build the shared message fixture used by integration tests.
pub fn facts() -> MessageFacts {
    MessageFacts {
        envelope_from: Some(mailbox("sender@example.com", "example.com")),
        envelope_to: vec![mailbox("Envelope@one.test", "one.test")],
        from: vec![mailbox("sender@example.com", "example.com")],
        to: vec![
            mailbox("first@one.test", "one.test"),
            mailbox("second@two.test", "two.test"),
        ],
        carbon_copy: vec![mailbox("copy@three.test", "three.test")],
        subject: "Café Hello, World".into(),
        text: "Body has Straße and 🦀".into(),
        markup_body: "<b>Bright &amp; bold</b>".into(),
        message_identifier: Some("<msg.1@example.com>".into()),
        received_at: i64::MIN,
        size: u64::MAX,
        attachment_count: 2,
        headers: BTreeMap::from([
            ("x-tag".into(), vec!["Blue".into(), "green".into()]),
            ("subject".into(), vec!["Header value".into()]),
        ]),
    }
}
