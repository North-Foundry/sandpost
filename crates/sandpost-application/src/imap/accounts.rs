//! Management of a user's own IMAP accounts: credentials, mailbox configuration, and local
//! folders.
//!
//! Every operation acts only on accounts owned by the authenticated caller; another user's
//! account is reported as not found, whatever the caller's role. Generated passwords exist in
//! clear text only in the creation or regeneration response.
use super::catalog::{self, PlannedMailbox};
use crate::credentials::{
    GENERATED_USERNAME_ATTEMPTS, current_unix_seconds, generate_password, generated_username,
    hash_generated_password, validated_username,
};
use crate::{
    ApplicationChange, ApplicationError, AuthorizedApplication, authorization, validated_name,
};
use sandpost_core::{
    ImapAccount, ImapAccountIdentifier, ImapAccountMode, ImapFolderIdentifier, ImapFolderKind,
    ImapLinkedView, ImapLocalFolder, ImapMailboxSource, UserIdentifier, ViewIdentifier,
};
use sandpost_storage::{
    FilterExpression, ImapAccountSettings, ImapLocalFolderSettings, NewImapAccount,
    NewImapLocalFolder, StorageError,
};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;

/// Most IMAP accounts one user may own.
const MAXIMUM_ACCOUNTS_PER_USER: usize = 25;
/// Most local folders one account may have.
const MAXIMUM_FOLDERS_PER_ACCOUNT: usize = 100;
/// Most Views one account may link.
const MAXIMUM_LINKED_VIEWS: usize = 200;

/// How clients reach the IMAP service.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ImapSecurity {
    /// No TLS: passwords travel in clear text; only for loopback or trusted networks.
    None,
    /// Clients may upgrade with STARTTLS but may also log in without TLS.
    StartTlsOptional,
    /// Clients must upgrade with STARTTLS before logging in.
    StartTlsRequired,
    /// Every connection starts with TLS (IMAPS).
    Implicit,
}

/// Connection details of the runtime's IMAP service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImapServerDetails {
    /// Whether the IMAP service is configured and listening.
    pub enabled: bool,
    /// Host name or address clients should connect to.
    pub host: Option<String>,
    pub port: Option<u16>,
    pub security: ImapSecurity,
    /// The actual bound socket address.
    pub listen_address: Option<SocketAddr>,
}

impl ImapServerDetails {
    /// Details of an installation without an IMAP service.
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            host: None,
            port: None,
            security: ImapSecurity::None,
            listen_address: None,
        }
    }
}

/// One mailbox an account exposes, for display in management interfaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImapMailboxPreview {
    pub name: String,
    /// `inbox`, `trash`, `view`, or `folder`.
    pub source: &'static str,
    pub view_identifier: Option<ViewIdentifier>,
    pub folder_identifier: Option<ImapFolderIdentifier>,
}

/// An IMAP account with its configuration; never includes the password or its hash.
#[derive(Debug, Clone, Serialize)]
pub struct ImapAccountDetails {
    #[serde(flatten)]
    pub account: ImapAccount,
    pub linked_views: Vec<ImapLinkedView>,
    pub folders: Vec<ImapLocalFolder>,
    /// The mailboxes the account currently exposes, in LIST order.
    pub mailboxes: Vec<ImapMailboxPreview>,
}

/// Owner input for a new IMAP account; the password is always generated.
#[derive(Deserialize)]
pub struct NewImapAccountRequest {
    pub name: String,
    /// Optional explicit username; a unique one is derived from the name when omitted.
    #[serde(default)]
    pub username: Option<String>,
    /// Account organization mode; mirror when omitted.
    #[serde(default)]
    pub mode: Option<ImapAccountMode>,
    /// Deprecated compatibility field; used only if `mode` is omitted.
    #[serde(default = "mirror_by_default")]
    pub mirror_views: bool,
    /// Views to expose when not mirroring.
    #[serde(default)]
    pub linked_views: Vec<ImapLinkedView>,
}

/// New accounts mirror the owner's Views unless asked otherwise.
fn mirror_by_default() -> bool {
    true
}

