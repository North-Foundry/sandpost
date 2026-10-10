//! Scopes storage capability.
use crate::{Scope, ScopeIdentifier, StorageError, UserIdentifier};
use async_trait::async_trait;

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

    /// Assign a user to a scope; an unknown scope fails with NotFound and an unknown user with
    /// ConstraintViolation.
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
