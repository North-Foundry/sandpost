//! Transport-independent authorization, authoritative messages, and disposable search.
mod authorization;
mod changes;
mod credentials;
mod imap;
mod inboxes;
mod indexing;
mod instance;
mod listeners;
mod mailbox;
mod management;
mod read_status;
mod smtp_access;
pub use changes::ApplicationChange;
pub use imap::{
    ImapAccessError, ImapAccountDetails, ImapAccountSession, ImapAccountUpdate, ImapChangeWatch,
    ImapFolderRequest, ImapLogin, ImapMailboxHandle, ImapMailboxListing, ImapMailboxPreview,
    ImapMailboxSnapshot, ImapMailboxStatus, ImapMembershipChanges, ImapSecurity, ImapServerDetails,
    ImapService, IssuedImapCredentials, NewImapAccountRequest,
};
pub use inboxes::InboxDetails;
pub use indexing::SearchCoordinator;
pub use instance::{
    Instance, InstanceError, InstanceStatus, LoginRequest, LoginResponse, SessionUser, SetupRequest,
};
pub use listeners::{
    MailListenerControl, MailListenerEncryption, MailListenerSource, MailListenerState,
    MailListenerStatus,
};
pub use mailbox::WebFolderRequest;
pub use management::{
    AccountDetails, NewAccount, QueryDiagnostic, QueryValidation, ScopePreview, UpdateAccount,
};
pub use read_status::WebMessageFlagsRequest;
use sandpost_core::{
    GlobalRole, MessageFacts, MessageIdentifier, MessageSequence, UserIdentifier, ViewIdentifier,
};
use sandpost_query::{Expression, Field, Operator, Predicate, Value};
pub use sandpost_storage::{MessageSummary, SearchSynchronization};
use sandpost_storage::{Storage, StorageError};
use serde::Serialize;
pub use smtp_access::{
    IssuedSmtpCredentials, NewSmtpAccessRequest, SmtpAccessUpdate, SmtpAuthentication,
    SmtpAuthenticator, SmtpPrincipal, SmtpServerDetails,
};
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::broadcast;

/// Re-export the canonical storage capability used across the application boundary.
pub use sandpost_storage::Storage as StorageContract;

/// Search candidate batch size supported by every backend's index hydration contract.
const SEARCH_CANDIDATE_PAGE_SIZE: usize = 64;

