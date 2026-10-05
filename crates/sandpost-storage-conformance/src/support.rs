//! Shared helpers for the conformance checks.
use std::collections::BTreeMap;
use std::fmt::Debug;

use sandpost_core::{
    Attachment, EndpointIdentifier, GlobalRole, Mailbox, Message, MessageFacts, MessageIdentifier,
    Scope, ScopeIdentifier, SmtpEndpoint, User,
};
use sandpost_storage::{NewUser, Storage, StorageError};

use crate::ConformanceFailure;

/// Build a failure that names the check which failed.
pub(crate) fn failure(check: &'static str, detail: impl Into<String>) -> ConformanceFailure {
    ConformanceFailure {
        check,
        detail: detail.into(),
    }
}

/// Convert a storage result into a named conformance failure.
pub(crate) fn unwrap_storage<T>(
    check: &'static str,
    context: &str,
    result: Result<T, StorageError>,
) -> Result<T, ConformanceFailure> {
    result.map_err(|error| failure(check, format!("{context}: {error}")))
}

/// Fail with the check name unless the condition holds.
pub(crate) fn verify(
    check: &'static str,
    condition: bool,
    detail: impl Into<String>,
) -> Result<(), ConformanceFailure> {
    if condition {
        Ok(())
    } else {
        Err(failure(check, detail))
    }
}

/// Fail with the check name unless the two values are equal.
pub(crate) fn verify_equal<T: PartialEq + Debug>(
    check: &'static str,
    what: &str,
    actual: T,
    expected: T,
) -> Result<(), ConformanceFailure> {
    if actual == expected {
        Ok(())
    } else {
        Err(failure(
            check,
            format!("{what}: got {actual:?}, expected {expected:?}"),
        ))
    }
}

/// Return a token unique within this process for building isolated test data.
pub(crate) fn unique_token() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    COUNTER.fetch_add(1, Ordering::Relaxed) + 1
}

/// Build a unique email address so checks never collide on the unique-email constraint.
pub(crate) fn unique_email(prefix: &str) -> String {
    format!(
        "{prefix}-{}-{}@conformance.test",
        std::process::id(),
        unique_token()
    )
}

/// Build a new user record for the given role.
pub(crate) fn new_user(role: GlobalRole, email: &str) -> NewUser {
    NewUser {
        name: format!("conformance {role:?}"),
        email: email.to_owned(),
        password_hash: "conformance-hash".to_owned(),
        global_role: role,
        personal_filter: None,
    }
}

/// Create a user and return the stored record.
pub(crate) async fn create_user(
    storage: &dyn Storage,
    check: &'static str,
    role: GlobalRole,
    email: &str,
) -> Result<User, ConformanceFailure> {
    unwrap_storage(
        check,
        "create_user",
        storage.create_user(&new_user(role, email)).await,
    )
}

/// Save a fresh endpoint and return its identifier.
pub(crate) async fn save_endpoint(
    storage: &dyn Storage,
    check: &'static str,
    name: &str,
) -> Result<EndpointIdentifier, ConformanceFailure> {
    let endpoint = SmtpEndpoint {
        identifier: EndpointIdentifier::new(),
        name: name.to_owned(),
    };
    unwrap_storage(
        check,
        "save_endpoint",
        storage.save_endpoint(&endpoint).await,
    )?;
    Ok(endpoint.identifier)
}

/// Save a fresh root scope on an endpoint and return its identifier.
pub(crate) async fn save_scope(
    storage: &dyn Storage,
    check: &'static str,
    endpoint: EndpointIdentifier,
    name: &str,
) -> Result<ScopeIdentifier, ConformanceFailure> {
    let scope = Scope {
        identifier: ScopeIdentifier::new(),
        endpoint_identifier: endpoint,
        parent: None,
        name: name.to_owned(),
        description: None,
        filter: String::new(),
        position: 0,
        policy_version: 1,
    };
    unwrap_storage(check, "save_scope", storage.save_scope(&scope).await)?;
    Ok(scope.identifier)
}

/// Build a minimal message with only the supplied subject.
pub(crate) fn simple_message(subject: &str) -> Message {
    Message {
        identifier: MessageIdentifier::new(),
        facts: MessageFacts {
            subject: subject.to_owned(),
            ..MessageFacts::default()
        },
        raw_message: Vec::new(),
        attachments: Vec::new(),
    }
}

/// Build a mailbox, deriving its domain from the address.
pub(crate) fn mailbox(address: &str) -> Mailbox {
    Mailbox {
        address: address.to_owned(),
        domain: address
            .rsplit_once('@')
            .map(|(_, domain)| domain.to_owned())
            .unwrap_or_default(),
    }
}

/// Build an attachment with the given filename, size, and content hash.
pub(crate) fn attachment(filename: &str, size: u64, content_hash: &str) -> Attachment {
    Attachment {
        filename: Some(filename.to_owned()),
        content_type: "application/octet-stream".to_owned(),
        size,
        content_hash: content_hash.to_owned(),
    }
}

/// Build a header map from ordered name and values pairs.
pub(crate) fn headers(entries: &[(&str, &[&str])]) -> BTreeMap<String, Vec<String>> {
    entries
        .iter()
        .map(|(name, values)| {
            (
                (*name).to_owned(),
                values.iter().map(|value| (*value).to_owned()).collect(),
            )
        })
        .collect()
}
