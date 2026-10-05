//! Canonical users, global and endpoint roles, memberships, and saved views.
use crate::{EndpointIdentifier, InboxIdentifier, ScopeIdentifier, UserIdentifier, ViewIdentifier};
use serde::{Deserialize, Serialize};

/// Instance-wide authority for one canonical user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GlobalRole {
    /// Full instance authority, including owner-only operations.
    Owner,
    /// Global SandPost administrator for users, endpoints, scopes, and shared resources.
    Admin,
    /// No instance-wide administrative authority; may still administer individual endpoints.
    Member,
}

/// A canonical user account: identity, credentials, and instance-wide role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    pub identifier: UserIdentifier,
    pub name: String,
    pub email: String,
    pub password_hash: String,
    pub global_role: GlobalRole,
    pub personal_filter: Option<String>,
    /// Unix seconds, UTC, when the account was created.
    pub created_at: i64,
    /// Unix seconds, UTC, when the account was last changed.
    pub updated_at: i64,
}

/// Administrative authority over one SMTP endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndpointRole {
    /// Endpoint-local administrator for configuration, scopes, assignments, and views.
    Admin,
    /// Normal interactive member of the endpoint.
    Member,
    /// Read-oriented access to the endpoint.
    Viewer,
}

/// Degree of endpoint mail a membership can reach, independent of its role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MailAccess {
    /// All endpoint mail, subject to higher-level and personal restrictions.
    All,
    /// Only mail matching scopes assigned to this user for the endpoint.
    Scoped,
}

/// A user's affiliation with one endpoint: role plus mail visibility mode.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EndpointMembership {
    pub user_identifier: UserIdentifier,
    pub endpoint_identifier: EndpointIdentifier,
    pub role: EndpointRole,
    pub mail_access: MailAccess,
}

/// A role-less assignment of a user to a mail subset within one endpoint.
///
/// A scope membership only answers which endpoint mail subsets reach the user. It never
/// grants administrative authority; that comes from global and endpoint roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeMembership {
    pub user_identifier: UserIdentifier,
    pub scope_identifier: ScopeIdentifier,
}

/// A compatibility recipient inbox backed by an owner-scoped View.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inbox {
    #[serde(rename = "id")]
    pub identifier: InboxIdentifier,
    #[serde(rename = "user_id")]
    pub user_identifier: UserIdentifier,
    pub name: String,
    pub filter: String,
}

/// A named saved query scoped to an endpoint and optionally owned by one user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct View {
    pub identifier: ViewIdentifier,
    pub endpoint_identifier: EndpointIdentifier,
    pub owner_identifier: Option<UserIdentifier>,
    pub name: String,
    pub filter: String,
}

/// A configured mail endpoint with a stable identifier and display name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmtpEndpoint {
    pub identifier: EndpointIdentifier,
    pub name: String,
}
