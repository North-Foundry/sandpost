//! Health storage capability.
use crate::StorageError;
use async_trait::async_trait;

/// Liveness probing for the selected backend.
#[async_trait]
pub trait StorageHealth: Send + Sync {
    /// Check that the backend can serve a trivial request.
    async fn health(&self) -> Result<(), StorageError>;
}
