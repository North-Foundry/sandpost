//! Cohesive storage capabilities and the composed Storage dependency.
use crate::{
    Attachment, EndpointIdentifier, EndpointMembership, IndexedMessage, Message, MessageFacts,
    MessageIdentifier, MessageListQuery, MessageSequence, MessageSummary, NewUser, Scope,
    ScopeIdentifier, SearchOperation, SearchSynchronization, SmtpEndpoint, StorageError,
    UpdateUser, User, UserIdentifier, View, ViewIdentifier,
};
use async_trait::async_trait;

/// Canonical user accounts, credentials, and instance-wide roles.
#[async_trait]
pub trait UserStorage: Send + Sync {
    /// Load one user by its stable identifier.
    async fn get_user(&self, identifier: UserIdentifier) -> Result<Option<User>, StorageError>;

    /// Load one user by exact stored email.
    async fn get_user_by_email(&self, email: &str) -> Result<Option<User>, StorageError>;

    /// List all users ordered by identifier.
    async fn list_users(&self) -> Result<Vec<User>, StorageError>;

    /// Create a user with the supplied role.
    async fn create_user(&self, user: &NewUser) -> Result<User, StorageError>;

    /// Replace a user's mutable fields, preserving the last-owner invariant.
    ///
    /// Returns None when the identifier is unknown and LastOwner when the change would
    /// remove the final owner.
    async fn update_user(
        &self,
        identifier: UserIdentifier,
        changes: &UpdateUser,
    ) -> Result<Option<User>, StorageError>;

    /// Delete a user, refusing to remove the final owner.
    async fn delete_user(&self, identifier: UserIdentifier) -> Result<bool, StorageError>;

    /// Count existing users for bootstrap decisions.
    async fn count_users(&self) -> Result<u64, StorageError>;

    /// Atomically create the first owner, refusing bootstrap if any user already exists.
    async fn create_first_owner(&self, owner: &NewUser) -> Result<User, StorageError>;
}

/// SMTP endpoints and their per-user memberships.
#[async_trait]
pub trait EndpointStorage: Send + Sync {
    /// Load one endpoint by identifier.
    async fn get_endpoint(
        &self,
        identifier: EndpointIdentifier,
    ) -> Result<Option<SmtpEndpoint>, StorageError>;

    /// List configured endpoints in stable identifier order.
    async fn list_endpoints(&self) -> Result<Vec<SmtpEndpoint>, StorageError>;

    /// Insert or update an endpoint while preserving its identifier.
    async fn save_endpoint(&self, endpoint: &SmtpEndpoint) -> Result<(), StorageError>;

    /// Delete an endpoint; backends reject deletion while scopes or mail reference it.
    async fn delete_endpoint(&self, identifier: EndpointIdentifier) -> Result<bool, StorageError>;

    /// Load one user's membership on an endpoint.
    async fn get_endpoint_membership(
        &self,
        user: UserIdentifier,
        endpoint: EndpointIdentifier,
    ) -> Result<Option<EndpointMembership>, StorageError>;

    /// List every membership of one endpoint ordered by user identifier.
    async fn list_endpoint_memberships(
        &self,
        endpoint: EndpointIdentifier,
    ) -> Result<Vec<EndpointMembership>, StorageError>;

    /// List every endpoint membership held by one user.
    async fn list_user_endpoint_memberships(
        &self,
        user: UserIdentifier,
    ) -> Result<Vec<EndpointMembership>, StorageError>;

    /// Insert or update a membership (upsert on the composite key).
    async fn set_endpoint_membership(
        &self,
        membership: &EndpointMembership,
    ) -> Result<(), StorageError>;

    /// Remove a membership and every dependent scope membership for that endpoint.
    async fn remove_endpoint_membership(
        &self,
        user: UserIdentifier,
        endpoint: EndpointIdentifier,
    ) -> Result<bool, StorageError>;
}

/// Participant-defined mail scopes and their role-less memberships.
#[async_trait]
pub trait ScopeStorage: Send + Sync {
    /// Load one scope by identifier.
    async fn get_scope(&self, identifier: ScopeIdentifier) -> Result<Option<Scope>, StorageError>;

    /// List all scopes ordered by configured position and identifier.
    async fn list_scopes(&self) -> Result<Vec<Scope>, StorageError>;

    /// Insert or update a scope, enforcing policy-version changes for filter or parent edits.
    async fn save_scope(&self, scope: &Scope) -> Result<(), StorageError>;

    /// Delete a scope when no other scope references it as a parent.
    async fn delete_scope(&self, identifier: ScopeIdentifier) -> Result<bool, StorageError>;

    /// Assign a user to a scope.
    ///
    /// Backends must fail with ConstraintViolation when the user lacks an endpoint membership
    /// for the scope's endpoint.
    async fn assign_user_to_scope(
        &self,
        user: UserIdentifier,
        scope: ScopeIdentifier,
    ) -> Result<(), StorageError>;

    /// Remove one user's assignment to a scope.
    async fn remove_user_from_scope(
        &self,
        user: UserIdentifier,
        scope: ScopeIdentifier,
    ) -> Result<bool, StorageError>;

    /// List all scope identifiers assigned to a user in deterministic order.
    async fn list_user_scopes(
        &self,
        user: UserIdentifier,
    ) -> Result<Vec<ScopeIdentifier>, StorageError>;

    /// List users assigned to one scope in deterministic order.
    async fn list_scope_members(
        &self,
        scope: ScopeIdentifier,
    ) -> Result<Vec<UserIdentifier>, StorageError>;
}

