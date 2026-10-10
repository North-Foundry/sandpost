//! Reusable backend conformance suite for the SandPost storage contract.
//!
//! A third-party backend runs this suite against one dyn Storage implementation to prove it
//! satisfies the behaviour the application relies on. The suite is self-contained: it creates
//! its own users, SMTP accesses, IMAP accounts, scopes, views, and messages with fresh identifiers and depends only
//! on the storage contract, the shared domain types, the canonical query AST, and Tokio.
//!
//! The storage under test must be empty when run_all starts, because the first check proves
//! bootstrap behaviour that is only observable before any user exists.
mod imap;
mod messages;
mod scopes;
mod search;
mod smtp_server;
mod support;
mod users;
mod views;

pub use imap::{
    imap_account_configuration, imap_account_credentials, imap_flags_and_expunge,
    imap_mailbox_identities, imap_membership_synchronization,
};
pub use messages::{
    index_messages_revisions, message_deletion, message_filter_equivalence,
    message_ingestion_and_retrieval, message_ordering_and_pagination,
};
pub use scopes::{scope_crud, scope_memberships};
pub use search::search_outbox;
pub use smtp_server::{
    smtp_access_authentication_rules, smtp_access_credentials, smtp_server_address,
};
pub use users::{bootstrap, duplicate_user_email, global_roles, last_owner_invariant, user_crud};
pub use views::view_crud;

use sandpost_storage::Storage;

use support::{unwrap_storage, verify};

/// A single failed conformance check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConformanceFailure {
    /// Name of the failed check.
    pub check: &'static str,
    /// What the check observed and what it expected.
    pub detail: String,
}

impl std::fmt::Display for ConformanceFailure {
    /// Render the failing check name together with the observed detail.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "conformance check '{}' failed: {}",
            self.check, self.detail
        )
    }
}

impl std::error::Error for ConformanceFailure {}

/// Verify the backend answers a liveness probe.
pub async fn health(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    const CHECK: &str = "health";
    unwrap_storage(CHECK, "health", storage.health().await)
}

/// Run every conformance check against an empty storage implementation.
///
/// The checks run in a fixed order and stop at the first failure. The storage must be empty:
/// this function fails with the run_all check when any user already exists.
pub async fn run_all(storage: &dyn Storage) -> Result<(), ConformanceFailure> {
    let existing = unwrap_storage("run_all", "count_users", storage.count_users().await)?;
    verify(
        "run_all",
        existing == 0,
        format!("the conformance suite requires an empty storage but found {existing} users"),
    )?;
    health(storage).await?;
    bootstrap(storage).await?;
    user_crud(storage).await?;
    duplicate_user_email(storage).await?;
    global_roles(storage).await?;
    last_owner_invariant(storage).await?;
    smtp_server_address(storage).await?;
    smtp_access_credentials(storage).await?;
    smtp_access_authentication_rules(storage).await?;
    scope_crud(storage).await?;
    scope_memberships(storage).await?;
    view_crud(storage).await?;
    message_ingestion_and_retrieval(storage).await?;
    message_ordering_and_pagination(storage).await?;
    message_filter_equivalence(storage).await?;
    message_deletion(storage).await?;
    index_messages_revisions(storage).await?;
    search_outbox(storage).await?;
    imap_account_credentials(storage).await?;
    imap_account_configuration(storage).await?;
    imap_mailbox_identities(storage).await?;
    imap_membership_synchronization(storage).await?;
    imap_flags_and_expunge(storage).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::ConformanceFailure;

    /// The failure message must name the check that failed.
    #[test]
    fn failure_names_the_failed_check() {
        let failure = ConformanceFailure {
            check: "example_check",
            detail: "observed something unexpected".to_owned(),
        };
        let rendered = failure.to_string();
        assert!(rendered.contains("example_check"), "{rendered}");
        assert!(
            rendered.contains("observed something unexpected"),
            "{rendered}"
        );
    }
}
