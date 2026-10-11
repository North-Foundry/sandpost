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

/// Build a driver decode failure for an invalid stored column without exposing its value.
pub(crate) fn invalid_column(name: &str) -> sqlx::Error {
    sqlx::Error::ColumnDecode {
        index: name.to_owned(),
        source: Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid {name}"),
        )),
    }
}
