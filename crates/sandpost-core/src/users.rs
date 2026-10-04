//! User identities, scope memberships, roles, and personal inbox definitions.
use crate::{InboxIdentifier, ScopeIdentifier, UserIdentifier};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    #[serde(rename = "id")]
    pub identifier: UserIdentifier,
    pub name: String,
    pub personal_filter: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Owner,
    #[serde(rename = "admin")]
    Administrator,
    Member,
    Viewer,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Membership {
    #[serde(rename = "user_id")]
    pub user_identifier: UserIdentifier,
    #[serde(rename = "scope_id")]
    pub scope_identifier: ScopeIdentifier,
    pub role: Role,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Inbox {
    #[serde(rename = "id")]
    pub identifier: InboxIdentifier,
    #[serde(rename = "user_id")]
    pub user_identifier: UserIdentifier,
    pub name: String,
    pub filter: String,
}
