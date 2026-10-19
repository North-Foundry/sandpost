//! Action-checked management of scopes, views, users, and the search index.
//!
//! SMTP server and SMTP access administration lives in the `smtp_access` module.
use crate::instance::validated_name;
use crate::{
    ApplicationChange, ApplicationError, AuthorizedApplication, MessageSummary, QueryOptions,
    SearchSynchronization, authorization, instance, validate_page_size,
};
use sandpost_core::{
    GlobalRole, MailAccess, Scope, ScopeIdentifier, User, UserIdentifier, View, ViewIdentifier,
};
use sandpost_query::Expression;
use sandpost_storage::{NewUser, StorageError, UpdateUser};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Public user identity, role, and mail access, excluding password material.
#[derive(Clone, Debug, Serialize)]
pub struct AccountDetails {
    pub identifier: UserIdentifier,
    pub name: String,
    pub email: String,
    #[serde(rename = "role")]
    pub global_role: GlobalRole,
    pub mail_access: MailAccess,
    pub personal_filter: Option<String>,
}

/// Administrator-provided identity, role, and mail access for a new user.
#[derive(Deserialize)]
pub struct NewAccount {
    pub name: String,
    pub email: String,
    pub password: String,
    #[serde(rename = "role")]
    pub global_role: GlobalRole,
    /// Mail the account may read; scoped (nothing until scopes are assigned) when omitted.
    #[serde(default = "default_mail_access")]
    pub mail_access: MailAccess,
    pub personal_filter: Option<String>,
}

/// Changes to a user identity, credentials, global role, or mail access.
#[derive(Deserialize)]
pub struct UpdateAccount {
    pub name: Option<String>,
    pub password: Option<String>,
    #[serde(rename = "role")]
    pub global_role: Option<GlobalRole>,
    pub mail_access: Option<MailAccess>,
    pub personal_filter: Option<String>,
}

/// Return the mail access of an account created without an explicit choice.
fn default_mail_access() -> MailAccess {
    MailAccess::Scoped
}

/// Result of checking query source without saving or running it.
#[derive(Clone, Debug, Serialize)]
pub struct QueryValidation {
    pub valid: bool,
    pub error: Option<QueryDiagnostic>,
}

/// One query compilation failure at a zero-based UTF-8 byte position.
#[derive(Clone, Debug, Serialize)]
pub struct QueryDiagnostic {
    pub message: String,
    pub position: usize,
}

/// An unsaved scope filter to sample against captured mail, inheriting its parent chain.
#[derive(Clone, Debug, Deserialize)]
pub struct ScopePreview {
    #[serde(default)]
    pub parent: Option<ScopeIdentifier>,
    pub filter: String,
    #[serde(default = "default_preview_limit")]
    pub limit: usize,
}

/// Return the number of newest matches sampled by a scope preview when none is requested.
fn default_preview_limit() -> usize {
    20
}

impl AuthorizedApplication {
    /// List scope definitions: every scope for global administrators, otherwise the caller's own.
    pub async fn list_scopes(&self) -> Result<Vec<Scope>, ApplicationError> {
        let account = self.account().await?;
        let storage = &self.application.storage;
        let scopes = storage.list_scopes().await?;
        if is_global_admin(account.global_role) {
            return Ok(scopes);
        }
        let assigned: HashSet<_> = storage
            .list_user_scopes(self.user_identifier)
            .await?
            .into_iter()
            .collect();
        Ok(scopes
            .into_iter()
            .filter(|scope| assigned.contains(&scope.identifier))
            .collect())
    }