/// Partial change to an IMAP account; omitted fields keep their value.
#[derive(Debug, Default, Deserialize)]
pub struct ImapAccountUpdate {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub mode: Option<ImapAccountMode>,
    /// Deprecated compatibility field; ignored when `mode` is supplied.
    #[serde(default)]
    pub mirror_views: Option<bool>,
    /// Replaces the linked View selection when present.
    #[serde(default)]
    pub linked_views: Option<Vec<ImapLinkedView>>,
}

/// Owner input for creating or replacing an IMAP-local folder.
#[derive(Debug, Clone, Deserialize)]
pub struct ImapFolderRequest {
    pub name: String,
    /// Canonical query-language source.
    pub filter: String,
    /// Whether the folder participates in INBOX when the account does not mirror Views.
    #[serde(default = "mirror_by_default")]
    pub include_in_inbox: bool,
}

/// Newly issued IMAP credentials: the only representation that carries the password.
///
/// Intentionally lacks Debug so the password is never formatted into logs.
#[derive(Serialize)]
pub struct IssuedImapCredentials {
    pub account: ImapAccountDetails,
    pub password: String,
    pub server: ImapServerDetails,
}

/// Describe the planned mailboxes for display.
fn previews(planned: &[PlannedMailbox]) -> Vec<ImapMailboxPreview> {
    planned
        .iter()
        .map(|mailbox| {
            let (source, view_identifier, folder_identifier) = match mailbox.source {
                ImapMailboxSource::Inbox => ("inbox", None, None),
                ImapMailboxSource::Trash => ("trash", None, None),
                ImapMailboxSource::View(view) => ("view", Some(view), None),
                ImapMailboxSource::LocalFolder(folder) => ("folder", None, Some(folder)),
            };
            ImapMailboxPreview {
                name: mailbox.name.clone(),
                source,
                view_identifier,
                folder_identifier,
            }
        })
        .collect()
}

impl AuthorizedApplication {
    /// Describe how clients connect to the IMAP service.
    pub async fn imap_server(&self) -> Result<ImapServerDetails, ApplicationError> {
        self.account().await?;
        Ok(self.imap_server_details())
    }

    /// The runtime's IMAP connection details, or a disabled description.
    fn imap_server_details(&self) -> ImapServerDetails {
        self.application
            .imap_server
            .read()
            .map(|details| details.clone())
            .unwrap_or(None)
            .unwrap_or_else(ImapServerDetails::disabled)
    }

    /// Load an account the caller owns; another user's account is not found.
    async fn owned_imap_account(
        &self,
        identifier: ImapAccountIdentifier,
    ) -> Result<ImapAccount, ApplicationError> {
        self.account().await?;
        self.application
            .storage
            .get_imap_account(identifier)
            .await?
            .filter(|account| account.owner_identifier == self.user_identifier)
            .ok_or(ApplicationError::NotFound)
    }

    /// Assemble an account's configuration and exposed mailboxes.
    async fn imap_account_details(
        &self,
        account: ImapAccount,
    ) -> Result<ImapAccountDetails, ApplicationError> {
        let storage = &self.application.storage;
        let linked_views = if account.mode == ImapAccountMode::Mirror {
            storage.list_imap_linked_views(account.identifier).await?
        } else {
            Vec::new()
        };
        let folders = storage.list_imap_local_folders(account.identifier).await?;
        let visible_views = if account.mode == ImapAccountMode::Mirror {
            storage.list_views(account.owner_identifier).await?
        } else {
            Vec::new()
        };
        let planned = catalog::plan(&account, &visible_views, &linked_views, &folders);
        Ok(ImapAccountDetails {
            mailboxes: previews(&planned),
            account,
            linked_views,
            folders,
        })
    }

    /// Validate a linked View selection: Views the caller can see, deduplicated, with valid
    /// aliases.
    async fn validated_linked_views(
        &self,
        views: Vec<ImapLinkedView>,
    ) -> Result<Vec<ImapLinkedView>, ApplicationError> {
        if views.len() > MAXIMUM_LINKED_VIEWS {
            return Err(ApplicationError::InvalidInput(format!(
                "link at most {MAXIMUM_LINKED_VIEWS} views"
            )));
        }
        let visible = self
            .application
            .storage
            .list_views(self.user_identifier)
            .await?;
        let mut validated: Vec<ImapLinkedView> = Vec::with_capacity(views.len());
        for view in views {
            if !visible
                .iter()
                .any(|candidate| candidate.identifier == view.view_identifier)
            {
                return Err(ApplicationError::InvalidInput(
                    "linked views must be smart folders you can see".into(),
                ));
            }
            if validated
                .iter()
                .any(|existing| existing.view_identifier == view.view_identifier)
            {
                continue;
            }
            let alias = view
                .alias
                .filter(|alias| !alias.trim().is_empty())
                .map(|alias| validated_name(&alias))
                .transpose()?;
            validated.push(ImapLinkedView {
                view_identifier: view.view_identifier,
                alias,
            });
        }
        Ok(validated)
    }