/// Personal and shared saved views.
#[async_trait]
pub trait ViewStorage: Send + Sync {
    /// List a user's personal views and all shared views by name and identifier.
    async fn list_views(&self, user: UserIdentifier) -> Result<Vec<View>, StorageError>;

    /// Load one view by identifier.
    async fn get_view(&self, identifier: ViewIdentifier) -> Result<Option<View>, StorageError>;

    /// Insert or update a view.
    async fn save_view(&self, view: &View) -> Result<(), StorageError>;

    /// Delete a view by identifier.
    async fn delete_view(&self, identifier: ViewIdentifier) -> Result<bool, StorageError>;

    /// Create a caller-owned recipient view with transactional duplicate-address protection.
    ///
    /// Compatibility helper for the legacy inbox API; it stores a personal view on the default
    /// endpoint whose filter matches one normalized sandpost.local address.
    async fn create_recipient_view(
        &self,
        user: UserIdentifier,
        name: &str,
        address: &str,
    ) -> Result<View, StorageError>;
}

/// Atomic message ingestion, retrieval, ordering, and deletion.
#[async_trait]
pub trait MessageStorage: Send + Sync {
    /// Insert a normalized message and all relational facts atomically for one endpoint.
    async fn insert_message(
        &self,
        message: &Message,
        endpoint: EndpointIdentifier,
    ) -> Result<MessageSequence, StorageError>;

    /// Load a message and its original bytes from one snapshot.
    async fn get_message(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<Message>, StorageError>;

    /// Load a message, its endpoint, and its original bytes from one snapshot.
    async fn get_message_with_endpoint(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<(EndpointIdentifier, Message)>, StorageError>;

    /// Load normalized facts and attachments without raw bytes.
    async fn get_message_metadata(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<(MessageFacts, Vec<Attachment>)>, StorageError>;

    /// Load endpoint, normalized facts, and attachments without raw bytes.
    async fn get_message_metadata_with_endpoint(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<(EndpointIdentifier, MessageFacts, Vec<Attachment>)>, StorageError>;

    /// List message summaries newest first using endpoint, filter, and keyset bounds.
    async fn list_messages(
        &self,
        query: MessageListQuery,
    ) -> Result<Vec<MessageSummary>, StorageError>;

    /// Delete a message and its cascading child facts in one transaction.
    async fn delete_message(&self, identifier: MessageIdentifier) -> Result<bool, StorageError>;

    /// Delete only the exact authorized revision, returning false for stale or missing mail.
    async fn delete_message_at_revision(
        &self,
        identifier: MessageIdentifier,
        revision: u64,
    ) -> Result<bool, StorageError>;

    /// Return the endpoint currently owning a stored message.
    async fn message_endpoint(
        &self,
        identifier: MessageIdentifier,
    ) -> Result<Option<EndpointIdentifier>, StorageError>;

    /// Return the highest stored mail sequence, or zero when empty.
    async fn max_message_sequence(&self) -> Result<MessageSequence, StorageError>;

    /// Load lightweight summaries in caller-supplied identifier order, omitting missing IDs.
    async fn hydrate_messages(
        &self,
        identifiers: &[MessageIdentifier],
    ) -> Result<Vec<MessageSummary>, StorageError>;

    /// Hydrate only messages whose current revision equals the requested revision.
    async fn hydrate_current_messages(
        &self,
        requested: &[(MessageIdentifier, u64)],
    ) -> Result<Vec<MessageSummary>, StorageError>;

    /// Read normalized index records and revisions after an exclusive sequence, bounded by limit.
    async fn index_messages(
        &self,
        after: MessageSequence,
        through: Option<MessageSequence>,
        limit: usize,
    ) -> Result<Vec<IndexedMessage>, StorageError>;

    /// Load up to the backend batch maximum of normalized index records by identifier.
    async fn indexed_messages(
        &self,
        identifiers: &[MessageIdentifier],
    ) -> Result<Vec<IndexedMessage>, StorageError>;
}

/// Durable search-outbox synchronization state.
#[async_trait]
pub trait SearchSynchronizationStorage: Send + Sync {
    /// Read durable search sequence watermarks and the number of queued operations.
    async fn search_status(&self) -> Result<SearchSynchronization, StorageError>;

    /// Read synchronization state and a bounded pending batch from one snapshot.
    async fn search_batch(
        &self,
        after: u64,
        limit: usize,
    ) -> Result<(SearchSynchronization, Vec<SearchOperation>), StorageError>;

    /// Return pending search operations after an exclusive operation-sequence cursor.
    async fn pending_search_operations(
        &self,
        after: u64,
        limit: usize,
    ) -> Result<Vec<SearchOperation>, StorageError>;

    /// Acknowledge all queued operations through an inclusive sequence in one transaction.
    async fn acknowledge_search_operations(&self, through: u64) -> Result<(), StorageError>;
}

/// Liveness probing for the selected backend.
#[async_trait]
pub trait StorageHealth: Send + Sync {
    /// Check that the backend can serve a trivial request.
    async fn health(&self) -> Result<(), StorageError>;
}

/// The complete storage dependency used by application and transport layers.
///
/// A concrete backend implements the capability traits above; consumers depend only on Storage
/// (typically through an Arc). Every capability is object-safe, so one trait object serves the
/// whole application.
pub trait Storage:
    UserStorage
    + EndpointStorage
    + ScopeStorage
    + ViewStorage
    + MessageStorage
    + SearchSynchronizationStorage
    + StorageHealth
    + Send
    + Sync
{
}

impl<T> Storage for T where
    T: UserStorage
        + EndpointStorage
        + ScopeStorage
        + ViewStorage
        + MessageStorage
        + SearchSynchronizationStorage
        + StorageHealth
        + Send
        + Sync
{
}
