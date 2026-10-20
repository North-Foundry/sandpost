//! JavaScript argument validation, safe DTO conversion, and public error messages.

use napi::{Error, Result, Status};
use sandpost_application::{ApplicationError, InstanceError};
use sandpost_core::{
    ImapAccountIdentifier, ImapFolderIdentifier, InboxIdentifier, MessageIdentifier,
    MessageSequence, PersonaExecutionIdentifier, PersonaIdentifier, ScopeIdentifier,
    SmtpAccessIdentifier, UserIdentifier, ViewIdentifier,
};
use serde::Serialize;
use serde_json::Value;

/// Validate a JavaScript sequence without silently truncating or losing integer precision.
pub(super) fn sequence_from_number(value: f64) -> Result<MessageSequence> {
    if !value.is_finite()
        || value.fract() != 0.0
        || !(0.0..=9_007_199_254_740_991.0).contains(&value)
    {
        return Err(Error::new(
            Status::InvalidArg,
            "before must be a nonnegative safe integer",
        ));
    }
    Ok(MessageSequence(value as u64))
}

/// Validate page size before converting JavaScript numbers to application arguments.
pub(super) fn page_size_from_number(value: f64) -> Result<usize> {
    if !value.is_finite() || value.fract() != 0.0 || !(1.0..=100.0).contains(&value) {
        return Err(Error::new(
            Status::InvalidArg,
            "limit must be between 1 and 100",
        ));
    }
    Ok(value as usize)
}

/// Validate a JavaScript attachment index before converting it to a platform usize.
pub(super) fn attachment_index_from_number(value: f64) -> Result<usize> {
    if !value.is_finite()
        || value.fract() != 0.0
        || value < 0.0
        || value > 9_007_199_254_740_991.0
        || value > usize::MAX as f64
    {
        return Err(Error::new(
            Status::InvalidArg,
            "attachment index must be a nonnegative safe integer",
        ));
    }
    Ok(value as usize)
}

/// Parse a public UUID at the native trust boundary.
pub(super) fn parse_message_identifier(value: &str) -> Result<MessageIdentifier> {
    uuid::Uuid::parse_str(value)
        .map(MessageIdentifier)
        .map_err(|_| Error::new(Status::InvalidArg, "invalid message identifier"))
}

/// Parse a bounded batch of message UUIDs at the native trust boundary.
pub(super) fn parse_message_identifiers(values: Vec<String>) -> Result<Vec<MessageIdentifier>> {
    if values.len() > 256 {
        return Err(Error::new(
            Status::InvalidArg,
            "at most 256 message identifiers may be moved at once",
        ));
    }
    values
        .iter()
        .map(|value| parse_message_identifier(value))
        .collect()
}

/// Parse a public inbox UUID at the native trust boundary.
pub(super) fn parse_inbox_identifier(value: &str) -> Result<InboxIdentifier> {
    value
        .parse()
        .map_err(|_| Error::new(Status::InvalidArg, "invalid inbox identifier"))
}

/// Parse a saved-view UUID at the native trust boundary.
pub(super) fn parse_view_identifier(value: &str) -> Result<ViewIdentifier> {
    value
        .parse()
        .map_err(|_| Error::new(Status::InvalidArg, "invalid view identifier"))
}

/// Parse a scope UUID at the native trust boundary.
pub(super) fn parse_scope_identifier(value: &str) -> Result<ScopeIdentifier> {
    value
        .parse()
        .map_err(|_| Error::new(Status::InvalidArg, "invalid scope identifier"))
}

/// Parse an account UUID at the native trust boundary.
pub(super) fn parse_user_identifier(value: &str) -> Result<UserIdentifier> {
    value
        .parse()
        .map_err(|_| Error::new(Status::InvalidArg, "invalid user identifier"))
}

/// Parse an SMTP access UUID at the native trust boundary.
pub(super) fn parse_smtp_access_identifier(value: &str) -> Result<SmtpAccessIdentifier> {
    value
        .parse()
        .map_err(|_| Error::new(Status::InvalidArg, "invalid SMTP access identifier"))
}

/// Parse an IMAP account UUID at the native trust boundary.
pub(super) fn parse_imap_account_identifier(value: &str) -> Result<ImapAccountIdentifier> {
    value
        .parse()
        .map_err(|_| Error::new(Status::InvalidArg, "invalid IMAP account identifier"))
}

/// Parse an IMAP-local folder UUID at the native trust boundary.
pub(super) fn parse_imap_folder_identifier(value: &str) -> Result<ImapFolderIdentifier> {
    value
        .parse()
        .map_err(|_| Error::new(Status::InvalidArg, "invalid IMAP folder identifier"))
}

/**
 * Parse a Persona UUID at the native trust boundary.
 */
