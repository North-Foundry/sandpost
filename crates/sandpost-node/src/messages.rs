//! Authorized mail listing, literal search, inboxes, MIME, attachments, and deletion.

use crate::NativeRuntime;
use crate::boundary::{
    application_error, attachment_index_from_number, deserialize, page_size_from_number,
    parse_imap_folder_identifier, parse_inbox_identifier, parse_message_identifier,
    parse_message_identifiers, parse_view_identifier, sequence_from_number, serialize,
};
use napi::{Error, Result, Status, bindgen_prelude::Buffer};
use napi_derive::napi;
use sandpost_application::{BulkLinkCheckRequest, QueryOptions};
use sandpost_core::ViewIdentifier;
use serde::Deserialize;
use serde_json::{Value, json};

/// JSON request for moving messages in the Web mailbox.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MoveMessagesRequest {
    identifiers: Vec<String>,
    folder_identifier: Option<String>,
}

/// JSON request for changing primary read status.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SeenMessagesRequest {
    identifiers: Vec<String>,
    seen: bool,
}

/// JSON request for checking the links of several messages.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BulkLinkCheckInput {
    identifiers: Vec<String>,
}

/// JSON request for moving messages to the Web Trash.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct TrashMessagesRequest {
    identifiers: Vec<String>,
}

