//! Adapters for the authenticated user's own IMAP accounts and their IMAP-local folders.
//!
//! Every operation acts only on accounts the native session's user owns; the application reports
//! another user's account as not found. Only account creation and password regeneration return a
//! password, and inputs never carry one.

use crate::NativeRuntime;
use crate::boundary::{
    application_error, deserialize, page_size_from_number, parse_imap_account_identifier,
    parse_imap_folder_identifier, sequence_from_number, serialize,
};
use napi::{Error, Result, Status};
use napi_derive::napi;
use sandpost_application::{
    ApplicationError, ImapAccountUpdate, ImapFolderRequest, NewImapAccountRequest, WebFolderRequest,
};
use serde_json::{Value, json};

/// Map account failures, naming the missing record instead of the generic message error.
fn imap_account_error(error: ApplicationError) -> Error {
    match error {
        ApplicationError::NotFound => Error::new(Status::GenericFailure, "IMAP account not found"),
        error => application_error(error),
    }
}

/// Map local-folder failures; the account or the folder may be the missing record.
fn imap_folder_error(error: ApplicationError) -> Error {
    match error {
        ApplicationError::NotFound => {
            Error::new(Status::GenericFailure, "IMAP account or folder not found")
        }
        error => application_error(error),
    }
}

/// Map manual-folder failures without referring to IMAP account projections.
fn web_folder_error(error: ApplicationError) -> Error {
    match error {
        ApplicationError::NotFound => Error::new(Status::GenericFailure, "folder not found"),
        error => application_error(error),
    }
}

#[napi]
impl NativeRuntime {
    /// Describe how mail clients reach the IMAP service, or report that it is disabled.
    #[napi]
    pub async fn imap_server_get(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .imap_server()
                .await
                .map_err(application_error)?,
        )
    }

    /// List the session user's IMAP accounts with their mailboxes, never with passwords.
    #[napi]
    pub async fn imap_accounts_list(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .list_imap_accounts()
                .await
                .map_err(imap_account_error)?,
        )
    }

    /// Load one of the session user's IMAP accounts after validating its identifier.
    #[napi]
    pub async fn imap_account_get(&self, identifier: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .get_imap_account(parse_imap_account_identifier(&identifier)?)
                .await
                .map_err(imap_account_error)?,
        )
    }

    /// Create an IMAP account; the generated password is returned only by this call.
    #[napi]
    pub async fn imap_account_create(&self, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: NewImapAccountRequest = deserialize(input, "invalid IMAP account")?;
        serialize(
            self.application
                .authorized(&user)
                .create_imap_account(request)
                .await
                .map_err(imap_account_error)?,
        )
    }

    /// Change an IMAP account's name, status, or mailbox selection; omitted fields are kept.
    #[napi]
    pub async fn imap_account_update(&self, identifier: String, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let update: ImapAccountUpdate = deserialize(input, "invalid IMAP account update")?;
        serialize(
            self.application
                .authorized(&user)
                .update_imap_account(parse_imap_account_identifier(&identifier)?, update)
                .await
                .map_err(imap_account_error)?,
        )
    }

    /// Delete an IMAP account and its IMAP state; captured mail is untouched.
    #[napi]
    pub async fn imap_account_delete(&self, identifier: String) -> Result<()> {
        let user = self.ensure_authenticated().await?;
        self.application
            .authorized(&user)
            .delete_imap_account(parse_imap_account_identifier(&identifier)?)
            .await
            .map_err(imap_account_error)
    }

    /// Regenerate an IMAP account password; the new one is returned only by this call and the
    /// previous one, with every session it opened, stops working immediately.
    #[napi]
    pub async fn imap_account_regenerate(&self, identifier: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .regenerate_imap_account_password(parse_imap_account_identifier(&identifier)?)
                .await
                .map_err(imap_account_error)?,
        )
    }

    /// Restore mailbox MOVE exclusions, preserving user deletions, reported as `{ restored }` like
    /// the HTTP API.
    #[napi]
    pub async fn imap_account_restore(&self, identifier: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let restored = self
            .application
            .authorized(&user)
            .restore_imap_hidden_messages(parse_imap_account_identifier(&identifier)?)
            .await
            .map_err(imap_account_error)?;
        serialize(json!({ "restored": restored }))
    }

    /// Create an IMAP-local folder whose canonical query the application validates.
    #[napi]
    pub async fn imap_folder_create(&self, account: String, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: ImapFolderRequest = deserialize(input, "invalid IMAP folder")?;
        serialize(
            self.application
                .authorized(&user)
                .create_imap_folder(parse_imap_account_identifier(&account)?, request)
                .await
                .map_err(imap_folder_error)?,
        )
    }

    /// Replace an IMAP-local folder's name, query, and INBOX participation.
    #[napi]
    pub async fn imap_folder_update(
        &self,
        account: String,
        folder: String,
        input: Value,
    ) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: ImapFolderRequest = deserialize(input, "invalid IMAP folder")?;
        serialize(
            self.application
                .authorized(&user)
                .update_imap_folder(
                    parse_imap_account_identifier(&account)?,
                    parse_imap_folder_identifier(&folder)?,
                    request,
                )
                .await
                .map_err(imap_folder_error)?,
        )
    }

    /// Delete an IMAP-local folder; Sandpost Views and captured mail are untouched.
    #[napi]
    pub async fn imap_folder_delete(&self, account: String, folder: String) -> Result<()> {
        let user = self.ensure_authenticated().await?;
        self.application
            .authorized(&user)
            .delete_imap_folder(
                parse_imap_account_identifier(&account)?,
                parse_imap_folder_identifier(&folder)?,
            )
            .await
            .map_err(imap_folder_error)
    }

    /// List the signed-in user's Web-managed manual folders.
    #[napi]
    pub async fn folders_list(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .list_web_folders()
                .await
                .map_err(web_folder_error)?,
        )
    }

    /// Create a Web-managed manual folder from camelCase JSON and return its native DTO shape.
    #[napi]
    pub async fn folder_create(&self, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: WebFolderRequest = deserialize(input, "invalid folder")?;
        serialize(
            self.application
                .authorized(&user)
                .create_web_folder(request)
                .await
                .map_err(web_folder_error)?,
        )
    }

    /// Rename a Web-managed manual folder or replace its exposed IMAP accounts.
    #[napi]
    pub async fn folder_update(&self, identifier: String, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: WebFolderRequest = deserialize(input, "invalid folder")?;
        serialize(
            self.application
                .authorized(&user)
                .update_web_folder(parse_imap_folder_identifier(&identifier)?, request)
                .await
                .map_err(web_folder_error)?,
        )
    }

    /// Delete an owned Web-managed folder without deleting its messages.
    #[napi]
    pub async fn folder_delete(&self, identifier: String) -> Result<()> {
        let user = self.ensure_authenticated().await?;
        self.application
            .authorized(&user)
            .delete_web_folder(parse_imap_folder_identifier(&identifier)?)
            .await
            .map_err(web_folder_error)
    }

    /// Query a Web folder with the native keyset and canonical-query bounds.
    #[napi]
    pub async fn folder_messages(
        &self,
        identifier: String,
        before: Option<f64>,
        limit: Option<f64>,
        query: Option<String>,
    ) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .query_web_mailbox(
                    sandpost_application::QueryOptions {
                        view_identifier: None,
                        query,
                        before: before.map(sequence_from_number).transpose()?,
                        limit: page_size_from_number(limit.unwrap_or(50.0))?,
                    },
                    Some(parse_imap_folder_identifier(&identifier)?),
                )
                .await
                .map_err(web_folder_error)?,
        )
    }
}
