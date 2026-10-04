use sandpost_core::{Mailbox, MessageFacts, Scope, ScopeIdentifier};

/// Build a scope fixture from its parent and local filter.
pub fn scope(parent: Option<ScopeIdentifier>, filter: &str) -> Scope {
    Scope {
        identifier: ScopeIdentifier::new(),
        parent,
        name: "test".into(),
        description: None,
        filter: filter.into(),
        position: 0,
        policy_version: 1,
    }
}

/// Build normalized sender facts for the given domain.
pub fn facts(domain: &str) -> MessageFacts {
    MessageFacts {
        from: vec![Mailbox {
            address: format!("dev@{domain}"),
            domain: domain.into(),
        }],
        subject: "hello".into(),
        ..MessageFacts::default()
    }
}
