//! Shared decoding of backend-neutral identifiers and invalid stored columns.
use crate::StorageError;

/// Parse a stored identifier and preserve the invalid source value in the error.
pub(crate) fn parse_identifier<IdentifierType: std::str::FromStr>(
    value: &str,
) -> Result<IdentifierType, StorageError> {
    value
        .parse()
        .map_err(|_| StorageError::InvalidData(value.to_owned()))
}

/// Build a rusqlite decode failure for an invalid stored column value.
pub(crate) fn invalid_column(index: usize, name: &str) -> rusqlite::Error {
    rusqlite::Error::InvalidColumnType(index, name.to_owned(), rusqlite::types::Type::Text)
}