    /// Validate and save a security query; no mail row or index document is changed.
    pub async fn save_scope(&self, mut scope: Scope) -> Result<Scope, ApplicationError> {
        self.require_global_admin().await?;
        scope.name = validated_name(&scope.name)?;
        let storage = &self.application.storage;
        if let Some(previous) = storage.get_scope(scope.identifier).await? {
            scope.policy_version = previous
                .policy_version
                .checked_add(1)
                .ok_or_else(|| ApplicationError::InvalidInput("scope version exhausted".into()))?;
        } else {
            scope.policy_version = 1;
        }
        authorization::validate_scope_tree(storage, &scope, self.application.maximum_scope_depth)
            .await?;
        storage.save_scope(&scope).await?;
        self.notify(ApplicationChange::Visibility);
        Ok(scope)
    }

    /// Remove a security definition and its assignments without touching messages.
    pub async fn delete_scope(&self, identifier: ScopeIdentifier) -> Result<(), ApplicationError> {
        self.require_global_admin().await?;
        let deleted = self
            .application
            .storage
            .delete_scope(identifier)
            .await
            .map_err(|error| match error {
                StorageError::ConstraintViolation(_) => {
                    ApplicationError::InUse("delete or move its child scopes first".into())
                }
                error => ApplicationError::Storage(error),
            })?;
        if deleted {
            self.notify(ApplicationChange::Visibility);
            Ok(())
        } else {
            Err(ApplicationError::NotFound)
        }
    }

    /// Sample the newest mail an unsaved scope filter would select.
    ///
    /// The filter is combined with every ancestor filter of `parent`, exactly as authorization
    /// inherits them, and with the caller's own visibility: an administrator never previews
    /// mail they could not read. Nothing is saved.
    pub async fn preview_scope(
        &self,
        preview: ScopePreview,
    ) -> Result<Vec<MessageSummary>, ApplicationError> {
        validate_page_size(preview.limit)?;
        self.require_global_admin().await?;
        let storage = &self.application.storage;
        let mut filters = vec![authorization::expression(&preview.filter)?];
        let mut visited = HashSet::new();
        let mut parent = preview.parent;
        while let Some(identifier) = parent {
            if !visited.insert(identifier) {
                return Err(ApplicationError::InvalidInput(
                    "scope ancestry contains a cycle".into(),
                ));
            }
            let ancestor = storage
                .get_scope(identifier)
                .await?
                .ok_or(ApplicationError::NotFound)?;
            filters.push(authorization::expression(&ancestor.filter)?);
            parent = ancestor.parent;
        }
        let mut query = authorization::compose(
            storage,
            self.user_identifier,
            &QueryOptions {
                limit: preview.limit,
                ..QueryOptions::default()
            },
            None,
        )
        .await?;
        filters.insert(0, query.filter);
        query.filter = Expression::And(filters);
        self.query_visible_messages(query).await
    }

    /// Check query source and report the first compilation error, without saving or running it.
    pub fn validate_query(&self, source: &str) -> QueryValidation {
        match sandpost_query::compile(source) {
            Ok(_) => QueryValidation {
                valid: true,
                error: None,
            },
            Err(error) => QueryValidation {
                valid: false,
                error: Some(QueryDiagnostic {
                    message: error.message,
                    position: error.position,
                }),
            },
        }
    }

    /// Inspect the users assigned to one scope.
    pub async fn list_scope_assignments(
        &self,
        scope: ScopeIdentifier,
    ) -> Result<Vec<UserIdentifier>, ApplicationError> {
        self.require_global_admin().await?;
        let storage = &self.application.storage;
        storage
            .get_scope(scope)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        Ok(storage.list_scope_members(scope).await?)
    }

    /// Assign a scope whose results join the user's mail-visibility union when access is scoped.
    pub async fn assign_scope(
        &self,
        assigned_user: UserIdentifier,
        scope: ScopeIdentifier,
    ) -> Result<(), ApplicationError> {
        self.require_global_admin().await?;
        let storage = &self.application.storage;
        storage
            .get_user(assigned_user)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        storage
            .assign_user_to_scope(assigned_user, scope)
            .await
            .map_err(|error| match error {
                StorageError::NotFound => ApplicationError::NotFound,
                error => ApplicationError::Storage(error),
            })?;
        self.notify(ApplicationChange::Visibility);
        Ok(())
    }

