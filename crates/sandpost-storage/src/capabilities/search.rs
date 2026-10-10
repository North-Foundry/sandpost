//! Search storage capability.
use crate::{SearchOperation, SearchSynchronization, StorageError};
use async_trait::async_trait;

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
