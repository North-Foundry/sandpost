//! Translation of SQLite driver errors into backend-neutral storage errors.
use sandpost_storage::StorageError;

/// Translate one rusqlite error, preserving constraint detail without exposing the driver type.
pub(crate) fn sqlite_error(error: rusqlite::Error) -> StorageError {
    match error {
        rusqlite::Error::SqliteFailure(code, message)
            if code.code == rusqlite::ErrorCode::ConstraintViolation =>
        {
            StorageError::ConstraintViolation(
                message.unwrap_or_else(|| "constraint violation".into()),
            )
        }
        error => StorageError::Backend(Box::new(error)),
    }
}

/// Convert rusqlite results with the shared translation.
pub(crate) trait StorageResult<T> {
    /// Map a rusqlite error into a semantic storage error.
    fn storage(self) -> Result<T, StorageError>;
}

impl<T> StorageResult<T> for Result<T, rusqlite::Error> {
    /// Translate a driver result into the backend-independent storage error contract.
    fn storage(self) -> Result<T, StorageError> {
        self.map_err(sqlite_error)
    }
}