    /// List the caller's IMAP accounts with their configuration.
    pub async fn list_imap_accounts(&self) -> Result<Vec<ImapAccountDetails>, ApplicationError> {
        self.account().await?;
        let accounts = self
            .application
            .storage
            .list_imap_accounts(self.user_identifier)
            .await?;
        let mut details = Vec::with_capacity(accounts.len());
        for account in accounts {
            details.push(self.imap_account_details(account).await?);
        }
        Ok(details)
    }

    /// Load one of the caller's IMAP accounts.
    pub async fn get_imap_account(
        &self,
        identifier: ImapAccountIdentifier,
    ) -> Result<ImapAccountDetails, ApplicationError> {
        let account = self.owned_imap_account(identifier).await?;
        self.imap_account_details(account).await
    }

    /// Create an IMAP account with a generated password returned only in this response.
    pub async fn create_imap_account(
        &self,
        request: NewImapAccountRequest,
    ) -> Result<IssuedImapCredentials, ApplicationError> {
        self.require_mail_mutation().await?;
        self.account().await?;
        let storage = &self.application.storage;
        let name = validated_name(&request.name)?;
        let explicit_username = request
            .username
            .filter(|username| !username.trim().is_empty())
            .map(|username| validated_username(&username))
            .transpose()?;
        let mode = request.mode.unwrap_or({
            if request.mirror_views {
                ImapAccountMode::Mirror
            } else {
                ImapAccountMode::Independent
            }
        });
        let linked_views = if mode == ImapAccountMode::Mirror {
            self.validated_linked_views(request.linked_views).await?
        } else {
            Vec::new()
        };
        let initial_authorization: FilterExpression =
            catalog::owner_authorization(storage, self.user_identifier).await?;
        if storage
            .list_imap_accounts(self.user_identifier)
            .await?
            .len()
            >= MAXIMUM_ACCOUNTS_PER_USER
        {
            return Err(ApplicationError::InvalidInput(format!(
                "a user can own at most {MAXIMUM_ACCOUNTS_PER_USER} IMAP accounts"
            )));
        }
        let password = generate_password()?;
        let password_hash = hash_generated_password(&password).await?;
        let mut attempts = 0;
        let account = loop {
            attempts += 1;
            let username = match &explicit_username {
                Some(username) => username.clone(),
                None => generated_username(&name, "imap")?,
            };
            match storage
                .create_imap_account(&NewImapAccount {
                    owner_identifier: self.user_identifier,
                    name: name.clone(),
                    username,
                    password_hash: password_hash.clone(),
                    mirror_views: mode == ImapAccountMode::Mirror,
                    mode,
                    initial_authorization: initial_authorization.clone(),
                    created_at: current_unix_seconds(),
                })
                .await
            {
                Ok(account) => break account,
                Err(StorageError::DuplicateImapUsername)
                    if explicit_username.is_none() && attempts < GENERATED_USERNAME_ATTEMPTS => {}
                Err(StorageError::DuplicateImapUsername) => return Err(ApplicationError::Conflict),
                Err(error) => return Err(error.into()),
            }
        };
        if !linked_views.is_empty() {
            storage
                .set_imap_linked_views(account.identifier, &linked_views)
                .await?;
        }
        tracing::info!(account = %account.identifier, username = %account.username, "IMAP account created");
        Ok(IssuedImapCredentials {
            account: self.imap_account_details(account).await?,
            password,
            server: self.imap_server_details(),
        })
    }

