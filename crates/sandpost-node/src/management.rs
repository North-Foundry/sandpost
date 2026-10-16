//! Administrative adapters for SMTP access, scopes, assignments, views, accounts, and search.

use crate::NativeRuntime;
use crate::boundary::{
    application_error, deserialize, parse_scope_identifier, parse_smtp_access_identifier,
    parse_user_identifier, parse_view_identifier, serialize,
};
use napi::Result;
use napi_derive::napi;
use sandpost_application::{
    NewAccount, NewSmtpAccessRequest, ScopePreview, SmtpAccessUpdate, UpdateAccount,
};
use sandpost_core::{Scope, View};
use serde_json::Value;

#[napi]
impl NativeRuntime {
    /// Describe the global SMTP server and its listener to a global administrator.
    #[napi]
    pub async fn smtp_server_get(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .smtp_server()
                .await
                .map_err(application_error)?,
        )
    }

    /// List SMTP accesses without passwords or hashes.
    #[napi]
    pub async fn smtp_accesses_list(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .list_smtp_accesses()
                .await
                .map_err(application_error)?,
        )
    }

    /// Create an SMTP access; the generated password is returned only by this call.
    #[napi]
    pub async fn smtp_access_create(&self, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let request: NewSmtpAccessRequest = deserialize(input, "invalid SMTP access")?;
        serialize(
            self.application
                .authorized(&user)
                .create_smtp_access(request)
                .await
                .map_err(application_error)?,
        )
    }

    /// Regenerate an SMTP access password; the new one is returned only by this call.
    #[napi]
    pub async fn smtp_access_regenerate(&self, identifier: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .regenerate_smtp_access_password(parse_smtp_access_identifier(&identifier)?)
                .await
                .map_err(application_error)?,
        )
    }

    /// Change an SMTP access's status or authentication rules.
    #[napi]
    pub async fn smtp_access_update(&self, identifier: String, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let update: SmtpAccessUpdate = deserialize(input, "invalid SMTP access update")?;
        serialize(
            self.application
                .authorized(&user)
                .update_smtp_access(parse_smtp_access_identifier(&identifier)?, update)
                .await
                .map_err(application_error)?,
        )
    }

    /// Delete an SMTP access after validating its public identifier.
    #[napi]
    pub async fn smtp_access_delete(&self, identifier: String) -> Result<()> {
        let user = self.ensure_authenticated().await?;
        self.application
            .authorized(&user)
            .delete_smtp_access(parse_smtp_access_identifier(&identifier)?)
            .await
            .map_err(application_error)
    }

    /// Check query source and report its first compilation error without running it.
    #[napi]
    pub async fn query_validate(&self, query: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(self.application.authorized(&user).validate_query(&query))
    }

    /// Sample the newest mail an unsaved scope filter would select within its endpoint.
    #[napi]
    pub async fn scope_preview(&self, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let preview: ScopePreview = deserialize(input, "invalid scope preview")?;
        serialize(
            self.application
                .authorized(&user)
                .preview_scope(preview)
                .await
                .map_err(application_error)?,
        )
    }

    /// List configured scopes for the authenticated native user.
    #[napi]
    pub async fn scopes_list(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .list_scopes()
                .await
                .map_err(application_error)?,
        )
    }

    /// Save a scope from a serde-compatible native value.
    #[napi]
    pub async fn scope_save(&self, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let scope: Scope = deserialize(input, "invalid scope")?;
        serialize(
            self.application
                .authorized(&user)
                .save_scope(scope)
                .await
                .map_err(application_error)?,
        )
    }

    /// Delete a scope after validating its public identifier.
    #[napi]
    pub async fn scope_delete(&self, identifier: String) -> Result<()> {
        let user = self.ensure_authenticated().await?;
        self.application
            .authorized(&user)
            .delete_scope(parse_scope_identifier(&identifier)?)
            .await
            .map_err(application_error)
    }

    /// List account identifiers assigned to a scope.
    #[napi]
    pub async fn scope_assignments_list(&self, identifier: String) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .list_scope_assignments(parse_scope_identifier(&identifier)?)
                .await
                .map_err(application_error)?,
        )
    }

    /// Grant a scope assignment to an account.
    #[napi]
    pub async fn scope_assign(&self, account: String, scope: String) -> Result<()> {
        let user = self.ensure_authenticated().await?;
        self.application
            .authorized(&user)
            .assign_scope(
                parse_user_identifier(&account)?,
                parse_scope_identifier(&scope)?,
            )
            .await
            .map_err(application_error)
    }

    /// Revoke a scope assignment from an account.
    #[napi]
    pub async fn scope_assignment_remove(&self, account: String, scope: String) -> Result<()> {
        let user = self.ensure_authenticated().await?;
        self.application
            .authorized(&user)
            .remove_scope_assignment(
                parse_user_identifier(&account)?,
                parse_scope_identifier(&scope)?,
            )
            .await
            .map_err(application_error)
    }

    /// List saved views for the authenticated native user.
    #[napi]
    pub async fn views_list(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .list_views()
                .await
                .map_err(application_error)?,
        )
    }

    /// Save a view from a serde-compatible native value.
    #[napi]
    pub async fn view_save(&self, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let view: View = deserialize(input, "invalid view")?;
        serialize(
            self.application
                .authorized(&user)
                .save_view(view)
                .await
                .map_err(application_error)?,
        )
    }

    /// Delete a view after validating its public identifier.
    #[napi]
    pub async fn view_delete(&self, identifier: String) -> Result<()> {
        let user = self.ensure_authenticated().await?;
        self.application
            .authorized(&user)
            .delete_view(parse_view_identifier(&identifier)?)
            .await
            .map_err(application_error)
    }

    /// List user account details without password hashes.
    #[napi]
    pub async fn users_list(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .list_users()
                .await
                .map_err(application_error)?,
        )
    }

    /// Create an account from a serde-compatible native value.
    #[napi]
    pub async fn user_create(&self, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let account: NewAccount = deserialize(input, "invalid account")?;
        serialize(
            self.application
                .authorized(&user)
                .create_user(account)
                .await
                .map_err(application_error)?,
        )
    }

    /// Update an account from a serde-compatible native value.
    #[napi]
    pub async fn user_update(&self, identifier: String, input: Value) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        let update: UpdateAccount = deserialize(input, "invalid account update")?;
        serialize(
            self.application
                .authorized(&user)
                .update_user(parse_user_identifier(&identifier)?, update)
                .await
                .map_err(application_error)?,
        )
    }

    /// Delete an account after validating its public identifier.
    #[napi]
    pub async fn user_delete(&self, identifier: String) -> Result<()> {
        let user = self.ensure_authenticated().await?;
        self.application
            .authorized(&user)
            .delete_user(parse_user_identifier(&identifier)?)
            .await
            .map_err(application_error)
    }

    /// Read search-index synchronization status.
    #[napi]
    pub async fn search_status(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .search_status()
                .await
                .map_err(application_error)?,
        )
    }

    /// Rebuild the search index and return its new synchronization status.
    #[napi]
    pub async fn search_rebuild(&self) -> Result<Value> {
        let user = self.ensure_authenticated().await?;
        serialize(
            self.application
                .authorized(&user)
                .rebuild_search()
                .await
                .map_err(application_error)?,
        )
    }
}