    /// Revoke a scope assignment; subsequent requests reread current grants.
    pub async fn remove_scope_assignment(
        &self,
        assigned_user: UserIdentifier,
        scope: ScopeIdentifier,
    ) -> Result<(), ApplicationError> {
        self.require_global_admin().await?;
        self.application
            .storage
            .remove_user_from_scope(assigned_user, scope)
            .await?;
        self.notify(ApplicationChange::Visibility);
        Ok(())
    }

    /// List the caller's personal views and every shared view.
    pub async fn list_views(&self) -> Result<Vec<View>, ApplicationError> {
        self.account().await?;
        Ok(self
            .application
            .storage
            .list_views(self.user_identifier)
            .await?)
    }

    /// Save a validated organizational query after enforcing personal/shared ownership.
    ///
    /// Anyone may keep personal views; only global administrators manage shared ones.
    pub async fn save_view(&self, mut view: View) -> Result<View, ApplicationError> {
        view.name = validated_name(&view.name)?;
        crate::validate_icon(&view.icon)?;
        authorization::expression(&view.filter)?;
        let storage = &self.application.storage;
        let shared_allowed = is_global_admin(self.account().await?.global_role);
        let personal = view.owner_identifier == Some(self.user_identifier);
        authorization::require_action(
            personal || (view.owner_identifier.is_none() && shared_allowed),
        )?;
        if let Some(existing) = storage.get_view(view.identifier).await? {
            let existing_personal = existing.owner_identifier == Some(self.user_identifier);
            authorization::require_action(
                existing_personal || (existing.owner_identifier.is_none() && shared_allowed),
            )?;
        }
        storage.save_view(&view).await?;
        self.notify(ApplicationChange::Visibility);
        Ok(view)
    }

