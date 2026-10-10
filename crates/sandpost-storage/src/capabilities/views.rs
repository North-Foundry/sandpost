//! Views storage capability.
use crate::{StorageError, UserIdentifier, View, ViewIdentifier};
use async_trait::async_trait;

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
    /// Compatibility helper for the legacy inbox API; it stores a personal view whose filter
    /// matches one normalized sandpost.local address.
    async fn create_recipient_view(
        &self,
        user: UserIdentifier,
        name: &str,
        address: &str,
    ) -> Result<View, StorageError>;
}
