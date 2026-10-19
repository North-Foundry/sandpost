//! User-owned Web manual folders, Trash, and organization independent of IMAP placement.

use crate::{
    ApplicationChange, ApplicationError, AuthorizedApplication, MessageSummary, QueryOptions,
    authorization, credentials::current_unix_seconds, validate_icon, validate_page_size,
    validated_name,
};
use sandpost_core::{
    GlobalRole, ImapAccountIdentifier, ImapFolderIdentifier, MessageIdentifier, WebFolder,
};
use sandpost_storage::{MessageListQuery, StorageError};
use serde::Deserialize;

/// A manual folder definition and its explicitly selected IMAP exposures.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WebFolderRequest {
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub exposed_accounts: Vec<ImapAccountIdentifier>,
}

impl AuthorizedApplication {
    /// List manual folders belonging only to the current principal.
    pub async fn list_web_folders(&self) -> Result<Vec<WebFolder>, ApplicationError> {
        self.account().await?;
        Ok(self
            .application
            .storage
            .list_web_folders(self.user_identifier)
            .await?)
    }

    /// Load a manual folder without revealing another user's folder even to administrators.
    async fn owned_web_folder(
        &self,
        identifier: ImapFolderIdentifier,
    ) -> Result<WebFolder, ApplicationError> {
        self.account().await?;
        self.application
            .storage
            .get_web_folder(identifier)
            .await?
            .filter(|folder| folder.owner_identifier == self.user_identifier)
            .ok_or(ApplicationError::NotFound)
    }

    /// Validate the display name and bound exposure input before persisting a definition.
    pub(crate) fn validate_web_folder(
        request: WebFolderRequest,
    ) -> Result<WebFolderRequest, ApplicationError> {
        if request.exposed_accounts.len() > 25 {
            return Err(ApplicationError::InvalidInput(
                "a folder can expose at most 25 IMAP accounts".into(),
            ));
        }
        let name = validated_name(&request.name)?;
        validate_icon(&request.icon)?;
        if name.split('/').any(|segment| segment.is_empty()) {
            return Err(ApplicationError::InvalidInput(
                "normal folder paths require nonempty hierarchy segments".into(),
            ));
        }
        let mut accounts = request.exposed_accounts;
        accounts.sort_unstable();
        accounts.dedup();
        Ok(WebFolderRequest {
            name,
            icon: request.icon,
            exposed_accounts: accounts,
        })
    }

    /// Map folder-name uniqueness to the same conflict used by existing folder management.
    fn web_folder_error(error: StorageError) -> ApplicationError {
        match error {
            StorageError::DuplicateImapFolderName => ApplicationError::Conflict,
            StorageError::NotFound => ApplicationError::NotFound,
            error => error.into(),
        }
    }

    /// Create a manual Web folder, exposing it only through selected owned accounts.
    pub async fn create_web_folder(
        &self,
        request: WebFolderRequest,
    ) -> Result<WebFolder, ApplicationError> {
        self.require_mail_mutation().await?;
        self.account().await?;
        if self
            .application
            .storage
            .list_web_folders(self.user_identifier)
            .await?
            .len()
            >= 100
        {
            return Err(ApplicationError::InvalidInput(
                "a user can have at most 100 manual folders".into(),
            ));
        }
        let request = Self::validate_web_folder(request)?;
        let folder = self
            .application
            .storage
            .create_web_folder(
                self.user_identifier,
                &request.name,
                request.icon.as_deref(),
                &request.exposed_accounts,
                current_unix_seconds(),
            )
            .await
            .map_err(Self::web_folder_error)?;
        self.notify(ApplicationChange::Visibility);
        Ok(folder)
    }

    /// Rename a manual Web folder or replace its selected exposures, retaining stable identities.
    pub async fn update_web_folder(
        &self,
        identifier: ImapFolderIdentifier,
        request: WebFolderRequest,
    ) -> Result<WebFolder, ApplicationError> {
        self.require_mail_mutation().await?;
        self.owned_web_folder(identifier).await?;
        let request = Self::validate_web_folder(request)?;
        let folder = self
            .application
            .storage
            .update_web_folder(
                identifier,
                &request.name,
                request.icon.as_deref(),
                &request.exposed_accounts,
            )
            .await
            .map_err(Self::web_folder_error)?
            .ok_or(ApplicationError::NotFound)?;
        self.notify(ApplicationChange::Visibility);
        Ok(folder)
    }

    /// Remove a manual folder and its placement/exposure state without deleting any mail.
    pub async fn delete_web_folder(
        &self,
        identifier: ImapFolderIdentifier,
    ) -> Result<(), ApplicationError> {
        let owner = self.account().await?;
        authorization::require_action(owner.global_role != GlobalRole::Viewer)?;
        self.owned_web_folder(identifier).await?;
        if !self
            .application
            .storage
            .delete_web_folder(identifier)
            .await?
        {
            return Err(ApplicationError::NotFound);
        }
        self.notify(ApplicationChange::Visibility);
        Ok(())
    }