    /// Delete an owned or administratively managed shared query without touching mail.
    pub async fn delete_view(&self, identifier: ViewIdentifier) -> Result<(), ApplicationError> {
        let storage = &self.application.storage;
        let view = storage
            .get_view(identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        let personal = view.owner_identifier == Some(self.user_identifier);
        let shared_allowed = is_global_admin(self.account().await?.global_role);
        authorization::require_action(
            personal || (view.owner_identifier.is_none() && shared_allowed),
        )?;
        storage.delete_view(identifier).await?;
        self.notify(ApplicationChange::Visibility);
        Ok(())
    }

    /// Return user identities without leaking password hashes.
    pub async fn list_users(&self) -> Result<Vec<AccountDetails>, ApplicationError> {
        self.require_global_admin().await?;
        Ok(self
            .application
            .storage
            .list_users()
            .await?
            .into_iter()
            .map(account_details)
            .collect())
    }

    /// Create a user; a non-owner administrator cannot mint owners.
    pub async fn create_user(
        &self,
        account: NewAccount,
    ) -> Result<AccountDetails, ApplicationError> {
        let caller = self.account().await?;
        authorization::require_action(is_global_admin(caller.global_role))?;
        require_global_role_grant(caller.global_role, account.global_role)?;
        let user = self
            .application
            .storage
            .create_user(&NewUser {
                name: validated_name(&account.name)?,
                email: instance::normalize_email(&account.email)?,
                password_hash: instance::hash_password(&account.password)?,
                global_role: account.global_role,
                mail_access: account.mail_access,
                personal_filter: account.personal_filter,
            })
            .await
            .map_err(map_conflict)?;
        Ok(account_details(user))
    }

    /// Update a user, keeping role changes within the caller's own authority.
    pub async fn update_user(
        &self,
        identifier: UserIdentifier,
        changes: UpdateAccount,
    ) -> Result<AccountDetails, ApplicationError> {
        let caller = self.account().await?;
        authorization::require_action(is_global_admin(caller.global_role))?;
        let mut user = self
            .application
            .storage
            .get_user(identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        require_global_role_grant(caller.global_role, user.global_role)?;
        if let Some(name) = changes.name {
            user.name = validated_name(&name)?;
        }
        if let Some(password) = changes.password {
            user.password_hash = instance::hash_password(&password)?;
        }
        if let Some(role) = changes.global_role {
            require_global_role_grant(caller.global_role, role)?;
            user.global_role = role;
        }
        if let Some(mail_access) = changes.mail_access {
            user.mail_access = mail_access;
        }
        if let Some(filter) = changes.personal_filter {
            user.personal_filter = Some(filter);
        }
        let updated = self
            .application
            .storage
            .update_user(
                identifier,
                &UpdateUser {
                    name: user.name,
                    email: user.email,
                    password_hash: user.password_hash,
                    global_role: user.global_role,
                    mail_access: user.mail_access,
                    personal_filter: user.personal_filter,
                },
            )
            .await
            .map_err(map_conflict)?
            .ok_or(ApplicationError::NotFound)?;
        self.application
            .instance
            .revoke_user_sessions(identifier)
            .await?;
        self.notify(ApplicationChange::Visibility);
        Ok(account_details(updated))
    }

    /// Delete a user, preserving at least one global owner.
    pub async fn delete_user(&self, identifier: UserIdentifier) -> Result<(), ApplicationError> {
        let caller = self.account().await?;
        authorization::require_action(is_global_admin(caller.global_role))?;
        let user = self
            .application
            .storage
            .get_user(identifier)
            .await?
            .ok_or(ApplicationError::NotFound)?;
        require_global_role_grant(caller.global_role, user.global_role)?;
        self.application.storage.delete_user(identifier).await?;
        self.application
            .instance
            .revoke_user_sessions(identifier)
            .await?;
        self.notify(ApplicationChange::Visibility);
        Ok(())
    }

    /// Expose index lag only to instance owners.
    pub async fn search_status(&self) -> Result<SearchSynchronization, ApplicationError> {
        authorization::require_action(self.account().await?.global_role == GlobalRole::Owner)?;
        self.application.search.status().await
    }

    /// Rebuild disposable search state while preserving authoritative SQL and live ingestion.
    pub async fn rebuild_search(&self) -> Result<SearchSynchronization, ApplicationError> {
        authorization::require_action(self.account().await?.global_role == GlobalRole::Owner)?;
        self.application.search.rebuild().await
    }

    /// Load the caller's authoritative account, refusing stale or unknown identities.
    pub(crate) async fn account(&self) -> Result<User, ApplicationError> {
        authorization::user(&self.application.storage, self.user_identifier).await
    }

    /// Refuse callers without instance-wide administrative authority.
    pub(crate) async fn require_global_admin(&self) -> Result<(), ApplicationError> {
        authorization::require_action(is_global_admin(self.account().await?.global_role))
    }
}

/// Report whether a global role carries instance-wide administrative authority.
pub(crate) fn is_global_admin(role: GlobalRole) -> bool {
    matches!(role, GlobalRole::Owner | GlobalRole::Admin)
}

/// Strip credential material from a user response.
fn account_details(user: User) -> AccountDetails {
    AccountDetails {
        identifier: user.identifier,
        name: user.name,
        email: user.email,
        global_role: user.global_role,
        mail_access: user.mail_access,
        personal_filter: user.personal_filter,
    }
}

/// Prevent account administration from granting a role above the caller's own authority.
fn require_global_role_grant(
    caller: GlobalRole,
    granted: GlobalRole,
) -> Result<(), ApplicationError> {
    if caller == GlobalRole::Owner {
        return Ok(());
    }
    authorization::require_action(granted != GlobalRole::Owner)
}

/// Translate a backend uniqueness failure into a transport-level conflict.
fn map_conflict(error: sandpost_storage::StorageError) -> ApplicationError {
    match error {
        sandpost_storage::StorageError::DuplicateUserEmail => ApplicationError::Conflict,
        error => ApplicationError::Storage(error),
    }
}
