//! IMAP-local folder creation and settings.
use sandpost_core::ImapAccountIdentifier;

/// A new IMAP-local folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewImapLocalFolder {
    /// When supplied, require this account generation in the configuration transaction, even if disabled.
    pub expected_mode_generation: Option<u64>,
    pub account_identifier: ImapAccountIdentifier,
    pub name: String,
    /// Validated query-language source.
    pub filter: String,
    pub include_in_inbox: bool,
    /// Unix seconds, UTC.
    pub created_at: i64,
}

/// The mutable settings of an IMAP-local folder, replaced together.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImapLocalFolderSettings {
    /// When supplied, require this account generation in the configuration transaction, even if disabled.
    pub expected_mode_generation: Option<u64>,
    pub name: String,
    pub filter: String,
    pub include_in_inbox: bool,
}

/// Settings for a Web-managed folder's name and account exposure list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebFolderSettings {
    pub name: String,
    pub exposed_accounts: Vec<ImapAccountIdentifier>,
}