    /// Change an account's name, status, or mailbox configuration.
    ///
    /// Disabling ends the account's running sessions; mailbox changes reach them at their next
    /// synchronization.
    pub async fn update_imap_account(
        &self,
        identifier: ImapAccountIdentifier,
        update: ImapAccountUpdate,
    ) -> Result<ImapAccountDetails, ApplicationError> {
        self.require_mail_mutation().await?;
        let account = self.owned_imap_account(identifier).await?;
        let storage = &self.application.storage;
        let mode = update
            .mode
            .or_else(|| {
                update.mirror_views.map(|mirror| {
                    if mirror {
                        ImapAccountMode::Mirror
                    } else {
                        ImapAccountMode::Independent
                    }
                })
            })
            .unwrap_or(account.mode);
        let settings = ImapAccountSettings {
            name: match update.name {
                Some(name) => validated_name(&name)?,
                None => account.name.clone(),
            },
            enabled: update.enabled.unwrap_or(account.enabled),
            mirror_views: mode == ImapAccountMode::Mirror,
            mode,
        };
        let linked_views = match update.linked_views {
            Some(views) if settings.mode == ImapAccountMode::Mirror => {
                Some(self.validated_linked_views(views).await?)
            }
            None => None,
            Some(_) => None,
        };
        if let Some(views) = &linked_views {
            storage.set_imap_linked_views(identifier, views).await?;
        }
        let updated = storage
            .update_imap_account(identifier, &settings)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        self.notify(ApplicationChange::ImapAccount(identifier));
        tracing::info!(account = %identifier, enabled = updated.enabled, mode = ?updated.mode, "IMAP account updated");
        self.imap_account_details(updated).await
    }

    /// Replace an account's password with a new generated one returned only in this response.
    ///
    /// The previous password stops working at once and the account's running sessions end.
    pub async fn regenerate_imap_account_password(
        &self,
        identifier: ImapAccountIdentifier,
    ) -> Result<IssuedImapCredentials, ApplicationError> {
        self.require_mail_mutation().await?;
        self.owned_imap_account(identifier).await?;
        let password = generate_password()?;
        let password_hash = hash_generated_password(&password).await?;
        let account = self
            .application
            .storage
            .replace_imap_account_password(identifier, &password_hash)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        self.notify(ApplicationChange::ImapAccount(identifier));
        tracing::info!(account = %identifier, "IMAP account password regenerated");
        Ok(IssuedImapCredentials {
            account: self.imap_account_details(account).await?,
            password,
            server: self.imap_server_details(),
        })
    }

    /// Delete an account and its IMAP state; captured mail is untouched.
    pub async fn delete_imap_account(
        &self,
        identifier: ImapAccountIdentifier,
    ) -> Result<(), ApplicationError> {
        self.require_mail_mutation().await?;
        self.owned_imap_account(identifier).await?;
        if !self
            .application
            .storage
            .delete_imap_account(identifier)
            .await?
        {
            return Err(ApplicationError::NotFound);
        }
        self.notify(ApplicationChange::ImapAccount(identifier));
        tracing::info!(account = %identifier, "IMAP account deleted");
        Ok(())
    }

    /// Let messages expunged in IMAP clients reappear in the account's dynamic mailboxes.
    pub async fn restore_imap_hidden_messages(
        &self,
        identifier: ImapAccountIdentifier,
    ) -> Result<u64, ApplicationError> {
        self.require_mail_mutation().await?;
        self.owned_imap_account(identifier).await?;
        let restored = self
            .application
            .storage
            .clear_imap_exclusions(identifier)
            .await?;
        self.notify(ApplicationChange::ImapAccount(identifier));
        Ok(restored)
    }

    /// Validate local folder input.
    fn validated_folder(request: ImapFolderRequest) -> Result<ImapFolderRequest, ApplicationError> {
        let name = validated_name(&request.name)?;
        authorization::expression(&request.filter)?;
        Ok(ImapFolderRequest {
            name,
            filter: request.filter.trim().to_owned(),
            include_in_inbox: request.include_in_inbox,
        })
    }

    /// Map a duplicate folder name to a conflict.
    fn folder_conflict(error: StorageError) -> ApplicationError {
        match error {
            StorageError::DuplicateImapFolderName => ApplicationError::Conflict,
            error => ApplicationError::Storage(error),
        }
    }