/// A committed notification; transports must authorize it before exposing identifiers.
#[derive(Debug, Clone, Serialize)]
pub struct MessageEvent {
    #[serde(rename = "id")]
    pub identifier: MessageIdentifier,
    #[serde(rename = "seq")]
    pub sequence: MessageSequence,
}
/// SQL-owned facts and attachment metadata for an authorized message.
#[derive(Debug, Clone, Serialize)]
pub struct MessageDetail {
    #[serde(rename = "id")]
    pub identifier: MessageIdentifier,
    pub facts: MessageFacts,
    pub attachments: Vec<sandpost_core::Attachment>,
}
/// Selection constraints composed after security authorization.
#[derive(Debug, Clone)]
pub struct QueryOptions {
    pub view_identifier: Option<ViewIdentifier>,
    pub query: Option<String>,
    pub before: Option<MessageSequence>,
    pub limit: usize,
}
impl Default for QueryOptions {
    /// Select the newest fifty authorized messages.
    fn default() -> Self {
        Self {
            view_identifier: None,
            query: None,
            before: None,
            limit: 50,
        }
    }
}
/// Application failures shared by every transport.
#[derive(Debug, Error)]
pub enum ApplicationError {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("conflicting definition")]
    Conflict,
    #[error("{0}")]
    InUse(String),
    #[error("limit must be between 1 and 100")]
    InvalidPage,
    #[error("item not found")]
    NotFound,
    #[error("operation is not permitted")]
    Forbidden,
    #[error("search unavailable: {0}")]
    Unavailable(String),
    #[error("storage operation failed: {0}")]
    Storage(#[from] StorageError),
    #[error("search operation failed: {0}")]
    Search(#[from] sandpost_search::SearchError),
    #[error("filesystem operation failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("instance operation failed: {0}")]
    Instance(#[from] InstanceError),
    #[error("application worker failed: {0}")]
    Worker(#[from] tokio::task::JoinError),
}
/// Shared state; normal message operations require an explicit authenticated principal.
#[derive(Clone)]
pub struct Application {
    storage: Arc<dyn Storage>,
    search: SearchCoordinator,
    events: broadcast::Sender<MessageEvent>,
    instance: Instance,
    maximum_scope_depth: Option<usize>,
    mail_listeners: Option<Arc<dyn MailListenerControl>>,
    /// Committed changes that may alter what IMAP sessions show; see [`ApplicationChange`].
    changes: broadcast::Sender<ApplicationChange>,
    /// Connection details of the runtime's IMAP service, when one runs.
    imap_server: Arc<std::sync::RwLock<Option<ImapServerDetails>>>,
}
/// Reusable message authorization boundary for HTTP, native hosts, and other callers.
#[derive(Clone)]
pub struct AuthorizedApplication {
    application: Application,
    user_identifier: UserIdentifier,
}
impl Application {
    /// Construct a storage-only application with an isolated derived index and locked instance.
    pub async fn new(
        storage: Arc<dyn Storage>,
        events: broadcast::Sender<MessageEvent>,
    ) -> Result<Self, ApplicationError> {
        let instance = Instance::locked(storage.clone());
        Self::with_instance(storage, events, instance).await
    }
    /// Construct a test or embedded application with resolved installation identity.
    pub async fn with_instance(
        storage: Arc<dyn Storage>,
        events: broadcast::Sender<MessageEvent>,
        instance: Instance,
    ) -> Result<Self, ApplicationError> {
        let search = SearchCoordinator::memory(storage.clone()).await?;
        Ok(Self::with_search(storage, events, instance, search))
    }
    /// Attach the installation's durable index coordinator.
    pub fn with_search(
        storage: Arc<dyn Storage>,
        events: broadcast::Sender<MessageEvent>,
        instance: Instance,
        search: SearchCoordinator,
    ) -> Self {
        Self {
            storage,
            search,
            events,
            instance,
            maximum_scope_depth: None,
            mail_listeners: None,
            changes: broadcast::channel(1024).0,
            imap_server: Arc::new(std::sync::RwLock::new(None)),
        }
    }
    /// Attach the hosting runtime's read-only SMTP listener status.
    pub fn with_mail_listeners(mut self, control: Arc<dyn MailListenerControl>) -> Self {
        self.mail_listeners = Some(control);
        self
    }
    /// Apply the runtime hierarchy limit to management edits as well as startup validation.
    pub fn with_scope_depth_limit(mut self, maximum_depth: Option<usize>) -> Self {
        self.maximum_scope_depth = maximum_depth;
        self
    }
    /// Bind a transport-authenticated identity; roles and memberships are reread per operation.
    pub fn authorized(&self, user: &SessionUser) -> AuthorizedApplication {
        AuthorizedApplication {
            application: self.clone(),
            user_identifier: user.id,
        }
    }
    /// Return the shared authentication authority.
    pub fn instance(&self) -> &Instance {
        &self.instance
    }
    /// Return the coordinator used by the supervised worker and CLI maintenance.
    pub fn search_coordinator(&self) -> &SearchCoordinator {
        &self.search
    }
    /// Wait for the captured SQL mutation watermark before delivering list-refresh notifications.
    pub async fn wait_for_search(&self) -> Result<(), ApplicationError> {
        let sequence = self.storage.search_status().await?.latest_sequence;
        self.search.wait_for_sequence(sequence).await
    }
    /// Subscribe to internal notifications; authorize each before public delivery.
    pub fn subscribe(&self) -> broadcast::Receiver<MessageEvent> {
        self.events.subscribe()
    }
    /// Publish a committed notification to bounded internal subscribers.
    pub fn publish_message(
        &self,
        event: MessageEvent,
    ) -> Result<usize, broadcast::error::SendError<MessageEvent>> {
        self.events.send(event)
    }
    /// Probe the backend and the active index without blocking Tokio listener threads.
    pub async fn health(&self) -> Result<(), ApplicationError> {
        self.storage.health().await?;
        let search = self.search.clone();
        tokio::task::spawn_blocking(move || {
            search.search(&sandpost_search::MessageQuery {
                authorization: Some(Expression::False),
                filter: Expression::True,
                message_identifier: None,
                before: None,
                limit: 1,
            })
        })
        .await??;
        Ok(())
    }
}
impl AuthorizedApplication {
    /// Locate authorized IDs through Tantivy, then hydrate lightweight SQL metadata in batches.
    pub async fn query_messages(
        &self,
        options: QueryOptions,
    ) -> Result<Vec<MessageSummary>, ApplicationError> {
        validate_page_size(options.limit)?;
        let application = self.application.clone();
        let user = self.user_identifier;
        let query = authorization::compose(&application.storage, user, &options, None).await?;
        self.query_visible_messages(query).await
    }
    /// Return newest mail already visible to this principal.
    pub async fn list_messages(
        &self,
        before: Option<MessageSequence>,
        limit: usize,
    ) -> Result<Vec<MessageSummary>, ApplicationError> {
        self.query_web_mailbox(
            QueryOptions {
                before,
                limit,
                ..QueryOptions::default()
            },
            None,
        )
        .await
    }
    /// Adapt the inspector's literal search to canonical content predicates.
    pub async fn search_messages(
        &self,
        query: String,
        before: Option<MessageSequence>,
        limit: usize,
    ) -> Result<Vec<MessageSummary>, ApplicationError> {
        validate_page_size(limit)?;
        let query = query.trim().to_owned();
        if query.is_empty() || query.chars().count() > 256 {
            return Err(ApplicationError::InvalidInput(
                "search query must contain 1 to 256 characters".into(),
            ));
        }
        let application = self.application.clone();
        let user = self.user_identifier;
        let mut compiled = authorization::compose(
            &application.storage,
            user,
            &QueryOptions {
                before,
                limit,
                ..QueryOptions::default()
            },
            None,
        )
        .await?;
        compiled.filter = Expression::And(vec![compiled.filter, literal_search(&query)]);
        self.query_visible_messages(compiled).await
    }
    /// Refill an authorized result page after SQL excludes tombstones or stale index revisions.
    /// Indexed sequences advance the cursor even when every candidate has physically vanished.
    pub(crate) async fn query_visible_messages(
        &self,
        mut query: sandpost_search::MessageQuery,
    ) -> Result<Vec<MessageSummary>, ApplicationError> {
        let limit = query.limit;
        let filter = authorization::storage_filter(&query);
        query.limit = SEARCH_CANDIDATE_PAGE_SIZE;
        let mut results = Vec::with_capacity(limit);
        while results.len() < limit {
            let search = self.application.search.clone();
            let page_query = query.clone();
            let hits =
                tokio::task::spawn_blocking(move || search.search_hits_with_sequences(&page_query))
                    .await??;
            let Some((_, _, oldest)) = hits.last() else {
                break;
            };
            query.before = Some(*oldest);
            let requested: Vec<_> = hits
                .iter()
                .map(|(identifier, revision, _)| (*identifier, *revision))
                .collect();
            let summaries = self
                .application
                .storage
                .hydrate_current_user_messages(self.user_identifier, &requested, &filter)
                .await?;
            results.extend(summaries.into_iter().take(limit - results.len()));
            if hits.len() < SEARCH_CANDIDATE_PAGE_SIZE {
                break;
            }
        }
        Ok(results)
    }

    /// Check current SQL facts using exactly the same mail access and scope composition.
    pub async fn is_message_visible(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<bool, ApplicationError> {
        authorization::is_visible(&self.application.storage, self.user_identifier, identifier).await
    }
    /// Read facts only after centralized direct-access authorization.
    pub async fn get_message(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<MessageDetail, ApplicationError> {
        let (facts, attachments) = self
            .application
            .storage
            .get_user_message_metadata(self.user_identifier, identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        authorization::require_snapshot(&self.application.storage, self.user_identifier, &facts)
            .await?;
        Ok(MessageDetail {
            identifier,
            facts,
            attachments,
        })
    }
    /// Read original MIME through the same direct-access authorization boundary.
    pub async fn get_raw_message(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Vec<u8>, ApplicationError> {
        let message = self
            .application
            .storage
            .get_user_message(self.user_identifier, identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        authorization::require_snapshot(
            &self.application.storage,
            self.user_identifier,
            &message.facts,
        )
        .await?;
        Ok(message.raw_message)
    }
    /// Decode an attachment only after authorizing its containing message.
    pub async fn get_attachment(
        &self,
        identifier: MessageIdentifier,
        index: usize,
    ) -> Result<Vec<u8>, ApplicationError> {
        let message = self
            .application
            .storage
            .get_user_message(self.user_identifier, identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        authorization::require_snapshot(
            &self.application.storage,
            self.user_identifier,
            &message.facts,
        )
        .await?;
        tokio::task::spawn_blocking(move || {
            sandpost_mime::attachment_bytes(&message.raw_message, index)
                .map_err(|_| ApplicationError::NotFound)
        })
        .await?
    }
    /// Permanently hide authorized mail for this user while retaining its physical contents.
    pub async fn delete_message(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<(), ApplicationError> {
        let account = self.account().await?;
        authorization::require_action(account.global_role != GlobalRole::Viewer)?;
        let query = authorization::compose(
            &self.application.storage,
            self.user_identifier,
            &QueryOptions::default(),
            None,
        )
        .await?;
        let deleted = self
            .application
            .storage
            .delete_user_messages(
                self.user_identifier,
                Some(&[identifier]),
                &authorization::storage_filter(&query),
            )
            .await?;
        if deleted == 0 {
            return Err(ApplicationError::NotFound);
        }
        self.notify(ApplicationChange::Mail);
        Ok(())
    }

    /// Atomically hide every currently authorized, undeleted physical message for this user.
    /// This operation uses authoritative storage and does not depend on search-index progress.
    pub async fn delete_all_messages(&self) -> Result<u64, ApplicationError> {
        let account = self.account().await?;
        authorization::require_action(account.global_role != GlobalRole::Viewer)?;
        let query = authorization::compose(
            &self.application.storage,
            self.user_identifier,
            &QueryOptions::default(),
            None,
        )
        .await?;
        let deleted = self
            .application
            .storage
            .delete_user_messages(
                self.user_identifier,
                None,
                &authorization::storage_filter(&query),
            )
            .await?;
        if deleted > 0 {
            self.notify(ApplicationChange::Mail);
        }
        Ok(deleted)
    }
}
/// Build a literal search expression from existing DSL fields and operators.
fn literal_search(value: &str) -> Expression {
    Expression::Predicate(Predicate {
        field: Field::Content,
        operator: Operator::Contains,
        value: Value::String(value.to_owned()),
    })
}
/// Enforce the public page-size bound.
pub(crate) fn validate_page_size(limit: usize) -> Result<(), ApplicationError> {
    if (1..=100).contains(&limit) {
        Ok(())
    } else {
        Err(ApplicationError::InvalidPage)
    }
}
/// Validate printable bounded names at domain-management boundaries.
fn validated_name(value: &str) -> Result<String, ApplicationError> {
    instance::validated_name(value)
        .map_err(|error| ApplicationError::InvalidInput(error.to_string()))
}
#[cfg(test)]
mod tests;
