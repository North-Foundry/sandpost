//! IMAP account creation, credentials, and settings.
use sandpost_core::{ImapAccount, ImapAccountMode, UserIdentifier};

/// A new IMAP account. Only the one-way password hash crosses the storage boundary.
///
/// Intentionally lacks Debug so credential material is never logged by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct NewImapAccount {
    pub owner_identifier: UserIdentifier,
    pub name: String,
    pub username: String,
    pub password_hash: String,
    pub mirror_views: bool,
    pub mode: ImapAccountMode,
    /// Current primary authorization used for the initial independent snapshot.
    pub initial_authorization: crate::FilterExpression,
    /// Unix seconds, UTC.
    pub created_at: i64,
}

/// An IMAP account and its stored password hash, read only for authentication.
///
/// Intentionally lacks Debug so the hash is never logged by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct ImapAccountCredential {
    pub account: ImapAccount,
    pub password_hash: String,
}

/// The mutable settings of an IMAP account, replaced together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapAccountSettings {
    pub name: String,
    pub enabled: bool,
    pub mirror_views: bool,
    pub mode: ImapAccountMode,
}