    /// Create an IMAP-local folder in one of the caller's accounts.
    pub async fn create_imap_folder(
        &self,
        account: ImapAccountIdentifier,
        request: ImapFolderRequest,
    ) -> Result<ImapLocalFolder, ApplicationError> {
        self.require_mail_mutation().await?;
        let account_details = self.owned_imap_account(account).await?;
        let request = Self::validated_folder(request)?;
        let storage = &self.application.storage;
        if storage.list_imap_local_folders(account).await?.len() >= MAXIMUM_FOLDERS_PER_ACCOUNT {
            return Err(ApplicationError::InvalidInput(format!(
                "an IMAP account can have at most {MAXIMUM_FOLDERS_PER_ACCOUNT} folders"
            )));
        }
        let folder = storage
            .create_imap_local_folder(&NewImapLocalFolder {
                expected_mode_generation: Some(account_details.mode_generation),
                account_identifier: account,
                name: request.name,
                filter: request.filter,
                include_in_inbox: request.include_in_inbox,
                created_at: current_unix_seconds(),
            })
            .await
            .map_err(Self::folder_conflict)?;
        self.notify(if account_details.mode == ImapAccountMode::Mirror {
            ApplicationChange::Visibility
        } else {
            ApplicationChange::ImapAccount(account)
        });
        Ok(folder)
    }

    /// Replace an IMAP-local folder's name, filter, and INBOX participation.
    pub async fn update_imap_folder(
        &self,
        account: ImapAccountIdentifier,
        folder: ImapFolderIdentifier,
        request: ImapFolderRequest,
    ) -> Result<ImapLocalFolder, ApplicationError> {
        self.require_mail_mutation().await?;
        let existing = self.owned_imap_folder(account, folder).await?;
        let account_details = self.owned_imap_account(account).await?;
        if existing.kind == ImapFolderKind::Private
            && account_details.mode == ImapAccountMode::Mirror
            || existing.kind == ImapFolderKind::Manual
                && account_details.mode == ImapAccountMode::Independent
        {
            return Err(ApplicationError::NotFound);
        }
        let request = Self::validated_folder(request)?;
        let updated = self
            .application
            .storage
            .update_imap_local_folder(
                folder,
                &ImapLocalFolderSettings {
                    expected_mode_generation: Some(account_details.mode_generation),
                    name: request.name,
                    filter: request.filter,
                    include_in_inbox: request.include_in_inbox,
                },
            )
            .await
            .map_err(Self::folder_conflict)?
            .ok_or(ApplicationError::NotFound)?;
        self.notify(if account_details.mode == ImapAccountMode::Mirror {
            ApplicationChange::Visibility
        } else {
            ApplicationChange::ImapAccount(account)
        });
        Ok(updated)
    }

    /// Delete an IMAP-local folder; Sandpost Views and mail are untouched.
    pub async fn delete_imap_folder(
        &self,
        account: ImapAccountIdentifier,
        folder: ImapFolderIdentifier,
    ) -> Result<(), ApplicationError> {
        let existing = self.owned_imap_folder(account, folder).await?;
        self.require_mail_mutation().await?;
        let account_details = self.owned_imap_account(account).await?;
        if existing.kind == ImapFolderKind::Private
            && account_details.mode == ImapAccountMode::Mirror
            || existing.kind == ImapFolderKind::Manual
                && account_details.mode == ImapAccountMode::Independent
        {
            return Err(ApplicationError::NotFound);
        }
        self.application
            .storage
            .delete_imap_local_folder(folder, Some(account_details.mode_generation))
            .await?;
        self.notify(if account_details.mode == ImapAccountMode::Mirror {
            ApplicationChange::Visibility
        } else {
            ApplicationChange::ImapAccount(account)
        });
        Ok(())
    }

    /// Require a folder to belong to one of the caller's accounts.
    async fn owned_imap_folder(
        &self,
        account: ImapAccountIdentifier,
        folder: ImapFolderIdentifier,
    ) -> Result<ImapLocalFolder, ApplicationError> {
        self.owned_imap_account(account).await?;
        self.application
            .storage
            .list_imap_local_folders(account)
            .await?
            .into_iter()
            .find(|stored| stored.identifier == folder)
            .ok_or(ApplicationError::NotFound)
    }

    /// The caller's identifier, for transports that scope responses to the caller.
    pub fn user_identifier(&self) -> UserIdentifier {
        self.user_identifier
    }
}
