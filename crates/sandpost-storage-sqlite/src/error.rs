//! Translation of SQLite driver errors into backend-neutral storage errors.
use sandpost_storage::StorageError;

/// Translate driver failures while keeping constraint and availability errors semantic.
pub(crate) fn sqlite_error(error: sqlx::Error) -> StorageError {
    match error {
        sqlx::Error::PoolClosed | sqlx::Error::PoolTimedOut | sqlx::Error::Io(_) => {
            StorageError::Unavailable
        }
        sqlx::Error::Database(ref database) => {
            let code = database.code().and_then(|code| code.parse::<i32>().ok());
            match code.map(|code| code & 255) {
                Some(5 | 6 | 14) => StorageError::Unavailable,
                Some(19) => StorageError::ConstraintViolation(database.message().to_owned()),
                Some(11 | 26) => StorageError::InvalidData("invalid SQLite database".into()),
                Some(1) if database.message() == "JSON cannot hold BLOB values" => {
                    StorageError::InvalidData("invalid stored recipient value".into())
                }
                _ => StorageError::Backend(Box::new(error)),
            }
        }
        sqlx::Error::ColumnDecode { index, .. } => {
            StorageError::InvalidData(format!("invalid stored column {index}"))
        }
        sqlx::Error::Decode(_) => StorageError::InvalidData("invalid stored value".into()),
        error => StorageError::Backend(Box::new(error)),
    }
}

/// Convert driver results through the backend-neutral error translation.
pub(crate) trait StorageResult<T> {
    /// Map a SQLx error into a semantic storage error.
    fn storage(self) -> Result<T, StorageError>;
}

impl<T> StorageResult<T> for Result<T, sqlx::Error> {
    /// Translate a driver result into the backend-independent storage error contract.
    fn storage(self) -> Result<T, StorageError> {
        self.map_err(sqlite_error)
    }
}