pub(super) fn parse_persona_identifier(value: &str) -> Result<PersonaIdentifier> {
    value
        .parse()
        .map_err(|_| Error::new(Status::InvalidArg, "invalid persona identifier"))
}

/**
 * Parse a Persona execution UUID at the native trust boundary.
 */
pub(super) fn parse_persona_execution_identifier(
    value: &str,
) -> Result<PersonaExecutionIdentifier> {
    value
        .parse()
        .map_err(|_| Error::new(Status::InvalidArg, "invalid persona execution identifier"))
}

/// Decode a native JSON object into its typed application input.
pub(super) fn deserialize<T: serde::de::DeserializeOwned>(
    value: Value,
    message: &'static str,
) -> Result<T> {
    serde_json::from_value(value).map_err(|_| Error::new(Status::InvalidArg, message))
}

/// Convert an existing application DTO into its unchanged serde wire representation.
pub(super) fn serialize(value: impl Serialize) -> Result<Value> {
    let mut value = serde_json::to_value(value).map_err(native_error)?;
    normalize_javascript_numbers(&mut value)?;
    Ok(value)
}

/// Keep serde integers as JS Numbers instead of napi-rs BigInts, matching HTTP DTOs.
/// Reject integers that cannot be represented safely rather than losing cursor precision.
fn normalize_javascript_numbers(value: &mut Value) -> Result<()> {
    match value {
        Value::Number(number) => {
            let value = number
                .as_f64()
                .ok_or_else(|| native_error("invalid wire number"))?;
            if (number.is_i64() || number.is_u64()) && value.abs() > 9_007_199_254_740_991.0 {
                return Err(native_error(
                    "wire integer exceeds JavaScript safe integer range",
                ));
            }
            *number = serde_json::Number::from_f64(value)
                .ok_or_else(|| native_error("invalid wire number"))?;
        }
        Value::Array(values) => {
            for value in values {
                normalize_javascript_numbers(value)?;
            }
        }
        Value::Object(values) => {
            for value in values.values_mut() {
                normalize_javascript_numbers(value)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Map application errors without exposing database paths or internal worker details.
pub(super) fn application_error(error: ApplicationError) -> Error {
    match error {
        ApplicationError::Forbidden => Error::new(Status::GenericFailure, "forbidden"),
        ApplicationError::Unavailable(_) => {
            Error::new(Status::GenericFailure, "service unavailable")
        }
        ApplicationError::Search(_) => {
            Error::new(Status::GenericFailure, "search operation failed")
        }
        ApplicationError::Io(_) => Error::new(Status::GenericFailure, "operation failed"),
        ApplicationError::Instance(error) => instance_error(error),
        ApplicationError::InvalidPage => {
            Error::new(Status::InvalidArg, "limit must be between 1 and 100")
        }
        ApplicationError::InvalidInput(message) => Error::new(Status::InvalidArg, message),
        ApplicationError::Conflict => Error::new(Status::GenericFailure, "conflict"),
        ApplicationError::InUse(message) => Error::new(Status::GenericFailure, message),
        ApplicationError::NotFound => Error::new(Status::GenericFailure, "message not found"),
        ApplicationError::Storage(_) | ApplicationError::Worker(_) => {
            Error::new(Status::GenericFailure, "storage operation failed")
        }
    }
}

/// Map instance failures to safe renderer-facing messages.
pub(super) fn instance_error(error: InstanceError) -> Error {
    match error {
        InstanceError::Authentication => {
            Error::new(Status::GenericFailure, "authentication required")
        }
        InstanceError::SetupRequired => Error::new(Status::GenericFailure, "setup required"),
        InstanceError::AlreadyInitialized => {
            Error::new(Status::GenericFailure, "already initialized")
        }
        InstanceError::InvalidInput(message) => Error::new(Status::InvalidArg, message),
        InstanceError::RecoveryRequired => {
            Error::new(Status::GenericFailure, "instance recovery required")
        }
        InstanceError::WriteMetadata(_) => Error::new(
            Status::GenericFailure,
            "instance metadata could not be written",
        ),
        InstanceError::CreateDirectory(_) => Error::new(
            Status::GenericFailure,
            "instance data directory could not be created",
        ),
        InstanceError::Storage(_) => Error::new(Status::GenericFailure, "storage operation failed"),
        InstanceError::Worker(_) => Error::new(Status::GenericFailure, "storage operation failed"),
        InstanceError::Cryptography(_) => {
            Error::new(Status::GenericFailure, "cryptographic operation failed")
        }
    }
}

/// Return the safe native error used when no process session is available.
pub(super) fn authentication_error() -> Error {
    Error::new(Status::GenericFailure, "authentication required")
}

/// Preserve operational startup and shutdown errors for the trusted native host.
pub(super) fn native_error(error: impl std::fmt::Display) -> Error {
    Error::new(Status::GenericFailure, error.to_string())
}