#[napi]
impl NativeRuntime {
    /// List stored messages through the same application operation used by HTTP.
    #[napi]
    pub async fn messages_list(
        &self,
        before: Option<f64>,
        limit: Option<f64>,
        inbox_identifier: Option<String>,
        view_identifier: Option<String>,
        query: Option<String>,
    ) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let before = before.map(sequence_from_number).transpose()?;
        let limit = page_size_from_number(limit.unwrap_or(50.0))?;
        let inbox_identifier = inbox_identifier
            .as_deref()
            .map(parse_inbox_identifier)
            .transpose()?;
        let view_identifier = view_identifier
            .as_deref()
            .map(parse_view_identifier)
            .transpose()?;
        let inbox_view = inbox_identifier
            .filter(|identifier| !identifier.0.is_nil())
            .map(|identifier| ViewIdentifier(identifier.0));
        if view_identifier.is_some() && inbox_view.is_some() && view_identifier != inbox_view {
            return Err(Error::new(
                Status::InvalidArg,
                "view identifier and inbox identifier disagree",
            ));
        }
        let view_identifier = view_identifier.or(inbox_view);
        let authorized = self.application.authorized(&user);
        let results = if view_identifier.is_some() || query.is_some() {
            let options = QueryOptions {
                view_identifier,
                query,
                before,
                limit,
            };
            if view_identifier.is_none() {
                authorized
                    .query_messages(options)
                    .await
                    .map_err(application_error)?
            } else {
                authorized
                    .query_web_mailbox(options, None)
                    .await
                    .map_err(application_error)?
            }
        } else {
            match inbox_identifier {
                Some(identifier) => authorized
                    .list_messages_in_inbox(Some(identifier), before, limit)
                    .await
                    .map_err(application_error)?,
                None => authorized
                    .list_messages(before, limit)
                    .await
                    .map_err(application_error)?,
            }
        };
        serialize(results)
    }

    /// List available system and user-defined inboxes as serde-compatible values.
    #[napi]
    pub async fn inboxes_list(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .list_inboxes()
                .await
                .map_err(application_error)?,
        )
    }

    /// Create an inbox from its name and email address.
    #[napi]
    pub async fn inbox_create(&self, name: String, address: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .create_inbox(name, address)
                .await
                .map_err(application_error)?,
        )
    }

    /// Search all messages and preserve the shared summary wire representation.
    #[napi]
    pub async fn messages_search(
        &self,
        query: String,
        before: Option<f64>,
        limit: Option<f64>,
    ) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .search_messages(
                    query,
                    before.map(sequence_from_number).transpose()?,
                    page_size_from_number(limit.unwrap_or(50.0))?,
                )
                .await
                .map_err(application_error)?,
        )
    }

    /// Transfer attachment bytes as a Node Buffer.
    #[napi]
    pub async fn message_attachment(&self, identifier: String, index: f64) -> Result<Buffer> {
        let user = self.ensure_authenticated().await?;
        Ok(self
            .application
            .authorized(&user)
            .get_attachment(
                parse_message_identifier(&identifier)?,
                attachment_index_from_number(index)?,
            )
            .await
            .map_err(application_error)?
            .into())
    }

    /// Delete a stored message and resolve to JavaScript undefined.
    #[napi]
    pub async fn message_delete(&self, identifier: String) -> Result<()> {
        let user = self.ensure_authenticated().await?;
        self.application
            .authorized(&user)
            .delete_message(parse_message_identifier(&identifier)?)
            .await
            .map_err(application_error)
    }

    /// Delete every message the signed-in user can read, reported as `{ deleted }` like
    /// `DELETE /api/v1/messages`.
    #[napi]
    pub async fn messages_delete_all(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let deleted = self
            .application
            .authorized(&user)
            .delete_all_messages()
            .await
            .map_err(application_error)?;
        serialize(json!({ "deleted": deleted }))
    }

    /// Move a bounded batch into an owned Web folder or back to its unfiled baseline.
    #[napi]
    pub async fn messages_move(&self, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: MoveMessagesRequest = deserialize(input, "invalid message move")?;
        let identifiers = parse_message_identifiers(request.identifiers)?;
        let folder = request
            .folder_identifier
            .as_deref()
            .map(parse_imap_folder_identifier)
            .transpose()?;
        let moved = self
            .application
            .authorized(&user)
            .move_web_messages(identifiers, folder)
            .await
            .map_err(application_error)?;
        serialize(json!({ "moved": moved }))
    }

    /// Copy primary messages into an owned folder or Inbox while preserving the source.
    #[napi]
    pub async fn messages_copy(&self, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: MoveMessagesRequest = deserialize(input, "invalid message copy")?;
        let folder = request
            .folder_identifier
            .as_deref()
            .map(parse_imap_folder_identifier)
            .transpose()?;
        let copied = self
            .application
            .authorized(&user)
            .copy_web_messages(parse_message_identifiers(request.identifiers)?, folder)
            .await
            .map_err(application_error)?;
        serialize(json!({ "copied": copied }))
    }

    /// Change supported primary flags, shared with every Mirror credential.
    #[napi]
    pub async fn messages_set_flags(&self, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: sandpost_application::WebMessageFlagsRequest =
            deserialize(input, "invalid message flags")?;
        let changed = self
            .application
            .authorized(&user)
            .set_web_messages_flags(
                request.identifiers,
                sandpost_storage::ImapFlagChange {
                    operation: request.operation,
                    flags: request.flags,
                },
            )
            .await
            .map_err(application_error)?;
        serialize(json!({ "changed": changed }))
    }

    /// Move a bounded batch to the Web Trash, reported as `{ trashed }` like
    /// `POST /api/v1/messages/trash`.
    #[napi]
    pub async fn messages_trash(&self, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: TrashMessagesRequest = deserialize(input, "invalid message trash")?;
        let identifiers = parse_message_identifiers(request.identifiers)?;
        let trashed = self
            .application
            .authorized(&user)
            .trash_web_messages(identifiers)
            .await
            .map_err(application_error)?;
        serialize(json!({ "trashed": trashed }))
    }

    /// Change primary read status, shared with every Mirror credential.
    #[napi]
    pub async fn messages_set_seen(&self, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: SeenMessagesRequest = deserialize(input, "invalid message read status")?;
        let changed = self
            .application
            .authorized(&user)
            .set_web_messages_seen(
                parse_message_identifiers(request.identifiers)?,
                request.seen,
            )
            .await
            .map_err(application_error)?;
        serialize(json!({ "changed": changed }))
    }

    /// Query the Web Trash with the native keyset and canonical-query bounds.
    #[napi]
    pub async fn trash_messages(
        &self,
        before: Option<f64>,
        limit: Option<f64>,
        query: Option<String>,
    ) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .query_web_trash(QueryOptions {
                    view_identifier: None,
                    query,
                    before: before.map(sequence_from_number).transpose()?,
                    limit: page_size_from_number(limit.unwrap_or(50.0))?,
                })
                .await
                .map_err(application_error)?,
        )
    }

    /// Permanently delete the Web Trash, reported as `{ deleted }` like `DELETE /api/v1/trash`.
    #[napi]
    pub async fn trash_empty(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let deleted = self
            .application
            .authorized(&user)
            .empty_web_trash()
            .await
            .map_err(application_error)?;
        serialize(json!({ "deleted": deleted }))
    }

    /// Read the serde-compatible message detail envelope from the application.
    #[napi]
    pub async fn message_get(&self, identifier: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .get_message(parse_message_identifier(&identifier)?)
                .await
                .map_err(application_error)?,
        )
    }

    /**
     * Check the original HTML of a message against email client support data.
     */
    #[napi]
    pub async fn message_html_check(&self, identifier: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .check_message_html(parse_message_identifier(&identifier)?)
                .await
                .map_err(application_error)?,
        )
    }

    /**
     * Read a message's stored and running link check state without starting a check.
     */
    #[napi]
    pub async fn message_link_check(&self, identifier: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .get_link_check(parse_message_identifier(&identifier)?)
                .await
                .map_err(application_error)?,
        )
    }

    /**
     * Start, or join, a background check of a message's links, like
     * `POST /api/v1/messages/<id>/link-check`.
     */
    #[napi]
    pub async fn message_link_check_start(&self, identifier: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .start_link_check(parse_message_identifier(&identifier)?)
                .await
                .map_err(application_error)?,
        )
    }

    /**
     * Start background link checks of several messages, reported as
     * `{ started, already_running, skipped }` like `POST /api/v1/messages/link-check`.
     */
    #[napi]
    pub async fn messages_link_check(&self, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: BulkLinkCheckInput = deserialize(input, "invalid bulk link check")?;
        let identifiers = parse_message_identifiers(request.identifiers)?;
        serialize(
            self.application
                .authorized(&user)
                .start_bulk_link_checks(BulkLinkCheckRequest { identifiers })
                .await
                .map_err(application_error)?,
        )
    }

    /// Transfer original MIME as a Node Buffer rather than a JSON byte array.
    #[napi]
    pub async fn message_raw(&self, identifier: String) -> Result<Buffer> {
        let user = self.ensure_authenticated().await?;
        Ok(self
            .application
            .authorized(&user)
            .get_raw_message(parse_message_identifier(&identifier)?)
            .await
            .map_err(application_error)?
            .into())
    }
}
