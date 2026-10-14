//! Shared Web message read status.

use crate::{
    ApplicationChange, ApplicationError, AuthorizedApplication, QueryOptions, authorization,
};
use sandpost_core::{ImapMessageFlags, MessageIdentifier};
use sandpost_storage::{ImapFlagChange, ImapFlagOperation, StorageError};
use serde::{Deserialize, Deserializer};

/// Shared HTTP/native input for supported primary flag mutations.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebMessageFlagsRequest {
    pub identifiers: Vec<MessageIdentifier>,
    pub operation: ImapFlagOperation,
    #[serde(deserialize_with = "deserialize_flags")]
    pub flags: ImapMessageFlags,
}

impl WebMessageFlagsRequest {
    /// Split the transport request into the application identifiers and flag change.
    pub fn into_parts(self) -> (Vec<MessageIdentifier>, ImapFlagChange) {
        (
            self.identifiers,
            ImapFlagChange {
                operation: self.operation,
                flags: self.flags,
            },
        )
    }
}

/// Decode a partial flag object with false booleans and no keywords for omitted fields.
fn deserialize_flags<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<ImapMessageFlags, D::Error> {
    #[derive(Default, Deserialize)]
    #[serde(default, deny_unknown_fields)]
    struct Flags {
        seen: bool,
        answered: bool,
        flagged: bool,
        draft: bool,
        deleted: bool,
        keywords: Vec<String>,
    }
    let flags = Flags::deserialize(deserializer)?;
    Ok(ImapMessageFlags {
        seen: flags.seen,
        answered: flags.answered,
        flagged: flags.flagged,
        draft: flags.draft,
        deleted: flags.deleted,
        keywords: flags.keywords,
    })
}

impl AuthorizedApplication {
    /// Change supported primary flags for currently authorized messages and notify every Mirror.
    pub async fn set_web_messages_flags(
        &self,
        identifiers: Vec<MessageIdentifier>,
        change: ImapFlagChange,
    ) -> Result<u64, ApplicationError> {
        if identifiers.len() > 256 {
            return Err(ApplicationError::InvalidInput(
                "a flag update accepts at most 256 messages".into(),
            ));
        }
        let query = authorization::compose(
            &self.application.storage,
            self.user_identifier,
            &QueryOptions::default(),
            None,
        )
        .await?;
        let changed = self
            .application
            .storage
            .set_user_message_flags(
                self.user_identifier,
                &identifiers,
                &change,
                &authorization::storage_filter(&query),
            )
            .await
            .map_err(|error| match error {
                StorageError::ConstraintViolation(message) => {
                    ApplicationError::InvalidInput(message)
                }
                StorageError::NotFound => ApplicationError::NotFound,
                error => error.into(),
            })?;
        if changed > 0 {
            self.notify(ApplicationChange::Visibility);
        }
        Ok(changed)
    }

    /// Set shared Seen on up to 256 messages the caller can currently read.
    pub async fn set_web_messages_seen(
        &self,
        identifiers: Vec<MessageIdentifier>,
        seen: bool,
    ) -> Result<u64, ApplicationError> {
        if identifiers.len() > 256 {
            return Err(ApplicationError::InvalidInput(
                "a Seen update accepts at most 256 messages".into(),
            ));
        }
        let query = authorization::compose(
            &self.application.storage,
            self.user_identifier,
            &QueryOptions::default(),
            None,
        )
        .await?;
        let updated = self
            .application
            .storage
            .set_user_message_seen(
                self.user_identifier,
                &identifiers,
                seen,
                &authorization::storage_filter(&query),
            )
            .await?;
        if updated > 0 {
            self.notify(ApplicationChange::Visibility);
        }
        Ok(updated)
    }
}
