//! Canonical users, global roles, mail access, scope memberships, and saved views.
use crate::{ScopeIdentifier, UserIdentifier, ViewIdentifier};
use serde::{Deserialize, Serialize};

/// Instance-wide authority for one canonical user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlobalRole {
    /// Full instance authority, including owner-only operations.
    Owner,
    /// Global Sandpost administrator for users, scopes, SMTP access, and shared resources.
    Admin,
    /// Reads and organizes mail with personal Smart Folders; no administrative authority.
    Member,
    /// Read-only: may read mail and keep personal Smart Folders, but cannot delete mail or
    /// manage shared resources.
    Viewer,
}

/// A canonical user account: identity, credentials, and instance-wide role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub identifier: UserIdentifier,
    pub name: String,
    pub email: String,
    pub password_hash: String,
    pub global_role: GlobalRole,
    /// Which captured mail the account may read, independent of its role.
    pub mail_access: MailAccess,
    pub personal_filter: Option<String>,
    /// Unix seconds, UTC, when the account was created.
    pub created_at: i64,
    /// Unix seconds, UTC, when the account was last changed.
    pub updated_at: i64,
}

/// Degree of captured mail an account can reach, independent of its role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailAccess {
    /// All captured mail, subject to the account's personal filter.
    All,
    /// Only mail matching the scopes assigned to this account (none when nothing is assigned).
    Scoped,
}

/// A role-less assignment of a user to a mail subset.
///
/// A scope membership only answers which mail subsets reach a scoped user. It never grants
/// administrative authority; that comes from the global role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeMembership {
    pub user_identifier: UserIdentifier,
    pub scope_identifier: ScopeIdentifier,
}

/// A named saved query optionally owned by one user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub identifier: ViewIdentifier,
    pub owner_identifier: Option<UserIdentifier>,
    pub name: String,
    pub filter: String,
}