    /// Move visible mail in the Web organization; None restores its filtered/general baseline.
    ///
    /// Moving a trashed message restores it from the Web Trash.
    pub async fn move_web_messages(
        &self,
        identifiers: Vec<MessageIdentifier>,
        folder: Option<ImapFolderIdentifier>,
    ) -> Result<u64, ApplicationError> {
        self.require_mail_mutation().await?;
        if identifiers.len() > 256 {
            return Err(ApplicationError::InvalidInput(
                "a move accepts at most 256 messages".into(),
            ));
        }
        if let Some(identifier) = folder {
            self.owned_web_folder(identifier).await?;
        }
        let query = authorization::compose(
            &self.application.storage,
            self.user_identifier,
            &QueryOptions::default(),
            None,
        )
        .await?;
        let moved = self
            .application
            .storage
            .place_user_messages(
                self.user_identifier,
                &identifiers,
                folder,
                &authorization::storage_filter(&query),
            )
            .await?;
        if moved > 0 {
            self.notify(ApplicationChange::Visibility);
        }
        Ok(moved)
    }

    /// Copy authorized primary mail into an owned folder or Inbox, preserving source occurrences.
    pub async fn copy_web_messages(
        &self,
        identifiers: Vec<MessageIdentifier>,
        folder: Option<ImapFolderIdentifier>,
    ) -> Result<u64, ApplicationError> {
        self.require_mail_mutation().await?;
        if identifiers.len() > 256 {
            return Err(ApplicationError::InvalidInput(
                "a copy accepts at most 256 messages".into(),
            ));
        }
        if let Some(identifier) = folder {
            self.owned_web_folder(identifier).await?;
        }
        let query = authorization::compose(
            &self.application.storage,
            self.user_identifier,
            &QueryOptions::default(),
            None,
        )
        .await?;
        let copied = self
            .application
            .storage
            .copy_user_messages(
                self.user_identifier,
                &identifiers,
                folder,
                &authorization::storage_filter(&query),
            )
            .await
            .map_err(|error| match error {
                StorageError::ConstraintViolation(_) => ApplicationError::Conflict,
                StorageError::NotFound => ApplicationError::NotFound,
                StorageError::BatchLimitExceeded(_) => {
                    ApplicationError::InvalidInput("a copy accepts at most 256 messages".into())
                }
                error => error.into(),
            })?;
        if copied > 0 {
            self.notify(ApplicationChange::Visibility);
        }
        Ok(copied)
    }

    /// Query the manual folder or unfiled Web baseline against authoritative facts with keysets.
    pub async fn query_web_mailbox(
        &self,
        options: QueryOptions,
        folder: Option<ImapFolderIdentifier>,
    ) -> Result<Vec<MessageSummary>, ApplicationError> {
        validate_page_size(options.limit)?;
        if let Some(identifier) = folder {
            self.owned_web_folder(identifier).await?;
        }
        let query = authorization::compose(
            &self.application.storage,
            self.user_identifier,
            &options,
            None,
        )
        .await?;
        let listing = MessageListQuery {
            filter: Some(authorization::storage_filter(&query)),
            before: options.before,
            limit: options.limit,
        };
        if options.view_identifier.is_some() {
            Ok(self
                .application
                .storage
                .list_user_messages(self.user_identifier, listing)
                .await?)
        } else {
            Ok(self
                .application
                .storage
                .list_web_mailbox_messages(self.user_identifier, folder, listing)
                .await?)
        }
    }

    /// Move visible mail to the user's Web Trash, out of every Web mailbox and search.
    ///
    /// Like deletion this is refused to Viewers; unlike deletion it is undone by moving the mail.
    pub async fn trash_web_messages(
        &self,
        identifiers: Vec<MessageIdentifier>,
    ) -> Result<u64, ApplicationError> {
        if identifiers.len() > 256 {
            return Err(ApplicationError::InvalidInput(
                "a Trash request accepts at most 256 messages".into(),
            ));
        }
        let account = self.account().await?;
        authorization::require_action(account.global_role != GlobalRole::Viewer)?;
        let query = authorization::compose(
            &self.application.storage,
            self.user_identifier,
            &QueryOptions::default(),
            None,
        )
        .await?;
        let trashed = self
            .application
            .storage
            .trash_user_messages(
                self.user_identifier,
                &identifiers,
                &authorization::storage_filter(&query),
                current_unix_seconds(),
            )
            .await?;
        if trashed > 0 {
            self.notify(ApplicationChange::Visibility);
        }
        Ok(trashed)
    }

    /// Query the user's Web Trash against authoritative facts with keysets.
    pub async fn query_web_trash(
        &self,
        options: QueryOptions,
    ) -> Result<Vec<MessageSummary>, ApplicationError> {
        validate_page_size(options.limit)?;
        let query = authorization::compose(
            &self.application.storage,
            self.user_identifier,
            &options,
            None,
        )
        .await?;
        Ok(self
            .application
            .storage
            .list_web_trash_messages(
                self.user_identifier,
                MessageListQuery {
                    filter: Some(authorization::storage_filter(&query)),
                    before: options.before,
                    limit: options.limit,
                },
            )
            .await?)
    }

    /// Permanently delete every visible message in the user's Web Trash; returns how many.
    pub async fn empty_web_trash(&self) -> Result<u64, ApplicationError> {
        let account = self.account().await?;
        authorization::require_action(account.global_role != GlobalRole::Viewer)?;
        let query = authorization::compose(
            &self.application.storage,
            self.user_identifier,
            &QueryOptions::default(),
            None,
        )
        .await?;
        let deleted = self
            .application
            .storage
            .empty_user_trash(self.user_identifier, &authorization::storage_filter(&query))
            .await?;
        if deleted > 0 {
            self.notify(ApplicationChange::Mail);
        }
        Ok(deleted)
    }
}
