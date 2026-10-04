//! Typed persistence and schema validation errors.
use crate::migrations::SCHEMA_VERSION;
use sandpost_core::{MessageIdentifier, ScopeIdentifier};

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error(transparent)]
    Database(#[from] rusqlite::Error),
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
    #[error("unsupported database schema version {0} (current baseline {SCHEMA_VERSION})")]
    NewerSchema(i64),
    #[error("scope {0} has a different policy version")]
    StalePolicy(ScopeIdentifier),
    #[error("message {0} already exists")]
    DuplicateMessageIdentifier(MessageIdentifier),
    #[error("scope {0} policy versions must increase when its filter or parent changes")]
    PolicyVersionConflict(ScopeIdentifier),
    #[error("integer value is outside SQLite's supported range")]
    IntegerRange,
    #[error("storage connection lock was poisoned")]
    LockPoisoned,
    #[error("invalid stored value: {0}")]
    InvalidData(String),
}
