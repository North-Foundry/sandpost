//! Recipient inbox compatibility API backed by saved Views and the virtual default Inbox.

use crate::{
    ApplicationChange, ApplicationError, AuthorizedApplication, MessageSummary, QueryOptions,
    validated_name,
};
use sandpost_core::{
    InboxIdentifier, MessageSequence, View, ViewIdentifier, normalize_inbox_address,
};
use sandpost_storage::StorageError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Compatibility recipient inbox backed by a saved View.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxDetails {
    pub id: Uuid,
    pub name: String,
    pub address: Option<String>,
    pub system: bool,
}

impl AuthorizedApplication {
    /// Apply an owned or shared saved View through the legacy inbox identity.
    pub async fn list_messages_in_inbox(
        &self,
        inbox_identifier: Option<InboxIdentifier>,
        before: Option<MessageSequence>,
        limit: usize,
    ) -> Result<Vec<MessageSummary>, ApplicationError> {
        self.query_web_mailbox(
            QueryOptions {
                view_identifier: inbox_identifier
                    .filter(|identifier| !identifier.0.is_nil())
                    .map(|identifier| ViewIdentifier(identifier.0)),
                before,
                limit,
                ..QueryOptions::default()
            },
            None,
        )
        .await
    }
    /// Adapt recipient Views and virtual all-visible mail for the existing inspector.
    pub async fn list_inboxes(&self) -> Result<Vec<InboxDetails>, ApplicationError> {
        let views = self.list_views().await?;
        let mut result = vec![InboxDetails {
            id: Uuid::nil(),
            name: "Default Inbox".into(),
            address: None,
            system: true,
        }];
        result.extend(views.into_iter().filter_map(inbox_details));
        Ok(result)
    }
    /// Store a personal recipient View without copying or reindexing messages.
    pub async fn create_inbox(
        &self,
        name: String,
        address: String,
    ) -> Result<InboxDetails, ApplicationError> {
        let name = validated_name(&name)?;
        let address = validated_inbox_address(&address)?;
        self.account().await?;
        let view = self
            .application
            .storage
            .create_recipient_view(self.user_identifier, &name, &address)
            .await
            .map_err(|error| match error {
                StorageError::DuplicateInboxAddress => ApplicationError::Conflict,
                error => ApplicationError::Storage(error),
            })?;
        self.notify(ApplicationChange::Visibility);
        inbox_details(view).ok_or(ApplicationError::NotFound)
    }
}

/// Normalize a mailbox accepted by the legacy recipient-inbox API.
fn validated_inbox_address(address: &str) -> Result<String, ApplicationError> {
    normalize_inbox_address(address).ok_or_else(invalid_inbox_address)
}
/// Return the fixed validation error for unsupported inbox addresses.
fn invalid_inbox_address() -> ApplicationError {
    ApplicationError::InvalidInput(
        "address must be a valid ASCII email address with a 1 to 64 character dot-atom local part"
            .into(),
    )
}
/// Convert exact recipient Views while leaving richer queries to the Views API.
fn inbox_details(view: View) -> Option<InboxDetails> {
    let address = view
        .filter
        .strip_prefix("envelope.to.address == \"")?
        .strip_suffix('"')?
        .to_owned();
    Some(InboxDetails {
        id: view.identifier.0,
        name: view.name,
        address: Some(address),
        system: false,
    })
}
