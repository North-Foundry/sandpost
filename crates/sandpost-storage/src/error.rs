//! Backend-neutral persistence failures.
//!
//! Every concrete backend translates its own driver errors into this enum so callers never
//! depend on a specific database. Backend-specific detail can be preserved through the
//! Backend variant without exposing its concrete type in the public contract.
use sandpost_core::{MessageIdentifier, ScopeIdentifier};

/// A backend-independent storage failure.
#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    /// The backend cannot currently serve the request (for example, a closed writer or lock).
    #[error("storage is unavailable")]
    Unavailable,
    /// A uniqueness or membership invariant was violated by the request.
    #[error("conflicting stored definition")]
    Conflict,
    /// A referenced record does not exist.
    #[error("record not found")]
    NotFound,
    /// A referential or domain constraint rejected the request.
    #[error("constraint violation: {0}")]
    ConstraintViolation(String),
    /// Stored data or caller input was structurally invalid.
    #[error("invalid data: {0}")]
    InvalidData(String),
    /// Schema creation or forward migration failed.
    #[error("migration failed: {0}")]
    Migration(String),
    /// The database schema version is newer than this build supports.
    #[error("unsupported database schema version {0}")]
    NewerSchema(i64),
    /// A message with this public identifier already exists.
    #[error("message {0} already exists")]
    DuplicateMessageIdentifier(MessageIdentifier),
    /// A user with this email already exists.
    #[error("a user with this email already exists")]
    DuplicateUserEmail,
    /// An SMTP access with this username already exists.
    #[error("an SMTP access with this username already exists")]
    DuplicateSmtpUsername,
    /// An IMAP account with this username already exists.
    #[error("an IMAP account with this username already exists")]
    DuplicateImapUsername,
    /// The IMAP account already has a local folder with this name.
    #[error("an IMAP folder with this name already exists")]
    DuplicateImapFolderName,
    /// A compatibility inbox already uses this address for the owner.
    #[error("inbox address already exists")]
    DuplicateInboxAddress,
    /// A compatibility inbox address is not a normalizable local address.
    #[error("inbox address is invalid")]
    InvalidInboxAddress,
    /// A message revision no longer matches the caller's authoritative snapshot.
    #[error("message revision is stale")]
    StaleRevision,
    /// A scope filter or parent changed without advancing its policy version.
    #[error("scope {0} has a different policy version")]
    PolicyVersionConflict(ScopeIdentifier),
    /// The installation must always retain at least one owner.
    #[error("the last owner cannot be removed or demoted")]
    LastOwner,
    /// First-run setup already created a user for this installation.
    #[error("instance already has a user and cannot be bootstrapped")]
    InstanceAlreadyInitialized,
    /// A caller-supplied unsigned value does not fit the backend's integer range.
    #[error("integer value is outside the supported range")]
    IntegerRange,
    /// A bounded batch request exceeded its maximum size.
    #[error("batch exceeds the maximum size of {0}")]
    BatchLimitExceeded(usize),
    /// An opaque backend error retained for logging or diagnostics.
    #[error("storage backend error: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}
