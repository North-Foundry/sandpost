//! IMAP-local folder creation and settings.
use sandpost_core::ImapAccountIdentifier;

/// A new IMAP-local folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewImapLocalFolder {
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
    pub name: String,
    pub filter: String,
    pub include_in_inbox: bool,
}
