//! Users storage capability.
use crate::{NewUser, StorageError, UpdateUser, User, UserIdentifier};
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

    /// Delete a user, refusing to remove the final owner; scope memberships and personal views
    /// are removed with it.
    async fn delete_user(&self, identifier: UserIdentifier) -> Result<bool, StorageError>;

    /// Count existing users for bootstrap decisions.
    async fn count_users(&self) -> Result<u64, StorageError>;

    /// Atomically create the first owner, refusing bootstrap if any user already exists.
    async fn create_first_owner(&self, owner: &NewUser) -> Result<User, StorageError>;
}
